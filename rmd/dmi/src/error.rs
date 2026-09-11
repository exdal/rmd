#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataError(pub String);

impl std::fmt::Display for MetadataError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "bad dmi metadata: {}", self.0) }
}

#[derive(Debug)]
pub enum IconError {
    Io(std::io::Error),
    MissingMetadata,
    Metadata(MetadataError),
    Decode(String),
}

impl std::fmt::Display for IconError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::MissingMetadata => write!(f, "png has no dmi metadata"),
            Self::Metadata(e) => write!(f, "{e}"),
            Self::Decode(e) => write!(f, "png decode failed: {e}"),
        }
    }
}

impl std::error::Error for IconError {}

impl From<std::io::Error> for IconError {
    fn from(e: std::io::Error) -> Self { Self::Io(e) }
}

impl From<MetadataError> for IconError {
    fn from(e: MetadataError) -> Self { Self::Metadata(e) }
}

impl From<png::DecodingError> for IconError {
    fn from(e: png::DecodingError) -> Self {
        match e {
            png::DecodingError::IoError(e) => Self::Io(e),
            other => Self::Decode(other.to_string()),
        }
    }
}
