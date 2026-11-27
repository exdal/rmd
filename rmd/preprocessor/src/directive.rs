use std::path::Path;

use base::{file_path, location::Location};
use lexer::token::Token;

use crate::{
    Preprocessor, Spanned,
    define::{BodyPart, Define, Parameter},
    diagnostic::Code,
    eval,
    include::normalize,
    pastable,
};

impl<'a> Preprocessor<'a> {
    pub(super) fn handle_directive(&mut self, directive: Token<'a>, location: Location, from_file: bool) {
        // `#define DEFINE #define`
        if from_file && !self.can_use_directive {
            self.diagnose(
                Code::MisplacedDirective,
                location,
                "there can only be whitespace before a preprocessor directive",
            );
            self.skip_rest_of_line();
            return;
        }

        // `\t#define X 1`
        if !self.line_has_content {
            self.rewind_directive_line();
        }

        match directive {
            Token::PpDefine => self.directive_define(location),
            Token::PpUndef => self.directive_undef(),
            Token::PpInclude => self.directive_include(location),
            Token::PpIf => self.directive_if(location),
            Token::PpIfdef => self.directive_ifdef(location, false),
            Token::PpIfndef => self.directive_ifdef(location, true),
            Token::PpElif => self.directive_elif(location),
            Token::PpElse => self.directive_else(location),
            Token::PpEndif => self.directive_endif(location),
            Token::PpPragma => self.directive_pragma(location),
            Token::PpWarn(text) => self.diagnose(Code::WarningDirective, location, format!("#warn{text}")),
            Token::PpError(text) => self.diagnose(Code::ErrorDirective, location, format!("#error{text}")),
            other => self.diagnose(Code::BadDirective, location, format!("unexpected directive '{other}'")),
        }
    }

    /// `\t#define X 1`
    fn rewind_directive_line(&mut self) {
        self.line_layout.clear();

        let Some((depth, indent)) = self.line_indent.take() else {
            return;
        };

        if depth == self.include_stack.len()
            && let Some(state) = self.include_stack.last_mut()
        {
            state.lexer.restore_indent(indent);
        }
    }

    fn directive_define(&mut self, location: Location) {
        let Some(name_raw) = self.next_significant() else {
            return;
        };
        let name_token = name_raw.token;
        let name_location = name_raw.location;

        let Some(name) = name_token.word() else {
            self.diagnose(
                Code::BadDirective,
                name_location,
                "unexpected token, identifier expected for #define directive",
            );
            if name_token != Token::Newline {
                self.skip_rest_of_line();
            }

            return;
        };

        if name == "FILE_DIR" {
            self.directive_file_dir();
            return;
        }

        if name == "defined" {
            self.diagnose(
                Code::SoftReservedKeyword,
                name_location,
                "reserved keyword 'defined' cannot be used as a macro name",
            );
        }

        // `#define M (x) y`
        let mut params = None;
        if let Some(raw) = self.next_raw() {
            if raw.token == Token::ParenLeft && !raw.separated_from(name_location) {
                params = Some(self.read_parameters(raw.location));
            } else {
                self.push_back(raw);
            }
        }

        let body = self.read_macro_body();
        let body = match (name, body.as_slice()) {
            ("TRUE", [BodyPart::Token(Token::IntegerLiteral("1"))]) if params.is_none() => {
                vec![BodyPart::Token(Token::True)]
            },
            ("FALSE", [BodyPart::Token(Token::IntegerLiteral("0"))]) if params.is_none() => {
                vec![BodyPart::Token(Token::False)]
            },
            _ => body,
        };
        self.defines.define(Define {
            name,
            params,
            body,
            location,
            builtin: None,
        });
    }

    /// `#define FILE_DIR "icons"`
    fn directive_file_dir(&mut self) {
        let Some(raw) = self.next_significant() else {
            return;
        };
        let token = raw.token;
        let token_location = raw.location;

        let directory = match token {
            Token::StringLiteral(text) | Token::RawStringLiteral(text) => {
                Some(file_path::forward_slashes(text).into_owned())
            },
            Token::Dot => Some(".".to_string()),
            _ => None,
        };

        let Some(directory) = directory else {
            self.diagnose(
                Code::InvalidFileDirDefine,
                token_location,
                format!("'{token}' is not a valid directory"),
            );
            self.skip_rest_of_line();
            return;
        };

        let base = self.include_stack.last().map(|s| s.dir.clone()).unwrap_or_default();
        self.resource_dirs.push(normalize(&base.join(directory)));
        self.skip_rest_of_line();
    }

    fn read_parameters(&mut self, open: Location) -> Vec<Parameter<'a>> {
        let mut params: Vec<Parameter<'a>> = Vec::new();
        let mut can_consume_comma = false;
        let mut found_variadic = false;

