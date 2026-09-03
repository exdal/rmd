pub mod error;
pub mod token;

use core::location::{FileId, Location, Position};

use crate::{
    error::{LexError, LexErrorKind},
    token::Token,
};

const NUMBER_MARKERS: [&str; 4] = ["#INF", "#IND", "#QNAN", "#SNAN"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BraceKind {
    Square,
    Interpolation { multiline: bool },
}

#[derive(Clone, Debug)]
pub struct IndentState {
    stack: Vec<usize>,
    pending_dedents: usize,
}

pub struct Lexer<'a> {
    buffer_view: &'a str,
    file: FileId,
    offset: usize,
    line: usize,
    line_offset: usize,

    indent_stack: Vec<usize>,
    pending_dedents: usize,
    at_line_start: bool,
    /// An included file shares one logical indent stack with its includer, so only the outermost
    /// stream closes its blocks at end of file - see `Preprocessor::open`.
    close_indents_at_eof: bool,

    paren_depth: usize,
    brace_stack: Vec<BraceKind>,
    layout_disabled: bool,

    errors: Vec<LexError>,
}

impl<'a> Lexer<'a> {
    pub fn new(buffer_view: &'a str) -> Self { Self::with_file(buffer_view, FileId::default()) }

    pub fn with_file(buffer_view: &'a str, file: FileId) -> Self {
        Self {
            buffer_view,
            file,
            offset: 0,
            line: 0,
            line_offset: 0,
            indent_stack: vec![0],
            pending_dedents: 0,
            at_line_start: true,
            close_indents_at_eof: true,
            paren_depth: 0,
            brace_stack: Vec::new(),
            layout_disabled: false,
            errors: Vec::new(),
        }
    }

    pub fn errors(&self) -> &[LexError] { &self.errors }

    pub fn take_errors(&mut self) -> Vec<LexError> { std::mem::take(&mut self.errors) }

    pub fn file(&self) -> FileId { self.file }

    pub fn at_line_start(&self) -> bool { self.at_line_start }

    pub fn indent_state(&self) -> IndentState {
        IndentState {
            stack: self.indent_stack.clone(),
            pending_dedents: self.pending_dedents,
        }
    }

    pub fn restore_indent(&mut self, state: IndentState) {
        self.indent_stack = state.stack;
        self.pending_dedents = state.pending_dedents;
    }

    /// Leave open blocks open at end of file, for a stream that continues in another buffer.
    pub fn keep_indents_at_eof(&mut self) { self.close_indents_at_eof = false; }

    /// How many `Dedent`s it would take to close everything still open.
    pub fn open_indents(&self) -> usize { self.pending_dedents + self.indent_stack.len().saturating_sub(1) }

    pub fn position(&self) -> Position { Position::new(self.line + 1, self.offset - self.line_offset + 1) }

    fn byte_at(&self, offset: usize) -> u8 { self.buffer_view.as_bytes().get(offset).copied().unwrap_or(0) }

    fn peek(&self, look_ahead: usize) -> u8 { self.byte_at(self.offset + look_ahead) }

    fn is_eof(&self) -> bool { self.offset >= self.buffer_view.len() }

    fn consume(&mut self) {
        debug_assert_ne!(self.peek(0), b'\n', "Unexpected new line");
        self.offset += 1;
    }

    fn consume_any(&mut self) {
        if self.peek(0) == b'\n' {
            self.line += 1;
            self.line_offset = self.offset + 1;
        }

        self.offset += 1;
    }

    fn error(&mut self, kind: LexErrorKind) {
        let position = self.position();
        self.errors.push(LexError {
            location: Location::in_file(self.file, position, position),
            kind,
        });
    }

    fn slice(&self, start: usize) -> &'a str { &self.buffer_view[start..self.offset] }

    fn located(&self, token: Token<'a>, begin: Position) -> (Token<'a>, Location) {
        (token, Location::in_file(self.file, begin, self.position()))
    }

    pub fn layout_suppressed(&self) -> bool { self.suppress_layout() }

    /// For a file spliced in by `#include` from inside an expression: its lines are continuations of
    /// the one holding the directive, so none of its layout means anything.
    pub fn disable_layout(&mut self) { self.layout_disabled = true; }

    fn suppress_layout(&self) -> bool { self.layout_disabled || self.paren_depth > 0 || !self.brace_stack.is_empty() }

    fn skip_horizontal_whitespace(&mut self) {
        while matches!(self.peek(0), b' ' | b'\t') {
            self.consume();
        }
    }

    fn skip_to_end_of_line(&mut self) {
        while !self.is_eof() && self.peek(0) != b'\n' {
            self.consume();
        }
    }

    /// A `\` at end of line continues a string onto the next one, and the empty lines writers leave
    /// between the halves belong to that continuation rather than ending the string.
    fn skip_blank_lines(&mut self) {
        loop {
            let (_, next) = self.measure_indent();
            if next >= self.buffer_view.len() {
                return;
            }

            match self.byte_at(next) {
                b'\n' => {
                    self.offset = next;
                    self.consume_any();
                },
                b'\r' => self.offset = next + 1,
                _ => return,
            }
        }
    }

    fn measure_indent(&self) -> (usize, usize) {
        let mut offset = self.offset;
        let mut width = 0;

        while matches!(self.byte_at(offset), b' ' | b'\t') {
            width += 1;
            offset += 1;
        }

        (width, offset)
    }

    fn read_indentation(&mut self) -> Option<Token<'a>> {
        loop {
            let (width, next) = self.measure_indent();
            if next >= self.buffer_view.len() {
                self.offset = next;
                return None;
            }

            match self.byte_at(next) {
                b'\n' => {
                    self.offset = next;
                    self.consume_any();
                },
                b'\r' => {
                    self.offset = next + 1;
                },
                b'/' if self.byte_at(next + 1) == b'/' => {
                    self.offset = next;
                    self.skip_to_end_of_line();
                },
                // Comments come out before indentation is measured, so a `/* */` opening a line
                // neither opens a block nor closes the one around it, however many lines it spans.
                b'/' if self.byte_at(next + 1) == b'*' => {
                    self.offset = next;
                    self.read_block_comment();
                    self.skip_horizontal_whitespace();
                    if !self.is_eof() && !matches!(self.peek(0), b'\n' | b'\r') {
                        return self.apply_indent(width);
                    }
                },
                _ => {
                    self.offset = next;
                    return self.apply_indent(width);
                },
            }
        }
    }

