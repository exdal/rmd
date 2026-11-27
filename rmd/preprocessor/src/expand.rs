use std::{fmt::Write, ops::Range};

use base::location::Location;
use lexer::token::{ContextMacro, Token};

use crate::{
    Preprocessor, RawToken,
    define::{BodyPart, Builtin, Define, NameId, Slot},
    diagnostic::Code,
    hide_set::HideSet,
    pastable,
};

/// `M(a, b)`
#[derive(Default)]
pub(super) struct Arguments<'a> {
    tokens: Vec<RawToken<'a>>,
    bounds: Vec<Range<usize>>,
}

impl<'a> Arguments<'a> {
    fn clear(&mut self) {
        self.tokens.clear();
        self.bounds.clear();
    }

    fn finish_argument(&mut self) {
        let start = self.bounds.last().map_or(0, |bound| bound.end);
        self.bounds.push(start..self.tokens.len());
    }

    fn get(&self, index: usize) -> Option<&[RawToken<'a>]> {
        self.bounds.get(index).map(|bound| &self.tokens[bound.clone()])
    }

    fn iter(&self) -> impl Iterator<Item = &[RawToken<'a>]> {
        self.bounds.iter().map(|bound| &self.tokens[bound.clone()])
    }
}

impl<'a> Preprocessor<'a> {
    /// `#define M(x) x`
    pub(super) fn try_macro(&mut self, name: &str, invocation: RawToken<'a>) -> bool {
        let Some((id, define)) = self.defines.lookup(name) else {
            return false;
        };

        if self.hide_sets.contains(invocation.hidden, id) {
            return false;
        }

        let mut arguments = std::mem::take(&mut self.arguments);
        arguments.clear();

        let is_called = define.is_function_like();
        if is_called && !self.collect_arguments(&mut arguments) {
            self.arguments = arguments;
            return false;
        }

        self.expand(&define, id, is_called.then_some(&arguments), invocation);
        self.arguments = arguments;

        true
    }

