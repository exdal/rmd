use core::location::Location;

#[derive(Clone, Debug)]
pub enum ParseErrorKind {
    EndOfFile,
    InvalidToken,
    UnexpectedToken { expected: String, got: String },
    ExpectedPath,
    ExpectedExpression,
    UnexpectedIndent,
    InvalidVarModifier(String),
    ExpressionLimit,
}

impl std::fmt::Display for ParseErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EndOfFile => write!(f, "end of file"),
            Self::InvalidToken => write!(f, "invalid token"),
            Self::UnexpectedToken { expected, got } => write!(f, "expected '{expected}' got '{got}'"),
            Self::ExpectedPath => write!(f, "expected a type path"),
            Self::ExpectedExpression => write!(f, "expected an expression"),
            Self::UnexpectedIndent => write!(f, "unexpected indent"),
            Self::InvalidVarModifier(s) => write!(f, "invalid var modifier '{s}'"),
            Self::ExpressionLimit => write!(f, "too many expressions in one file"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ParseError {
    pub location: Location,
    pub kind: ParseErrorKind,
}

impl ParseError {
    pub fn end_of_file() -> Self {
        Self {
            kind: ParseErrorKind::EndOfFile,
            location: Location::default(),
        }
    }

    pub fn expression_limit(location: Location) -> Self {
        Self {
            kind: ParseErrorKind::ExpressionLimit,
            location,
        }
    }

    pub fn invalid_token(location: Location) -> Self {
        Self {
            kind: ParseErrorKind::InvalidToken,
            location,
        }
    }

    pub fn unexpected(expected: impl std::fmt::Display, got: impl std::fmt::Display, location: Location) -> Self {
        Self {
            kind: ParseErrorKind::UnexpectedToken {
                expected: expected.to_string(),
                got: got.to_string(),
            },
            location,
        }
    }

    pub fn expected_path(location: Location) -> Self {
        Self {
            kind: ParseErrorKind::ExpectedPath,
            location,
        }
    }

    pub fn expected_expression(location: Location) -> Self {
        Self {
            kind: ParseErrorKind::ExpectedExpression,
            location,
        }
    }
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Parser error at {}: {}", self.location.begin, self.kind)
    }
}

impl std::error::Error for ParseError {}
