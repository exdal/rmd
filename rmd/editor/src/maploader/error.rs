use std::{fmt, path::PathBuf};

use dmm::error::MapError;

use super::MAX_NESTING;
use crate::maploader::map_module;

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub root: Option<map_module::RootLocation>,
    pub module_file: Option<PathBuf>,
    pub chain: Vec<PathBuf>,
    pub kind: ErrorKind,
}

#[derive(Debug, Clone)]
pub enum ErrorKind {
    UnsupportedLoader(String),
    InvalidField(&'static str),
    Read { path: PathBuf, message: String },
    Config(String),
    MissingKey,
    EmptyModules,
    Dmm(Vec<MapError>),
    Connectors { count: usize },
    Cycle,
    NestingLimit,
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedLoader(name) => write!(f, "unsupported modular loader {name:?}"),
            Self::InvalidField(field) => write!(f, "modular root {field} must be a string or null"),
            Self::Read { path, message } => write!(f, "reading {}: {message}", path.display()),
            Self::Config(message) => write!(f, "invalid modular config: {message}"),
            Self::MissingKey => f.write_str("room key is missing from the modular config"),
            Self::EmptyModules => f.write_str("room modules list is empty"),
            Self::Dmm(errors) => write!(f, "invalid module DMM: {errors:?}"),
            Self::Connectors { count } => write!(f, "module must contain exactly one connector, found {count}"),
            Self::Cycle => f.write_str("module references an ancestor map"),
            Self::NestingLimit => write!(f, "module nesting exceeds {MAX_NESTING} levels"),
        }
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(root) = &self.root {
            write!(
                f,
                "{} at ({},{},{}) prefab {}",
                root.source_map.display(),
                root.coord.x,
                root.coord.y,
                root.coord.z,
                root.prefab_index
            )?;
            if let Some(config) = &root.config_file {
                write!(f, " config {}", config.display())?;
            }
            if let Some(key) = &root.key {
                write!(f, " key {key:?}")?;
            }
            f.write_str(": ")?;
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
