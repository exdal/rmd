use core::{
    location::{FileId, Location},
    types::ProcId,
};
use std::{fmt::Write, path::Path};

use codegen::{CodeOffset, Module};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FaultKind {
    InstructionBudget,
    CallDepth,
    Memory,
    Blocked(String),
    Sleep,
    MissingProc(String),
    MissingVariable(String),
    Unsupported(String),
    InvalidReference,
    /// `read null.reagents`
    Null(String),
    InvalidOperation(String),
    Trap(String),
    Thrown(String),
}

impl std::fmt::Display for FaultKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InstructionBudget => formatter.write_str("instruction budget exceeded"),
            Self::CallDepth => formatter.write_str("call depth exceeded"),
            Self::Memory => formatter.write_str("allocation limit exceeded"),
            Self::Sleep => formatter.write_str("blocked by the sandbox: sleep"),
            Self::Blocked(what) => write!(formatter, "blocked by the sandbox: {what}"),
            Self::MissingProc(name) => write!(formatter, "undefined proc {name}"),
            Self::MissingVariable(name) => write!(formatter, "undefined var {name}"),
            Self::Unsupported(what) => write!(formatter, "unsupported: {what}"),
            Self::InvalidReference => formatter.write_str("invalid reference"),
            Self::Null(access) => write!(formatter, "cannot {access}"),
            Self::InvalidOperation(what) => write!(formatter, "invalid operation: {what}"),
            Self::Trap(reason) => write!(formatter, "trap: {reason}"),
            Self::Thrown(what) if what.is_empty() => formatter.write_str("uncaught exception"),
            Self::Thrown(what) => write!(formatter, "uncaught exception: {what}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct StackFrame {
    pub proc: ProcId,
    pub offset: Option<CodeOffset>,
    pub location: Location,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FaultKey {
    proc: Option<ProcId>,
    file: FileId,
    line: u32,
    col: u32,
    kind: FaultKind,
}

#[derive(Debug, Clone)]
pub struct Fault {
    pub kind: FaultKind,
    pub location: Location,
    pub frames: Vec<StackFrame>,
}

impl Fault {
    pub fn detached(kind: FaultKind) -> Self {
        Self {
            kind,
            location: Location::default(),
            frames: Vec::new(),
        }
    }

    pub fn proc(&self) -> Option<ProcId> { self.frames.first().map(|frame| frame.proc) }

    pub fn key(&self) -> FaultKey {
        FaultKey {
            proc: self.proc(),
            file: self.location.file,
            line: self.location.begin.line,
            col: self.location.begin.col,
            kind: self.kind.clone(),
        }
    }

    pub fn outermost_proc_id(&self) -> Option<ProcId> { self.frames.last().map(|frame| frame.proc) }

    /// A fault raised before `proc` ran its first instruction, such as setting up its globals
    pub fn entering(mut self, module: &Module, proc: ProcId) -> Self {
        if !self.frames.is_empty() {
            return self;
        }

        let location = module
            .function_for_proc(proc)
            .map(|function| function.location)
            .unwrap_or_default();
        self.location = location;
        self.frames.push(StackFrame {
            proc,
            offset: None,
            location,
        });

        self
    }

    /// ```text
    /// undefined var FOO in /obj/thing::update_icon at code/thing.dm:12:1
    ///   at /obj/thing::update_icon (code/thing.dm:12:1)
    ///   at /datum/demir/example::bake (code/demir.dm:40:1)
    /// ```
    pub fn format_fault_report<'p>(&self, module: &Module, path: impl Fn(FileId) -> Option<&'p Path>) -> String {
        let name = |proc| proc_name(module, proc);
        let mut report = self.kind.to_string();
        if let Some(proc) = self.proc() {
            let _ = write!(report, " in {}", name(proc));
        }

        if self.location.begin.line > 0 {
            let _ = write!(report, " at {}", self.location.display(path(self.location.file)));
        }

        for frame in &self.frames {
            let _ = write!(
                report,
                "\n  at {} ({})",
                name(frame.proc),
                frame.location.display(path(frame.location.file))
            );
        }

        report
    }
}

/// `/obj/thing::update_icon`
pub fn proc_name(module: &Module, proc: ProcId) -> &str {
    module
        .function_for_proc(proc)
        .and_then(|function| module.strings.get(function.name.0 as usize))
        .map_or("<unknown proc>", String::as_str)
}

impl std::fmt::Display for Fault {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{} at {}", self.kind, self.location)
    }
}

impl std::error::Error for Fault {}