    /// `M(a, b)`
    fn collect_arguments(&mut self, arguments: &mut Arguments<'a>) -> bool {
        let Some(open) = self.next_significant() else {
            return false;
        };

        if open.token != Token::ParenLeft {
            self.push_back(open);
            return false;
        }

        let mut depth = 1usize;

        loop {
            let Some(raw) = self.next_raw() else {
                self.diagnose(Code::BadDirective, open.location, "missing ')' in macro call");
                break;
            };
            let token = raw.token;

            match token {
                Token::Comma if depth == 1 => arguments.finish_argument(),
                Token::ParenRight => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }

                    arguments.tokens.push(raw);
                },
                Token::ParenLeft => {
                    depth += 1;
                    arguments.tokens.push(raw);
                },
                Token::Newline | Token::SuppressedNewline => {},
                token if token.is_comment() => {},
                _ => arguments.tokens.push(raw),
            }
        }

        arguments.finish_argument();

        true
    }

    /// Builds the expansion forwards on top of `pending`, then flips it so it pops in order.
    fn expand(&mut self, define: &Define<'a>, id: NameId, arguments: Option<&Arguments<'a>>, invocation: RawToken<'a>) {
        let start = self.pending.len();
        self.expansions.push(start);

        let fixed_hidden = self.hide_sets.with(invocation.hidden, id);
        if let Some(builtin) = define.builtin {
            self.pending.push(RawToken {
                token: self.expand_builtin(builtin, invocation.location),
                hidden: fixed_hidden,
                ..invocation
            });
            return;
        }

        let location = invocation.location;
        // `foo##x`, `foo x`
        let mut separated = true;

        for part in &define.body {
            match part {
                BodyPart::Space => separated = true,
                BodyPart::Stringify(name) => {
                    let text = match arguments.zip(define.lookup(name)) {
                        Some((arguments, slot)) => self.render_slot(slot, arguments).unwrap_or(name),
                        None => name,
                    };

                    self.pending.push(RawToken::expanded(
                        Token::RawStringLiteral(text),
                        location,
                        separated,
                        fixed_hidden,
                    ));
                    separated = false;
                },
                BodyPart::Paste(name) => {
                    if !self.splice_slot(start, location, arguments, define, name, false) {
                        let token = Token::from_identifier(name);
                        self.splice(start, location, token, false, fixed_hidden);
                    }

                    separated = false;
                },
                BodyPart::Token(token) => {
                    let is_slot = token
                        .word()
                        .is_some_and(|word| self.splice_slot(start, location, arguments, define, word, separated));
                    if !is_slot {
                        self.pending
                            .push(RawToken::expanded(*token, location, separated, fixed_hidden));
                    }

                    separated = false;
                },
            }
        }

        if let Some(first) = self.pending.get_mut(start) {
            first.spacing = invocation.spacing;
        }

        self.pending[start..].reverse();
    }

    fn expand_builtin(&self, builtin: Builtin, location: Location) -> Token<'a> {
        match builtin {
            Builtin::Line => Token::IntegerLiteral(self.arena.alloc(location.begin.line.to_string())),
            Builtin::File => {
                let path = self
                    .sources
                    .path(location.file)
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();

                Token::RawStringLiteral(self.arena.alloc(path))
            },
            Builtin::Type => Token::ContextMacro(ContextMacro::Type),
            Builtin::ImpliedType => Token::ContextMacro(ContextMacro::ImpliedType),
            Builtin::Proc => Token::ContextMacro(ContextMacro::Proc),
        }
    }

    /// `#define M(a, rest...)`
    fn splice_slot(
        &mut self, start: usize, location: Location, arguments: Option<&Arguments<'a>>, define: &Define<'a>,
        name: &str, separated: bool,
    ) -> bool {
        let Some((arguments, slot)) = arguments.zip(define.lookup(name)) else {
            return false;
        };

        let mut is_first = true;
        let mut splice = |this: &mut Self, token: Token<'a>, token_separated: bool, hidden: HideSet| {
            let separated = if is_first { separated } else { token_separated };
            is_first = false;
            this.splice(start, location, token, separated, hidden);
        };

        match slot {
            Slot::Positional(index) => {
                let Some(argument) = arguments.get(index) else {
                    return false;
                };

                for_argument(argument, false, |token, separated, hidden| {
                    splice(self, token, separated, hidden);
                });
            },
            Slot::Rest(index) => {
                for (offset, argument) in arguments.iter().skip(index).enumerate() {
                    if offset > 0 {
                        splice(self, Token::Comma, true, HideSet::default());
                    }

                    for_argument(argument, offset > 0, |token, separated, hidden| {
                        splice(self, token, separated, hidden);
                    });
                }
            },
        }

        true
    }

    /// `foo##x`
    fn splice(&mut self, start: usize, location: Location, token: Token<'a>, separated: bool, hidden: HideSet) {
        let last = self.pending.len().checked_sub(1).filter(|last| *last >= start);
        let pasted = (!separated)
            .then(|| pastable(&token).zip(last.and_then(|last| self.pending[last].token.word())))
            .flatten();

        match pasted.zip(last) {
            Some(((right, left), last)) => {
                let joined = self.arena.alloc_concat(&[left, right]);
                let merged = self.hide_sets.union(self.pending[last].hidden, hidden);
                let last = &mut self.pending[last];
                last.token = Token::from_identifier(joined);
                last.hidden = merged;
            },
            None => self
                .pending
                .push(RawToken::expanded(token, location, separated, hidden)),
        }
    }

    fn render_slot(&mut self, slot: Slot, arguments: &Arguments<'a>) -> Option<&'a str> {
        let mut text = std::mem::take(&mut self.stringified);
        text.clear();

        match slot {
            Slot::Positional(index) => {
                let Some(argument) = arguments.get(index) else {
                    self.stringified = text;
                    return None;
                };

                for raw in argument {
                    let _ = write!(text, "{}", raw.token);
                }
            },
            Slot::Rest(index) => {
                for (offset, argument) in arguments.iter().skip(index).enumerate() {
                    if offset > 0 {
                        text.push(',');
                    }

                    for raw in argument {
                        let _ = write!(text, "{}", raw.token);
                    }
                }
            },
        }

        let rendered = self.arena.alloc_str(&text);
        self.stringified = text;

        Some(rendered)
    }
}

fn for_argument<'a>(argument: &[RawToken<'a>], mut separated: bool, mut each: impl FnMut(Token<'a>, bool, HideSet)) {
    let mut previous: Option<Location> = None;

    for raw in argument {
        if let Some(previous) = previous {
            separated = raw.separated_from(previous);
        }

        each(raw.token, separated, raw.hidden);
        previous = Some(raw.location);
    }
}
