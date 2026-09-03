use core::location::Location;

use lexer::token::Token;

/// `defined(X)`, `fexists("x")`
pub struct Context<'ctx> {
    pub is_defined: &'ctx dyn Fn(&str) -> bool,
    pub file_exists: &'ctx dyn Fn(&str) -> bool,
}

pub struct Complaint {
    pub location: Location,
    pub message: String,
}

/// `#if 1 + 2`
pub fn evaluate(tokens: &[(Token<'_>, Location)], context: &Context<'_>) -> (Option<f32>, Vec<Complaint>) {
    let mut parser = Parser {
        tokens,
        index: 0,
        context,
        complaints: Vec::new(),
    };

    let value = parser.expression();

    if parser.index < parser.tokens.len() {
        let location = parser.location();
        parser.complain(location, "trailing tokens in preprocessor expression");
    }

    (value, parser.complaints)
}

struct Parser<'a, 'ctx> {
    tokens: &'a [(Token<'a>, Location)],
    index: usize,
    context: &'ctx Context<'ctx>,
    complaints: Vec<Complaint>,
}

impl<'a> Parser<'a, '_> {
    fn current(&self) -> Option<Token<'a>> { self.tokens.get(self.index).map(|(t, _)| *t) }

    fn location(&self) -> Location {
        self.tokens
            .get(self.index)
            .or_else(|| self.tokens.last())
            .map(|(_, l)| *l)
            .unwrap_or_default()
    }

    fn advance(&mut self) { self.index += 1; }

    fn check(&mut self, token: Token<'a>) -> bool {
        if self.current() == Some(token) {
            self.advance();
            return true;
        }

        false
    }

    fn check_any(&mut self, tokens: &[Token<'a>]) -> Option<Token<'a>> {
        let current = self.current()?;
        if tokens.contains(&current) {
            self.advance();
            return Some(current);
        }

        None
    }

    fn complain(&mut self, location: Location, message: impl Into<String>) {
        self.complaints.push(Complaint {
            location,
            message: message.into(),
        });
    }

    fn expression(&mut self) -> Option<f32> { self.logical_or() }

    fn logical_or(&mut self) -> Option<f32> {
        let mut left = self.logical_and()?;
        while self.check(Token::LogicalOr) {
            let Some(right) = self.logical_and() else {
                let location = self.location();
                self.complain(location, "expected a second value");
                break;
            };

            left = f32::from(u8::from(left != 0.0 || right != 0.0));
        }

        Some(left)
    }

    fn logical_and(&mut self) -> Option<f32> {
        let mut left = self.equality()?;
        while self.check(Token::LogicalAnd) {
            let Some(right) = self.equality() else {
                let location = self.location();
                self.complain(location, "expected a second value");
                break;
            };

            left = f32::from(u8::from(left != 0.0 && right != 0.0));
        }

        Some(left)
    }

    fn equality(&mut self) -> Option<f32> {
        let mut left = self.relational()?;
        while let Some(operator) =
            self.check_any(&[Token::CompareEqual, Token::CompareNotEqual, Token::CompareNotEqualAlt])
        {
            let Some(right) = self.relational() else {
                let location = self.location();
                self.complain(location, "expected a second value");
                break;
            };

            left = match operator {
                Token::CompareEqual => f32::from(u8::from(left == right)),
                _ => f32::from(u8::from(left != right)),
            };
        }

        Some(left)
    }

    fn relational(&mut self) -> Option<f32> {
        let mut left = self.additive()?;
        while let Some(operator) = self.check_any(&[
            Token::AngleLeft,
            Token::LessEqual,
            Token::AngleRight,
            Token::GreaterEqual,
        ]) {
            let Some(right) = self.additive() else {
                let location = self.location();
                self.complain(location, "expected a second value");
                break;
            };

            left = f32::from(u8::from(match operator {
                Token::AngleLeft => left < right,
                Token::LessEqual => left <= right,
                Token::AngleRight => left > right,
                _ => left >= right,
            }));
        }

        Some(left)
    }

    fn additive(&mut self) -> Option<f32> {
        let mut left = self.multiplicative()?;
        while let Some(operator) = self.check_any(&[Token::Add, Token::Sub]) {
            let Some(right) = self.multiplicative() else {
                let location = self.location();
                self.complain(location, "expected a second value");
                break;
            };

            left = if operator == Token::Add {
                left + right
            } else {
                left - right
            };
        }

        Some(left)
    }

    fn multiplicative(&mut self) -> Option<f32> {
        let mut left = self.power()?;
        while let Some(operator) = self.check_any(&[Token::Mul, Token::Slash, Token::Modulo]) {
            let Some(right) = self.power() else {
                let location = self.location();
                self.complain(location, "expected a second value");
                break;
            };

            left = match operator {
                Token::Mul => left * right,
                Token::Slash => left / right,
                _ => left % right,
            };
        }

        Some(left)
    }

    fn power(&mut self) -> Option<f32> {
        let left = self.unary()?;
        if self.check(Token::Pow) {
            let Some(right) = self.power() else {
                let location = self.location();
                self.complain(location, "expected a second value");
                return Some(left);
            };

            return Some(left.powf(right));
        }

        Some(left)
    }

    fn unary(&mut self) -> Option<f32> {
        if self.check(Token::Exclaim) {
            let value = self.unary()?;

            return Some(f32::from(u8::from(value == 0.0)));
        }

        self.sign()
    }

    fn sign(&mut self) -> Option<f32> {
        if let Some(operator) = self.check_any(&[Token::Add, Token::Sub]) {
            let value = self.sign()?;

            return Some(if operator == Token::Sub { -value } else { value });
        }

        self.primary()
    }

    fn primary(&mut self) -> Option<f32> {
        let location = self.location();

        match self.current()? {
            Token::ParenLeft => {
                self.advance();
                let inner = self.expression();
                if !self.check(Token::ParenRight) {
                    self.complain(location, "expected ')' to close expression");
                }

                inner
            },
            token if token.word() == Some("defined") => {
                self.advance();
                let name = self.call_argument(location, "defined")?.word();
                let Some(name) = name else {
                    self.complain(location, "defined() expects a macro name");
                    return Some(0.0);
                };

                Some(f32::from(u8::from((self.context.is_defined)(name))))
            },
            token if token.word() == Some("fexists") => {
                self.advance();
                let path = self.call_argument(location, "fexists")?;
                let Some(path) = path.text().filter(|_| path.is_literal()) else {
                    self.complain(location, "fexists() expects a file path");
                    return Some(0.0);
                };

                Some(f32::from(u8::from((self.context.file_exists)(path))))
            },
            Token::Null => {
                self.advance();

                Some(0.0)
            },
            Token::IntegerLiteral(text) | Token::FloatingPointLiteral(text) => {
                self.advance();
                match text.parse::<f32>() {
                    Ok(value) => Some(value),
                    Err(_) => {
                        self.complain(location, format!("'{text}' is not a number"));
                        Some(0.0)
                    },
                }
            },
            Token::HexLiteral(text) => {
                self.advance();
                match u32::from_str_radix(text.trim_start_matches("0x").trim_start_matches("0X"), 16) {
                    Ok(value) => Some(value as f32),
                    Err(_) => {
                        self.complain(location, format!("'{text}' is not a number"));
                        Some(0.0)
                    },
                }
            },
            token if token.is_literal() => {
                self.advance();
                self.complain(location, "strings are not valid in preprocessor expressions");

                Some(0.0)
            },
            token => {
                self.advance();
                self.complain(location, format!("'{token}' is not valid in a preprocessor expression"));

                Some(0.0)
            },
        }
    }

    /// `defined(X)`, `fexists("x")`
    fn call_argument(&mut self, location: Location, name: &str) -> Option<Token<'a>> {
        if !self.check(Token::ParenLeft) {
            self.complain(location, format!("expected '(' to begin {name}() expression"));
            return None;
        }

        let argument = self.current()?;
        self.advance();

        if !self.check(Token::ParenRight) {
            self.complain(location, format!("expected ')' to end {name}() expression"));
        }

        Some(argument)
    }
}
