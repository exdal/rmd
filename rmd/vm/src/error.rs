use core::{location::Location, types::ProcId};

use codegen::CodeOffset;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FaultKind {
    InstructionBudget,
    CallDepth,
    Memory,
    Blocked(String),
    MissingProc(String),
    MissingVariable(String),
    Unsupported(String),
    InvalidReference,
    InvalidOperation(String),
    Trap(String),
    Thrown,
}

#[derive(Debug, Clone)]
pub struct Fault {
    pub offset: Option<CodeOffset>,
    pub proc: Option<ProcId>,
    pub location: Location,
    pub kind: FaultKind,
}

impl Fault {
    pub fn detached(kind: FaultKind) -> Self {
        Self {
            offset: None,
            proc: None,
            location: Location::default(),
            kind,
        }
    }
}

impl std::fmt::Display for Fault {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{:?} at {}", self.kind, self.location)
    }
}

impl std::error::Error for Fault {}
