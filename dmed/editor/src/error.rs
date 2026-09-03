use ast::error::ParseError;
use preprocessor::error::PreprocessError;

#[derive(Debug)]
pub enum LoadError {
    Preprocess(PreprocessError),
    Parse(ParseError),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Preprocess(e) => write!(f, "{e}"),
            Self::Parse(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for LoadError {}

impl From<PreprocessError> for LoadError {
    fn from(e: PreprocessError) -> Self { Self::Preprocess(e) }
}

impl From<ParseError> for LoadError {
    fn from(e: ParseError) -> Self { Self::Parse(e) }
}
