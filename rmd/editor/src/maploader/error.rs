use std::{fmt, path::PathBuf};

use dmm::{Coord, error::MapError};

use super::MAX_NESTING;

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub coord: Option<Coord>,
    pub module_file: Option<PathBuf>,
    pub chain: Vec<PathBuf>,
    pub kind: ErrorKind,
}

#[derive(Debug, Clone)]
pub enum ErrorKind {
    Read { path: PathBuf, message: String },
    Dmm(Vec<MapError>),
    Anchors { count: usize },
    OutsideCodebase,
    Hook(String),
    Cycle,
    NestingLimit,
    Clipped { tiles: usize },
    PreviewIdsExhausted,
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, message } => write!(f, "reading {}: {message}", path.display()),
            Self::Dmm(errors) => write!(f, "invalid module DMM: {errors:?}"),
            Self::Anchors { count } => write!(f, "module must contain exactly one anchor, found {count}"),
            Self::OutsideCodebase => f.write_str("module path escapes the codebase directory"),
            Self::Hook(text) => f.write_str(text),
            Self::Cycle => f.write_str("module references an ancestor map"),
            Self::NestingLimit => write!(f, "module nesting exceeds {MAX_NESTING} levels"),
            Self::Clipped { tiles } => write!(f, "{tiles} module tiles fall outside the map and were clipped"),
            Self::PreviewIdsExhausted => f.write_str("module preview instance IDs are exhausted"),
        }
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(coord) = self.coord {
            write!(f, "({},{},{}): ", coord.x, coord.y, coord.z)?;
        }

        if let Some(module) = &self.module_file {
            write!(f, "{}: ", module.display())?;
        }

        write!(f, "{}", self.kind)?;
        if self.chain.len() > 1 {
            f.write_str(" (chain: ")?;
            for (index, path) in self.chain.iter().enumerate() {
                if index > 0 {
                    f.write_str(" -> ")?;
                }

                write!(f, "{}", path.display())?;
            }

            f.write_str(")")?;
        }

        Ok(())
    }
}

impl std::error::Error for Diagnostic {}
