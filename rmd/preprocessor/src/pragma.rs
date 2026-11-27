use std::path::Path;

use base::location::Location;
use lexer::token::Token;

use crate::{
    Preprocessor, RawToken,
    diagnostic::{Code, Level},
};

impl<'a> Preprocessor<'a> {
    pub(super) fn directive_pragma(&mut self, location: Location) {
        let mut arguments = Vec::new();
        while let Some(raw) = self.next_raw() {
            if raw.location.file != location.file || matches!(raw.token, Token::Newline | Token::SuppressedNewline) {
                self.push_back(raw);
                break;
            }

            if !raw.token.is_comment() {
                arguments.push(raw);
            }
        }

        let Some((&raw, arguments)) = arguments.split_first() else {
            self.diagnose(
                Code::BadDirective,
                location,
                "expected a pragma command or warning identifier",
            );

            return;
        };

        let command = raw.token.word();
        match command {
            Some("push" | "pop") => {
                if let Some(extra) = arguments.first() {
                    self.diagnose(
                        Code::BadDirective,
                        extra.location,
                        "unexpected argument to #pragma push/pop",
                    );
                } else if command == Some("push") {
                    self.pragmas.push();
                } else if !self.pragmas.pop() {
                    self.diagnose(
                        Code::BadDirective,
                        raw.location,
                        "#pragma pop without a matching #pragma push",
                    );
                }

                return;
            },
            Some("multiple") => {
                // same as BYOND, tokens come after directive is noop so we don't do any checks
                let path = self.sources.path(location.file).map(Path::to_path_buf);
                if let Some(path) = path {
                    // also, one source file can define multiple multiples (mm yes) so just insert works
                    self.multiple.insert(path);
                }

                return;
            },
            _ => {},
        }

        let level = match command {
            Some("ignore") => Some(Level::Disabled),
            Some("warn") => Some(Level::Warning),
            Some("error") => Some(Level::Error),
            _ => None,
        };

        if let Some(level) = level {
            if arguments.is_empty() || arguments.len() % 2 == 0 {
                self.diagnose(
                    Code::BadDirective,
                    arguments.last().map_or(raw.location, |arg| arg.location),
                    "expected a warning identifier after pragma command or comma",
                );
                return;
            }

            for (index, argument) in arguments.iter().enumerate() {
                let is_valid = if index % 2 == 0 {
                    argument.token.word().is_some() || matches!(argument.token, Token::IntegerLiteral(_))
                } else {
                    argument.token == Token::Comma
                };

                if !is_valid {
                    self.diagnose(
                        Code::BadDirective,
                        argument.location,
                        "expected a comma-separated list of warning identifiers",
                    );

                    return;
                }
            }

            for &argument in arguments.iter().step_by(2) {
                if let Some(code) = self.pragma_warning_code(argument, true) {
                    self.set_pragma_level(code, level, argument.location);
                }
            }

            return;
        }

        let Some(code) = self.pragma_warning_code(raw, false) else {
            return;
        };

        let [level_raw] = arguments else {
            self.diagnose(
                Code::BadDirective,
                arguments.get(1).map_or(raw.location, |arg| arg.location),
                "expected exactly one warning level after warning identifier",
            );
            return;
        };

        let level = level_raw
            .token
            .word()
            .map(str::to_ascii_lowercase)
            .and_then(|word| Level::from_pragma(&word));

        let Some(level) = level else {
            self.diagnose(
                Code::BadDirective,
                level_raw.location,
                "warnings can only be set to disabled, notice, warning, or error",
            );
            return;
        };

        self.set_pragma_level(code, level, raw.location);
    }

    fn pragma_warning_code(&mut self, raw: RawToken<'a>, allow_unknown_name: bool) -> Option<Code> {
        let token = raw.token;
        let token_location = raw.location;

        let code = match token {
            token if token.word().is_some() => {
                let code = Code::from_name(token.word().unwrap_or_default());
                if code.is_none() && allow_unknown_name {
                    return None;
                }

                code
            },
            Token::IntegerLiteral(text) => text.parse::<u16>().ok().and_then(Code::from_number),
            _ => {
                self.diagnose(
                    Code::BadDirective,
                    token_location,
                    format!("invalid warning identifier '{token}'"),
                );
                return None;
            },
        };

        if code.is_none() {
            self.diagnose(
                Code::InvalidWarningCode,
                token_location,
                format!("warning '{token}' does not exist"),
            );
        }

        code
    }

    fn set_pragma_level(&mut self, code: Code, level: Level, location: Location) {
        if !self.pragmas.set(code, level) {
            self.diagnose(
                Code::BadDirective,
                location,
                format!("{code} cannot be set: it must always be an error"),
            );
        }
    }
}
