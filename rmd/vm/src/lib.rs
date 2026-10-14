use core::file_path;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

pub mod bake;
mod builtins;
mod error;
pub mod eval;
pub mod heap;
mod intrinsic;
pub mod json;
mod lighting;
pub mod matrix;
pub mod profile;
mod toml_value;
pub mod ui;
pub mod value;
pub mod world;

pub use error::{Fault, FaultKey, FaultKind, StackFrame, proc_name};
pub use eval::Runtime;
pub use prelude::Intrinsic;
pub use value::{AppearanceDelta, AppearanceLighting, GenericValue};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub instruction_budget: u64,
    pub call_depth: usize,
    pub allocations: usize,
    pub text_bytes: usize,
    pub constant_depth: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            instruction_budget: 100_000,
            call_depth: 48,
            allocations: 100_000,
            text_bytes: 1_048_576,
            constant_depth: 128,
        }
    }
}

/// Workaround for icon_states() and related icon.# procs
/// TODO: think more about this
#[derive(Debug, Clone, Default)]
pub struct IconStates(Arc<HashMap<String, IconInfo>>);

#[derive(Debug, Clone)]
struct IconInfo {
    states: Vec<Arc<str>>,
    size: [u32; 2],
}

impl IconStates {
    pub fn new(icons: impl IntoIterator<Item = (String, Vec<String>, [u32; 2])>) -> Self {
        let mut table = HashMap::new();
        for (path, states, size) in icons {
            let mut seen = HashSet::new();
            let states = states
                .into_iter()
                .filter(|state| seen.insert(state.clone()))
                .map(Arc::from)
                .collect::<Vec<_>>();
            table.insert(icon_key(&path), IconInfo { states, size });
        }

        Self(Arc::new(table))
    }

    pub(crate) fn get(&self, path: &str) -> Option<&[Arc<str>]> {
        self.0.get(&icon_key(path)).map(|icon| icon.states.as_slice())
    }

    pub(crate) fn size(&self, path: &str) -> Option<[u32; 2]> { self.0.get(&icon_key(path)).map(|icon| icon.size) }
}

/// BYOND resolves resource paths case-insensitively and accepts either slash.
fn icon_key(path: &str) -> String { file_path::forward_slashes(path).to_ascii_lowercase() }

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub fault: Fault,
    pub count: usize,
}

#[derive(Debug, Default)]
pub struct Diagnostics {
    pub entries: Vec<Diagnostic>,
    index: HashMap<FaultKey, usize>,
}

impl Diagnostics {
    pub fn record(&mut self, fault: Fault) {
        let key = fault.key();

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
        let key = fault.key();

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
