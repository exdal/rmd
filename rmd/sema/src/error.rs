use core::{location::Location, path::TreePath, types::Identifier};
use std::path::Path;

#[derive(Clone, Debug)]
pub enum SemaErrorKind {
    UnknownType(TreePath),
    UndeclaredVar(Identifier),
    DuplicateVar(Identifier),
    DuplicateProc(Identifier),
    UnknownParentType(TreePath),
    CircularInheritance(TreePath),
}

impl std::fmt::Display for SemaErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownType(p) => write!(f, "unknown type '{p}'"),
            Self::UndeclaredVar(n) => write!(f, "var '{n}' is not declared on this type or any parent"),
            Self::DuplicateVar(n) => write!(f, "var '{n}' is declared twice"),
            Self::DuplicateProc(n) => write!(f, "proc '{n}' is declared twice"),
            Self::UnknownParentType(p) => write!(f, "unknown parent_type '{p}'"),
            Self::CircularInheritance(p) => write!(f, "circular inheritance at '{p}'"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct SemaError {
    pub location: Location,
    pub kind: SemaErrorKind,
}

impl SemaError {
    pub fn new(kind: SemaErrorKind, location: Location) -> Self { Self { location, kind } }

    pub fn display<'a>(&'a self, path: Option<&'a Path>) -> SemaErrorDisplay<'a> {
        SemaErrorDisplay { error: self, path }
    }
}

impl std::fmt::Display for SemaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Error at {}: {}", self.location.begin, self.kind)
    }
}

impl std::error::Error for SemaError {}

pub struct SemaErrorDisplay<'a> {
    error: &'a SemaError,
    path: Option<&'a Path>,
}

impl std::fmt::Display for SemaErrorDisplay<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Error at {}: {}",
            self.error.location.display(self.path),
            self.error.kind
        )
    }
}