    fn apply_indent(&mut self, width: usize) -> Option<Token<'a>> {
        let current = self.indent_stack.last().copied().unwrap_or(0);

        if width > current {
            self.indent_stack.push(width);
            return Some(Token::Indent);
        }

        if width < current {
            let mut dedents = 0usize;
            while self.indent_stack.last().is_some_and(|level| *level > width) {
                self.indent_stack.pop();
                dedents += 1;
            }

            if self.indent_stack.last().copied().unwrap_or(0) != width {
                self.error(LexErrorKind::InconsistentIndent);
                self.indent_stack.push(width);
            }

            self.pending_dedents = dedents.saturating_sub(1);
            return Some(Token::Dedent);
        }

        None
    }

    fn read_name(&mut self) -> Token<'a> {
        let start_offset = self.offset;

        self.consume();
        while self.peek(0).is_ascii_alphanumeric() || self.peek(0) == b'_' {
            self.consume();
        }

        Token::from_identifier(self.slice(start_offset))
    }

    fn read_number(&mut self) -> Token<'a> {
        let start_offset = self.offset;

        if self.peek(0) == b'0' && (self.peek(1) | 0x20) == b'x' {
            self.consume();
            self.consume();
            while self.peek(0).is_ascii_hexdigit() {
                self.consume();
            }

            return Token::HexLiteral(self.slice(start_offset));
        }

        let mut is_float = false;
        while self.peek(0).is_ascii_digit() {
            self.consume();
        }

        // `0. SECONDS` — numbers have no members, so a trailing `.` is the decimal point unless
        // something that could be a member name follows it.
        let trailing_point = !(self.peek(1).is_ascii_alphabetic() || self.peek(1) == b'_' || self.peek(1) == b'.');
        if self.peek(0) == b'.' && (self.peek(1).is_ascii_digit() || trailing_point) {
            is_float = true;
            self.consume();
            while self.peek(0).is_ascii_digit() {
                self.consume();
            }
        }

        // `1.#INF` and `1.#IND` are how BYOND spells infinity and not-a-number.
        if is_float && self.peek(0) == b'#' {
            let marker_offset = self.offset;
            self.consume();
            while self.peek(0).is_ascii_alphanumeric() {
                self.consume();
            }
            if !NUMBER_MARKERS.contains(&self.slice(marker_offset).to_ascii_uppercase().as_str()) {
                self.offset = marker_offset;
            }
        }

        if (self.peek(0) | 0x20) == b'e' {
            let exponent_offset = if matches!(self.peek(1), b'+' | b'-') { 2 } else { 1 };
            if self.peek(exponent_offset).is_ascii_digit() {
                is_float = true;
                for _ in 0..exponent_offset {
                    self.consume();
                }

                while self.peek(0).is_ascii_digit() {
                    self.consume();
                }
            }
        }

        let literal_str = self.slice(start_offset);
        if is_float {
            Token::FloatingPointLiteral(literal_str)
        } else {
            Token::IntegerLiteral(literal_str)
        }
    }

    fn read_string_body(&mut self, multiline: bool, begin: bool) -> Token<'a> {
        let start_offset = self.offset;

        loop {
            if self.is_eof() {
                self.error(LexErrorKind::UnterminatedString);
                let text = self.slice(start_offset);
                return if begin {
                    Token::StringLiteral(text)
                } else {
                    Token::InterpStringEnd(text)
                };
            }

            match self.peek(0) {
                b'\\' => {
                    self.consume_any();
                    if !self.is_eof() {
                        let continues_on_next_line = matches!(self.peek(0), b'\n' | b'\r');
                        self.consume_any();
                        if continues_on_next_line {
                            self.skip_blank_lines();
                        }
                    }
                },
                b'[' => {
                    let text = self.slice(start_offset);
                    self.consume();
                    self.brace_stack.push(BraceKind::Interpolation { multiline });

                    return if begin {
                        Token::InterpStringBegin(text)
                    } else {
                        Token::InterpStringMid(text)
                    };
                },
                b'"' if !multiline || self.peek(1) == b'}' => {
                    let text = self.slice(start_offset);
                    self.consume();
                    if multiline {
                        self.consume();
                    }

                    return if begin {
                        Token::StringLiteral(text)
                    } else {
                        Token::InterpStringEnd(text)
                    };
                },
                b'\n' if !multiline => {
                    self.error(LexErrorKind::UnterminatedString);
                    let text = self.slice(start_offset);
                    return if begin {
                        Token::StringLiteral(text)
                    } else {
                        Token::InterpStringEnd(text)
                    };
                },
                _ => self.consume_any(),
            }
        }
    }

    /// `'icons/obj/items.dmi'`
    fn read_resource_string(&mut self) -> Token<'a> {
        self.consume();
        let start_offset = self.offset;

        loop {
            if self.is_eof() {
                self.error(LexErrorKind::UnterminatedString);
                return Token::ResourceLiteral(self.slice(start_offset));
            }

            match self.peek(0) {
                b'\'' => {
                    let text = self.slice(start_offset);
                    self.consume();
                    return Token::ResourceLiteral(text);
                },
                b'\\' => {
                    self.consume_any();
                    if !self.is_eof() {
                        self.consume_any();
                    }
                },
                _ => self.consume_any(),
            }
        }
    }

    /// `@"raw"` and `@{"raw"}`. TODO: the `@(delim)text(delim)`
    /// The character after `@` is the delimiter and the string runs to its next occurrence, so
    /// `@"..."`, `@/.../` and `@@regex@` are all raw strings. `@{"` is the one two-character
    /// delimiter, and the only form that may span lines.
    fn read_raw_string(&mut self) -> Token<'a> {
        let at_offset = self.offset;
        self.consume();

        let multiline = self.peek(0) == b'{' && self.peek(1) == b'"';
        if multiline {
            self.consume();
            self.consume();
        } else if matches!(self.peek(0), b'\n' | b'\r') || self.is_eof() {
            self.offset = at_offset + 1;
            return Token::At;
        } else {
            self.consume();
        }

        let delimiter = self.byte_at(self.offset - 1);
        let start_offset = self.offset;
        loop {
            if self.is_eof() || (!multiline && matches!(self.peek(0), b'\n' | b'\r')) {
                self.error(LexErrorKind::UnterminatedString);
                return Token::RawStringLiteral(self.slice(start_offset));
            }

            let closes = if multiline {
                self.peek(0) == b'"' && self.peek(1) == b'}'
            } else {
                self.peek(0) == delimiter
            };
            if closes {
                let text = self.slice(start_offset);
                self.consume();
                if multiline {
                    self.consume();
                }

                return Token::RawStringLiteral(text);
            }

            self.consume_any();
        }
    }

    fn read_line_comment(&mut self) -> Token<'a> {
        let start_offset = self.offset;
        self.skip_to_end_of_line();

        Token::LineComment(self.slice(start_offset))
    }

    /// DM block comments nest, unlike C
    fn read_block_comment(&mut self) -> Token<'a> {
        let start_offset = self.offset;
        self.consume();
        self.consume();

        let mut depth = 1;
        while depth > 0 {
            if self.is_eof() {
                self.error(LexErrorKind::UnterminatedBlockComment);
                break;
            }

            match (self.peek(0), self.peek(1)) {
                (b'/', b'*') => {
                    self.consume_any();
                    self.consume_any();
                    depth += 1;
                },
                (b'/', b'/') => self.skip_to_end_of_line(),
                (b'*', b'/') => {
                    self.consume_any();
                    self.consume_any();
                    depth -= 1;
                },
                _ => self.consume_any(),
            }
        }

        Token::BlockComment(self.slice(start_offset))
    }

    fn read_rest_of_line(&mut self) -> &'a str {
        let start_offset = self.offset;
        self.skip_to_end_of_line();

        self.slice(start_offset)
    }

    fn read_directive(&mut self) -> Token<'a> {
        self.consume();
        let after_hash = self.offset;
        self.skip_horizontal_whitespace();

        let start_offset = self.offset;
        while self.peek(0).is_ascii_alphanumeric() || self.peek(0) == b'_' {
            self.consume();
        }

        match self.slice(start_offset) {
            "warn" | "warning" => Token::PpWarn(self.read_rest_of_line()),
            "error" => Token::PpError(self.read_rest_of_line()),
            word => match Token::from_directive(word) {
                Some(token) => token,
                None => {
                    self.offset = after_hash;
                    Token::Hash
                },
            },
        }
    }

    pub fn advance(&mut self) -> (Token<'a>, Location) {
        loop {
            if self.pending_dedents > 0 {
                self.pending_dedents -= 1;
                let position = self.position();
                return (Token::Dedent, Location::in_file(self.file, position, position));
            }

            if self.at_line_start {
                self.at_line_start = false;
                if self.suppress_layout() {
                    self.skip_horizontal_whitespace();
                } else if let Some(token) = self.read_indentation() {
                    let position = self.position();
                    return (token, Location::in_file(self.file, position, position));
                }
            }

            self.skip_horizontal_whitespace();

            let start_pos = self.position();
            if self.is_eof() {
                if self.close_indents_at_eof && self.indent_stack.len() > 1 {
                    self.indent_stack.pop();
                    return (Token::Dedent, Location::in_file(self.file, start_pos, start_pos));
                }

                return (Token::Eof, Location::in_file(self.file, start_pos, start_pos));
            }

            let token = match self.peek(0) {
                b'\r' => {
                    self.consume();
                    continue;
                },
                b'\n' => {
                    self.consume_any();
                    if self.suppress_layout() {
                        continue;
                    }

                    self.at_line_start = true;
                    Token::Newline
                },
                b'\\' => {
                    if self.peek(1) == b'\n' || (self.peek(1) == b'\r' && self.peek(2) == b'\n') {
                        self.consume();
                        if self.peek(0) == b'\r' {
                            self.consume();
                        }

                        self.consume_any();
                        continue;
                    }

                    if self.peek(1).is_ascii_alphanumeric() || self.peek(1) == b'_' {
                        self.consume();
                        let start_offset = self.offset;
                        self.consume();

                        Token::Identifier(self.slice(start_offset))
                    } else {
                        self.consume();
                        Token::Backslash
                    }
                },
                b'/' => match self.peek(1) {
                    b'/' => self.read_line_comment(),
                    b'*' => self.read_block_comment(),
                    b'=' => {
                        self.consume();
                        self.consume();
                        Token::DivEqual
                    },
                    _ => {
                        self.consume();
                        Token::Slash
                    },
                },
                b'"' => {
                    self.consume();
                    self.read_string_body(false, true)
                },
                b'{' => {
                    if self.peek(1) == b'"' {
                        self.consume();
                        self.consume();
                        self.read_string_body(true, true)
                    } else {
                        self.consume();
                        Token::BraceLeft
                    }
                },
                b'}' => {
                    self.consume();
                    Token::BraceRight
                },
                b'\'' => self.read_resource_string(),
                b'@' => self.read_raw_string(),
                b'#' => {
                    if self.peek(1) == b'#' {
                        self.consume();
                        self.consume();
                        if self.peek(0) == b'#' {
                            self.consume();
                            Token::Repeat
                        } else {
                            Token::Concat
                        }
                    } else {
                        self.read_directive()
                    }
                },
                b'(' => {
                    self.consume();
                    self.paren_depth += 1;
                    Token::ParenLeft
                },
                b')' => {
                    self.consume();
                    self.paren_depth = self.paren_depth.saturating_sub(1);
                    Token::ParenRight
                },
                b'[' => {
                    self.consume();
                    self.brace_stack.push(BraceKind::Square);
                    Token::SquareLeft
                },
                b']' => {
                    self.consume();
                    match self.brace_stack.pop() {
                        Some(BraceKind::Interpolation { multiline }) => self.read_string_body(multiline, false),
                        _ => Token::SquareRight,
                    }
                },
                b'=' => {
                    self.consume();
                    if self.peek(0) == b'=' {
                        self.consume();
                        Token::CompareEqual
                    } else {
                        Token::Equal
                    }
                },
                b'+' => {
                    self.consume();
                    match self.peek(0) {
                        b'=' => {
                            self.consume();
                            Token::AddEqual
                        },
                        b'+' => {
                            self.consume();
                            Token::Increment
                        },
                        _ => Token::Add,
                    }
                },
                b'-' => {
                    self.consume();
                    match self.peek(0) {
                        b'=' => {
                            self.consume();
                            Token::SubEqual
                        },
                        b'-' => {
                            self.consume();
                            Token::Decrement
                        },
                        _ => Token::Sub,
                    }
                },
                b'*' => {
                    self.consume();
                    match self.peek(0) {
                        b'=' => {
                            self.consume();
                            Token::MulEqual
                        },
                        b'*' => {
                            self.consume();
                            if self.peek(0) == b'=' {
                                self.consume();
                                Token::PowEqual
                            } else {
                                Token::Pow
                            }
                        },
                        _ => Token::Mul,
                    }
                },
                b'%' => {
                    self.consume();
                    match self.peek(0) {
                        b'%' => {
                            self.consume();
                            if self.peek(0) == b'=' {
                                self.consume();
                                Token::FloatModuloEqual
                            } else {
                                Token::FloatModulo
                            }
                        },
                        b'=' => {
                            self.consume();
                            Token::ModEqual
                        },
                        _ => Token::Modulo,
                    }
                },
                b'<' => {
                    self.consume();
                    match self.peek(0) {
                        b'=' => {
                            self.consume();
                            if self.peek(0) == b'>' {
                                self.consume();
                                Token::CompareThreeWay
                            } else {
                                Token::LessEqual
                            }
                        },
                        b'>' => {
                            self.consume();
                            Token::CompareNotEqualAlt
                        },
                        b'<' => {
                            self.consume();
                            if self.peek(0) == b'=' {
                                self.consume();
                                Token::ShiftLeftEqual
                            } else {
                                Token::ShiftLeft
                            }
                        },
                        _ => Token::AngleLeft,
                    }
                },
                b'>' => {
                    self.consume();
                    match self.peek(0) {
                        b'=' => {
                            self.consume();
                            Token::GreaterEqual
                        },
                        b'>' => {
                            self.consume();
                            if self.peek(0) == b'=' {
                                self.consume();
                                Token::ShiftRightEqual
                            } else {
                                Token::ShiftRight
                            }
                        },
                        _ => Token::AngleRight,
                    }
                },
                b'&' => {
                    self.consume();
                    match self.peek(0) {
                        b'&' => {
                            self.consume();
                            if self.peek(0) == b'=' {
                                self.consume();
                                Token::LogicalAndEqual
                            } else {
                                Token::LogicalAnd
                            }
                        },
                        b'=' => {
                            self.consume();
                            Token::AndEqual
                        },
                        _ => Token::BitAnd,
                    }
                },
                b'|' => {
                    self.consume();
                    match self.peek(0) {
                        b'|' => {
                            self.consume();
                            if self.peek(0) == b'=' {
                                self.consume();
                                Token::LogicalOrEqual
                            } else {
                                Token::LogicalOr
                            }
                        },
                        b'=' => {
                            self.consume();
                            Token::OrEqual
                        },
                        _ => Token::BitOr,
                    }
                },
                b'^' => {
                    self.consume();
                    if self.peek(0) == b'=' {
                        self.consume();
                        Token::XorEqual
                    } else {
                        Token::BitXor
                    }
                },
                b'~' => {
                    self.consume();
                    match self.peek(0) {
                        b'=' => {
                            self.consume();
                            Token::Equivalent
                        },
                        b'!' => {
                            self.consume();
                            Token::NotEquivalent
                        },
                        _ => Token::BitNot,
                    }
                },
                b'!' => {
                    self.consume();
                    if self.peek(0) == b'=' {
                        self.consume();
                        Token::CompareNotEqual
                    } else {
                        Token::Exclaim
                    }
                },
                b'?' => {
                    self.consume();
                    match self.peek(0) {
                        b'.' => {
                            self.consume();
                            Token::SafeDot
                        },
                        b':' => {
                            self.consume();
                            Token::SafeColon
                        },
                        b'[' => {
                            self.consume();
                            self.brace_stack.push(BraceKind::Square);
                            Token::SafeSquare
                        },
                        _ => Token::Question,
                    }
                },
                b':' => {
                    self.consume();
                    match self.peek(0) {
                        b':' => {
                            self.consume();
                            Token::Scope
                        },
                        b'=' => {
                            self.consume();
                            Token::AssignInto
                        },
                        _ => Token::Colon,
                    }
                },
                b'.' => {
                    self.consume();
                    if self.peek(0).is_ascii_digit() {
                        self.offset -= 1;
                        self.read_number()
                    } else if self.peek(0) == b'.' {
                        self.consume();
                        if self.peek(0) == b'.' {
                            self.consume();
                            Token::Ellipsis
                        } else {
                            Token::Super
                        }
                    } else {
                        Token::Dot
                    }
                },
                b',' => {
                    self.consume();
                    Token::Comma
                },
                b';' => {
                    self.consume();
                    Token::Semicolon
                },
                c if c.is_ascii_alphabetic() || c == b'_' => self.read_name(),
                c if c.is_ascii_digit() => self.read_number(),
                c => {
                    self.error(LexErrorKind::UnexpectedCharacter(c as char));
                    self.consume_any();
                    continue;
                },
            };

            return self.located(token, start_pos);
        }
    }

    pub fn next(&mut self, skip_comments: bool) -> (Token<'a>, Location) {
        loop {
            let (token, location) = self.advance();
            if skip_comments && token.is_comment() {
                continue;
            }

            return (token, location);
        }
    }
}

