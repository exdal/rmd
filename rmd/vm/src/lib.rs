use core::types::ProcId;
use std::collections::HashMap;

mod builtins;
mod error;
pub mod eval;
pub mod heap;
pub mod json;
pub mod value;
pub mod world;

pub use error::{Fault, FaultKind};
pub use eval::Runtime;
pub use value::{AppearanceDelta, GenericValue};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub instruction_budget: u64,
    pub call_depth: usize,
    pub allocations: usize,
    pub text_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            instruction_budget: 100_000,
            call_depth: 48,
            allocations: 100_000,
            text_bytes: 1_048_576,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub fault: Fault,
    pub count: usize,
}

#[derive(Debug, Default)]
pub struct Diagnostics {
    pub entries: Vec<Diagnostic>,
    index: HashMap<(Option<ProcId>, u32, usize, usize, FaultKind), usize>,
}

impl Diagnostics {
    pub fn record(&mut self, fault: Fault) {
        let key = (
            fault.proc,
            fault.location.file.0,
            fault.location.begin.line,
            fault.location.begin.col,
            fault.kind.clone(),
        );

        if let Some(index) = self.index.get(&key) {
            if let Some(entry) = self.entries.get_mut(*index) {
                entry.count += 1;
            }
        } else {
            self.index.insert(key, self.entries.len());
            self.entries.push(Diagnostic { fault, count: 1 });
        }
    }

    pub fn remove(&mut self, fault: &Fault) {
        let key = (
            fault.proc,
            fault.location.file.0,
            fault.location.begin.line,
            fault.location.begin.col,
            fault.kind.clone(),
        );

        if let Some(index) = self.index.get(&key)
            && let Some(entry) = self.entries.get_mut(*index)
        {
            entry.count = entry.count.saturating_sub(1);
        }
    }

    pub fn count(&self) -> usize { self.entries.iter().map(|diagnostic| diagnostic.count).sum() }
}

#[cfg(test)]
mod tests;
