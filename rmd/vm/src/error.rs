use core::{location::Location, types::ProcId};

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
    Thrown,
}

#[derive(Debug, Clone)]
pub struct Fault {
    pub node: Option<ir::IrNodeId>,
    pub proc: Option<ProcId>,
    pub location: Location,
    pub kind: FaultKind,
}

impl std::fmt::Display for Fault {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{:?} at {}", self.kind, self.location)
    }
}

impl std::error::Error for Fault {}