        loop {
            let Some(raw) = self.next_significant() else {
                self.diagnose(Code::BadDirective, open, "missing ')' in macro definition");
                return params;
            };
            let token = raw.token;
            let location = raw.location;

            match token {
                Token::ParenRight => return params,
                Token::Newline | Token::SuppressedNewline => {
                    self.push_back(raw);
                    self.diagnose(Code::BadDirective, open, "missing ')' in macro definition");
                    return params;
                },
                Token::Comma => {
                    if !can_consume_comma {
                        self.diagnose(Code::BadDirective, location, "unexpected ',' in macro parameter list");
                    }

                    can_consume_comma = false;
                },
                // `#define M(...)`
                Token::Ellipsis => {
                    can_consume_comma = true;
                    if self.reject_late_variadic(&params, location, &mut found_variadic) {
                        continue;
                    }

                    params.push(Parameter {
                        name: "...",
                        variadic: true,
                    });
                    found_variadic = true;
                },
                token => {
                    let Some(name) = token.word() else {
                        self.diagnose(Code::BadDirective, location, "expected a macro parameter");
                        return params;
                    };

                    can_consume_comma = true;
                    if self.reject_late_variadic(&params, location, &mut found_variadic) {
                        continue;
                    }

                    let variadic = self.check(Token::Ellipsis);
                    found_variadic |= variadic;
                    params.push(Parameter { name, variadic });
                },
            }
        }
    }

    /// `#define M(rest..., value)`
    fn reject_late_variadic(&mut self, params: &[Parameter<'a>], location: Location, found: &mut bool) -> bool {
        if !*found {
            return false;
        }

        let name = params.last().map(|p| p.name.to_string()).unwrap_or_default();
        self.diagnose(
            Code::BadDirective,
            location,
            format!("variadic argument '{name}' must be the last argument"),
        );
        *found = false;

        true
    }

    fn read_macro_body(&mut self) -> Vec<BodyPart<'a>> {
        let mut body: Vec<BodyPart<'a>> = Vec::new();
        let mut previous: Option<Location> = None;

        while let Some(raw) = self.next_raw() {
            let token = raw.token;
            let location = raw.location;
            if matches!(token, Token::Newline | Token::SuppressedNewline) {
                break;
            }

            if token.is_comment() {
                continue;
            }

            let separated = previous.is_some_and(|p| raw.separated_from(p));
            previous = Some(location);

            match token {
                // `#PARAM`, `##PARAM`
                Token::Hash | Token::Concat => {
                    let Some(next_raw) = self.next_raw() else {
                        break;
                    };
                    let next = next_raw.token;
                    let next_location = next_raw.location;

                    let Some(name) = pastable(&next) else {
                        self.push_back(next_raw);
                        continue;
                    };

                    if token == Token::Hash {
                        if separated {
                            body.push(BodyPart::Space);
                        }

                        body.push(BodyPart::Stringify(name));
                    } else {
                        body.push(BodyPart::Paste(name));
                    }

                    previous = Some(next_location);
                },
                token => {
                    if separated {
                        body.push(BodyPart::Space);
                    }

                    body.push(BodyPart::Token(token));
                },
            }
        }

        while matches!(body.last(), Some(BodyPart::Space)) {
            body.pop();
        }

        body
    }

    fn directive_undef(&mut self) {
        let Some(raw) = self.next_significant() else {
            return;
        };
        let token = raw.token;
        let token_location = raw.location;

        let Some(name) = token.word() else {
            self.diagnose(Code::BadDirective, token_location, "invalid macro identifier");
            return;
        };

        if self.defines.undef(name).is_none() {
            self.diagnose(
                Code::UndefineMissingDirective,
                token_location,
                format!("no macro named \"{name}\""),
            );
        }
    }

    fn directive_if(&mut self, location: Location) {
        self.last_if = location;

        let mut tokens = std::mem::take(&mut self.condition);
        self.read_condition(&mut tokens);
        if tokens.is_empty() {
            self.condition = tokens;
            self.diagnose(Code::BadDirective, location, "expression expected for #if");
            self.conditionals.push(Some(false));
            self.skip_if_body(false);
            return;
        }

        let is_defined = |name: &str| self.defines.is_defined(name);
        let base = self.include_stack.last().map_or(Path::new(""), |s| s.dir.as_path());
        let file_exists = |path: &str| base.join(file_path::forward_slashes(path).as_ref()).is_file();

        let (value, complaints) = eval::evaluate(
            &tokens,
            &eval::Context {
                is_defined: &is_defined,
                file_exists: &file_exists,
            },
        );

        self.condition = tokens;

        for complaint in complaints {
            self.diagnose(Code::BadDirective, complaint.location, complaint.message);
        }

        let Some(value) = value else {
            self.diagnose(Code::BadDirective, location, "expression is invalid");
            self.conditionals.push(Some(false));
            self.skip_if_body(false);
            return;
        };

        let taken = value != 0.0;
        self.conditionals.push(Some(taken));

        if !taken {
            self.skip_if_body(false);
        }
    }

    /// `#if defined(X) && fexists("x")`
    fn read_condition(&mut self, tokens: &mut Vec<Spanned<'a>>) {
        tokens.clear();
        let mut expand = true;

        while let Some(raw) = self.next_raw() {
            let token = raw.token;
            let location = raw.location;
            if matches!(token, Token::Newline | Token::SuppressedNewline) {
                break;
            }

            if token.is_comment() {
                continue;
            }

            if let Some(word) = token.word() {
                if word == "defined" {
                    expand = false;
                } else if expand && self.try_macro(word, raw) {
                    continue;
                }
            } else if token == Token::ParenRight {
                expand = true;
            }

            tokens.push((token, location));
        }
    }

    fn directive_ifdef(&mut self, location: Location, negated: bool) {
        self.last_if = location;

        let Some(raw) = self.next_significant() else {
            return;
        };
        let token = raw.token;
        let token_location = raw.location;

        let Some(name) = token.word() else {
            self.diagnose(Code::BadDirective, token_location, "expected a define identifier");
            self.conditionals.push(Some(false));
            self.skip_if_body(false);
            return;
        };

        let taken = self.defines.is_defined(name) != negated;
        self.conditionals.push(Some(taken));

        if !taken {
            self.skip_if_body(false);
        }
    }

    fn directive_elif(&mut self, location: Location) {
        match self.conditionals.last().copied() {
            None => {
                self.diagnose(Code::BadDirective, location, "unexpected #elif");
                self.skip_if_body(false);
            },
            Some(None) => {
                self.diagnose(
                    Code::BadDirective,
                    location,
                    "directive #elif cannot appear after #else in its flow control",
                );
                self.skip_if_body(false);
            },
            Some(Some(true)) => self.skip_if_body(false),
            Some(Some(false)) => {
                self.conditionals.pop();
                self.directive_if(location);
            },
        }
    }

    fn directive_else(&mut self, location: Location) {
        match self.conditionals.pop() {
            Some(Some(true)) => self.skip_if_body(true),
            Some(Some(false)) => self.conditionals.push(None),
            Some(None) => {
                self.conditionals.push(None);
                self.diagnose(Code::BadDirective, location, "unexpected #else");
            },
            None => self.diagnose(Code::BadDirective, location, "unexpected #else"),
        }
    }

    fn directive_endif(&mut self, location: Location) {
        if self.conditionals.pop().is_none() {
            self.diagnose(Code::BadDirective, location, "unexpected #endif");
        }
    }

    /// `#if 0 ... #endif`
    fn skip_if_body(&mut self, from_else: bool) {
        let carried = std::mem::take(&mut self.carried_layout);
        let line = std::mem::take(&mut self.line_layout);
        let line_indent = self.line_indent.take();
        let indent = self
            .include_stack
            .last()
            .map(|state| (self.include_stack.len(), state.lexer.indent_state()));
        let nesting = self
            .include_stack
            .last()
            .map(|state| (self.include_stack.len(), state.lexer.nesting_state()));
        let errors = self
            .include_stack
            .last()
            .map(|state| (self.include_stack.len(), state.lexer.error_count()));

        let mut depth = 1usize;
        let mut closed = false;
        while let Some(raw) = self.next_raw() {
            let token = raw.token;
            let location = raw.location;
            match token {
                Token::PpIf | Token::PpIfdef | Token::PpIfndef => depth += 1,
                Token::PpEndif => {
                    depth -= 1;
                    if depth == 0 {
                        if !from_else {
                            self.push_back(raw);
                        }

                        closed = true;
                        break;
                    }
                },
                Token::PpElse | Token::PpElif if depth == 1 => {
                    if from_else {
                        self.diagnose(Code::BadDirective, location, format!("unexpected {token} directive"));
                    }

                    self.push_back(raw);
                    closed = true;
                    break;
                },
                _ => {},
            }
        }

        if !closed {
            let location = self.last_if;
            self.diagnose(Code::BadDirective, location, "missing #endif directive");
        }

        self.carried_layout = carried;
        self.line_layout = line;
        self.line_indent = line_indent;
        self.can_use_directive = true;

        if let Some((stack_depth, indent)) = indent
            && stack_depth == self.include_stack.len()
            && let Some(state) = self.include_stack.last_mut()
        {
            state.lexer.restore_indent(indent);
        }

        if let Some((stack_depth, nesting)) = nesting
            && stack_depth == self.include_stack.len()
            && let Some(state) = self.include_stack.last_mut()
        {
            state.lexer.restore_nesting(nesting);
        }

        if let Some((stack_depth, count)) = errors
            && stack_depth == self.include_stack.len()
            && let Some(state) = self.include_stack.last_mut()
        {
            state.lexer.truncate_errors(count);
        }
    }
}
