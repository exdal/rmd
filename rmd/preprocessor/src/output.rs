use base::location::Location;
use lexer::token::Token;

use crate::{IncludeState, Preprocessor, RawToken, Spanned, adjacent, diagnostic::Code, pastable};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum IndentOwner {
    Layout,
    /// /proc/foo(){ ... }
    Brace,
}

impl<'a> Preprocessor<'a> {
    pub(super) fn drain(&mut self) {
        while !self.include_stack.is_empty() {
            if self.aborted {
                self.include_stack.clear();

                break;
            }

            self.pump();
        }
    }

    pub(super) fn end_stream(&mut self) {
        self.flush_layout();

        if self.line_has_content {
            self.line_has_content = false;
            let location = self.output.last().map(|(_, location)| *location).unwrap_or_default();
            self.output.push((Token::Newline, location));
        }

        self.can_use_directive = true;
        self.line_indent = None;
    }

    fn pump(&mut self) {
        let from_file = self.expansions.is_empty();
        let Some(mut raw) = self.next_raw() else {
            return;
        };
        let token = raw.token;
        let location = raw.location;

        if from_file {
            let previous = self.last_file_line.replace((location.file, location.end.line));
            if previous != Some((location.file, location.begin.line)) {
                self.can_use_directive = true;
            }
        }

        match token {
            Token::SuppressedNewline => self.can_use_directive = true,
            Token::Newline => {
                self.can_use_directive = true;

                if self.line_has_content {
                    self.line_has_content = false;
                    self.output.push((token, location));
                }
            },
            Token::Indent | Token::Dedent => self.line_layout.push((token, location)),
            token if token.is_comment() => {},
            token if token.is_directive() => self.handle_directive(token, location, from_file),
            token => {
                let (token, location) = if from_file {
                    self.glue_identifier(token, location)
                } else {
                    (token, location)
                };
                raw.token = token;
                raw.location = location;

                if let Some(name) = token.word()
                    && self.try_macro(name, raw)
                {
                    return;
                }

                self.emit(token, location);
            },
        }
    }

    fn emit(&mut self, token: Token<'a>, location: Location) {
        self.flush_layout();

        self.line_has_content = true;
        self.can_use_directive = token == Token::Semicolon;
        self.output.push((token, location));
    }

    fn flush_layout(&mut self) {
        let mut layout = std::mem::take(&mut self.carried_layout);
        layout.append(&mut self.line_layout);
        for (token, location) in layout.drain(..) {
            self.push_layout(token, location);
        }

        self.carried_layout = layout;
    }

    fn push_layout(&mut self, token: Token<'a>, location: Location) {
        match token {
            Token::Indent => {
                let after_brace = self.output.iter().rev().find(|(token, _)| *token != Token::Newline);
                let owner = match after_brace {
                    Some((Token::BraceLeft, _)) => IndentOwner::Brace,
                    _ => IndentOwner::Layout,
                };

                self.indent_owners.push(owner);
                if owner == IndentOwner::Brace {
                    return;
                }
            },
            Token::Dedent if self.indent_owners.pop() == Some(IndentOwner::Brace) => return,
            _ => {},
        }

        self.output.push((token, location));
    }

    pub(super) fn next_raw(&mut self) -> Option<RawToken<'a>> {
        loop {
            while let Some(&start) = self.expansions.last() {
                if self.pending.len() > start {
                    return self.pending.pop();
                }

                self.expansions.pop();
            }

            let depth = self.include_stack.len();
            let state = self.include_stack.last()?;
            if state.peeked.is_none() && state.lexer.at_line_start() {
                let indent = state.lexer.indent_state();
                self.line_indent = Some((depth, indent));
                self.carried_layout.append(&mut self.line_layout);
            }

            let state = self.include_stack.last_mut()?;
            let spanned = state.peeked.take().unwrap_or_else(|| state.lexer.advance());
            if spanned.0 != Token::Eof {
                return Some(RawToken::source(spanned));
            }

            let errors = state.lexer.take_errors();
            let finished = self.include_stack.pop()?;

            self.hand_back_indent(&finished);
            self.flush_layout();

            for error in errors {
                self.diagnose(Code::BadToken, error.location, error.kind.to_string());
            }
        }
    }

    /// Include indentation returns to its parent
    fn hand_back_indent(&mut self, finished: &IncludeState<'a>) {
        if let Some(parent) = self.include_stack.last_mut() {
            parent
                .lexer
                .restore_indent(finished.lexer.indent_state().without_errors());
            return;
        }

        let position = finished.lexer.position();
        let location = Location::in_file(finished.lexer.file(), position, position);
        for _ in 0..finished.lexer.open_indents() {
            self.line_layout.push((Token::Dedent, location));
        }
    }

    pub(super) fn next_significant(&mut self) -> Option<RawToken<'a>> {
        loop {
            let raw = self.next_raw()?;
            if !raw.token.is_comment() {
                return Some(raw);
            }
        }
    }

    pub(super) fn push_back(&mut self, raw: RawToken<'a>) {
        self.expansions.push(self.pending.len());
        self.pending.push(raw);
    }

    pub(super) fn check(&mut self, expected: Token<'a>) -> bool {
        let Some(spanned) = self.next_significant() else {
            return false;
        };

        if spanned.token == expected {
            return true;
        }

        self.push_back(spanned);

        false
    }

    pub(super) fn skip_rest_of_line(&mut self) {
        while let Some(raw) = self.next_raw() {
            if matches!(raw.token, Token::Newline | Token::SuppressedNewline) {
                break;
            }
        }
    }

    /// `ES\KE`
    fn glue_identifier(&mut self, token: Token<'a>, location: Location) -> Spanned<'a> {
        if token.word().is_none() {
            return (token, location);
        }

        let mut text: Option<String> = None;
        let mut end = location;

        while let Some((next, next_location)) = self.peek_file() {
            let Some(spelling) = pastable(&next).filter(|_| adjacent(end, next_location)) else {
                break;
            };

            self.take_peeked();
            text.get_or_insert_with(|| token.word().unwrap_or_default().to_string())
                .push_str(spelling);
            end = next_location;
        }

        match text {
            Some(text) => (
                Token::from_identifier(self.arena.alloc_str(&text)),
                Location::in_file(location.file, location.begin, end.end),
            ),
            None => (token, location),
        }
    }

    fn peek_file(&mut self) -> Option<Spanned<'a>> {
        let state = self.include_stack.last_mut()?;
        if state.peeked.is_none() {
            state.peeked = Some(state.lexer.advance());
        }

        state.peeked
    }

    fn take_peeked(&mut self) {
        if let Some(state) = self.include_stack.last_mut() {
            state.peeked = None;
        }
    }
}
