use core::location::Location;

#[derive(Clone, Debug)]
pub enum LexErrorKind {
    UnexpectedCharacter(char),
    UnterminatedString,
    UnterminatedBlockComment,
    InconsistentIndent,
}

impl std::fmt::Display for LexErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnexpectedCharacter(c) => write!(f, "unexpected character '{c}'"),
            Self::UnterminatedString => write!(f, "unterminated string literal"),
            Self::UnterminatedBlockComment => write!(f, "unterminated block comment"),
            Self::InconsistentIndent => write!(f, "inconsistent indentation"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct LexError {
    pub location: Location,
    pub kind: LexErrorKind,
}

impl std::fmt::Display for LexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Lexer error at {}: {}", self.location.begin, self.kind)
    }
}

impl std::error::Error for LexError {}
