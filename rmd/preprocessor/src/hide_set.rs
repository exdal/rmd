use rustc_hash::FxHashMap;

use crate::define::NameId;

/// Index into [`HideSets`], `0` being the empty set.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub(super) struct HideSet(u32);

struct HiddenMacro {
    name: NameId,
    previous: HideSet,
}

#[derive(Default)]
pub(super) struct HideSets {
    nodes: Vec<HiddenMacro>,
    extended: FxHashMap<(HideSet, NameId), HideSet>,
}

impl HideSets {
    fn node(&self, set: HideSet) -> Option<&HiddenMacro> {
        set.0.checked_sub(1).map(|index| &self.nodes[index as usize])
    }

    pub(super) fn contains(&self, set: HideSet, name: NameId) -> bool {
        let mut current = self.node(set);
        while let Some(hidden) = current {
            if hidden.name == name {
                return true;
            }

            current = self.node(hidden.previous);
        }

        false
    }

    pub(super) fn with(&mut self, set: HideSet, name: NameId) -> HideSet {
        if let Some(extended) = self.extended.get(&(set, name)) {
            return *extended;
        }

        let extended = if self.contains(set, name) {
            set
        } else {
            self.nodes.push(HiddenMacro { name, previous: set });
            HideSet(self.nodes.len() as u32)
        };
        self.extended.insert((set, name), extended);

        extended
    }

    pub(super) fn union(&mut self, set: HideSet, other: HideSet) -> HideSet {
        let mut merged = set;
        let mut current = other;
        while let Some(hidden) = self.node(current) {
            let (name, previous) = (hidden.name, hidden.previous);
            merged = self.with(merged, name);
            current = previous;
        }

        merged
    }
}
