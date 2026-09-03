use core::location::Location;
use std::path::Path;

use crate::diagnostic::{Code, Level};

#[derive(Clone, Debug)]
pub struct PreprocessError {
    pub location: Location,
    pub code: Code,
    pub level: Level,
    pub message: String,
}

impl PreprocessError {
    pub fn new(code: Code, level: Level, location: Location, message: impl Into<String>) -> Self {
        Self {
            location,
            code,
            level,
            message: message.into(),
        }
    }

    pub fn is_fatal(&self) -> bool { self.level == Level::Error }

    pub fn display<'a>(&'a self, path: Option<&'a Path>) -> PreprocessErrorDisplay<'a> {
        PreprocessErrorDisplay { error: self, path }
    }
}

impl std::fmt::Display for PreprocessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} {} at {}: {}",
            self.level, self.code, self.location.begin, self.message
        )
    }
}

impl std::error::Error for PreprocessError {}

pub struct PreprocessErrorDisplay<'a> {
    error: &'a PreprocessError,
    path: Option<&'a Path>,
}

impl std::fmt::Display for PreprocessErrorDisplay<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let error = self.error;

        write!(
            f,
            "{} {} at {}: {}",
            error.level,
            error.code,
            error.location.display(self.path),
            error.message
        )
    }
}