pub fn tokenize(buffer_view: &str) -> (Vec<(Token<'_>, Location)>, Vec<LexError>) {
    let mut lexer = Lexer::new(buffer_view);
    let mut tokens = Vec::new();

    loop {
        let (token, location) = lexer.advance();
        let is_eof = token == Token::Eof;
        tokens.push((token, location));

        if is_eof {
            break;
        }
    }

    let errors = lexer.errors().to_vec();
    (tokens, errors)
}

#[cfg(test)]
mod tests {
    use crate::{
        Lexer,
        token::{SoftKeyword, Token},
    };

    fn tokens(source: &str) -> Vec<Token<'_>> {
        let mut lexer = Lexer::new(source);
        let mut out = Vec::new();

        loop {
            let (token, _) = lexer.next(true);
            if token == Token::Eof {
                break;
            }

            out.push(token);
        }

        out
    }

    #[test]
    fn lexes_a_type_path() {
        assert_eq!(
            tokens("/obj/item/weapon"),
            vec![
                Token::Slash,
                Token::Identifier("obj"),
                Token::Slash,
                Token::Identifier("item"),
                Token::Slash,
                Token::Identifier("weapon"),
            ]
        );
    }

    #[test]
    fn tracks_indentation() {
        let source = "/obj/foo\n\tname = \"foo\"\n\t\ticon = 'a.dmi'\n/obj/bar\n";
        let out = tokens(source);

        assert!(out.contains(&Token::Indent));
        assert!(out.contains(&Token::Dedent));
        assert!(out.contains(&Token::StringLiteral("foo")));
        assert!(out.contains(&Token::ResourceLiteral("a.dmi")));
    }

    #[test]
    fn splits_interpolated_strings() {
        assert_eq!(
            tokens("\"hi [name] there\"")
                .into_iter()
                .filter(|t| !t.is_layout())
                .collect::<Vec<_>>(),
            vec![
                Token::InterpStringBegin("hi "),
                Token::Identifier("name"),
                Token::InterpStringEnd(" there"),
            ]
        );
    }

    #[test]
    fn reads_directives_anywhere_but_lets_concat_win() {
        assert_eq!(tokens("#define FOO 1")[0], Token::PpDefine);
        assert_eq!(tokens("#define DEFINE #define")[2], Token::PpDefine);
        assert_eq!(tokens("a ## b")[1], Token::Concat);
        assert_eq!(tokens("#ARG")[0], Token::Hash);
    }

    #[test]
    fn takes_warn_and_error_messages_verbatim() {
        assert_eq!(
            tokens("#warn you're gonna have a bad time")[0],
            Token::PpWarn(" you're gonna have a bad time")
        );
    }

    #[test]
    fn splits_escaped_identifiers() {
        assert_eq!(
            tokens("ES\\KE"),
            vec![Token::Identifier("ES"), Token::Identifier("K"), Token::Identifier("E"),]
        );
    }

    #[test]
    fn line_comments_hide_the_end_of_a_block_comment() {
        let mut lexer = Lexer::new("/*\n// */\n1");
        let (token, _) = lexer.next(true);

        assert_eq!(token, Token::Eof);
        assert!(!lexer.errors().is_empty());
    }

    #[test]
    fn nests_block_comments() {
        let mut lexer = Lexer::new("/* a /* b */ c */ 1");
        let (token, _) = lexer.next(true);

        assert_eq!(token, Token::IntegerLiteral("1"));
    }

    #[test]
    fn lexes_extended_parser_keywords_and_operators() {
        assert_eq!(
            tokens("catch final %% %%= <=> :="),
            vec![
                Token::Catch,
                Token::Soft(SoftKeyword::Final),
                Token::FloatModulo,
                Token::FloatModuloEqual,
                Token::CompareThreeWay,
                Token::AssignInto,
            ]
        );
    }

    #[test]
    fn takes_any_character_as_a_raw_string_delimiter() {
        assert_eq!(tokens("@\"plain\""), vec![Token::RawStringLiteral("plain")]);
        assert_eq!(tokens("@/slashy/"), vec![Token::RawStringLiteral("slashy")]);
        assert_eq!(tokens("@@#(a|b){6}@"), vec![Token::RawStringLiteral("#(a|b){6}")]);
        assert_eq!(
            tokens("@{\"over\nlines\"}"),
            vec![Token::RawStringLiteral("over\nlines")]
        );
    }

    #[test]
    fn reads_trailing_decimal_points_and_byond_infinities() {
        assert_eq!(
            tokens("0. + 1"),
            vec![
                Token::FloatingPointLiteral("0."),
                Token::Add,
                Token::IntegerLiteral("1"),
            ]
        );
        assert_eq!(tokens("1.#INF"), vec![Token::FloatingPointLiteral("1.#INF")]);
        assert_eq!(tokens("1.#IND"), vec![Token::FloatingPointLiteral("1.#IND")]);
    }

    #[test]
    fn line_leading_block_comments_leave_indentation_alone() {
        let out = tokens("/obj/foo\n\tname = \"a\"\n/*\n\tstray\n*/\n\tdesc = \"b\"\n");

        assert_eq!(out.iter().filter(|t| **t == Token::Indent).count(), 1);
        assert_eq!(out.iter().filter(|t| **t == Token::Dedent).count(), 1);
    }

    #[test]
    fn continued_strings_survive_the_blank_lines_between_them() {
        let mut lexer = Lexer::new("\"one \\\n\ttwo \\\n\n\tthree\"\n");
        let (token, _) = lexer.next(true);

        assert!(matches!(token, Token::StringLiteral(_)));
        assert!(lexer.errors().is_empty());
    }
}
