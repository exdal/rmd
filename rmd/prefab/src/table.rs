use std::{
    collections::HashMap,
    hash::{BuildHasher, RandomState},
};

use crate::{Prefab, PrefabId, prefabs_equal};

#[derive(Debug, Clone)]
struct Entry {
    prefab: Prefab,
    placed: u32,
    pinned: bool,
}

#[derive(Debug, Clone, Default)]
pub struct PrefabTable {
    entries: Vec<Entry>,
    index: HashMap<u64, Vec<PrefabId>>,
    hasher: RandomState,
}

impl PrefabTable {
    pub fn intern(&mut self, prefab: &Prefab) -> PrefabId {
        let Self { entries, index, hasher } = self;
        let bucket = index.entry(hasher.hash_one(prefab)).or_default();
        if let Some(id) = bucket
            .iter()
            .copied()
            .find(|id| prefabs_equal(&entries[id.0 as usize].prefab, prefab))
        {
            return id;
        }

        let id = PrefabId(u32::try_from(entries.len()).expect("prefab table holds at most u32::MAX entries"));
        entries.push(Entry {
            prefab: prefab.clone(),
            placed: 0,
            pinned: false,
        });
        bucket.push(id);

        id
    }

    pub fn find(&self, prefab: &Prefab) -> Option<PrefabId> {
        self.index
            .get(&self.hasher.hash_one(prefab))?
            .iter()
            .copied()
            .find(|id| prefabs_equal(self.get(*id), prefab))
    }

    pub fn get(&self, id: PrefabId) -> &Prefab {
        &self.entry(id).prefab
    }

    pub fn place(&mut self, id: PrefabId) {
        let entry = self.entry_mut(id);
        entry.placed = entry.placed.checked_add(1).expect("placement count overflow");
    }

    pub fn unplace(&mut self, id: PrefabId) {
        let entry = self.entry_mut(id);
        entry.placed = entry
            .placed
            .checked_sub(1)
            .expect("unplaced a prefab that was never placed");
    }

    pub fn placed(&self, id: PrefabId) -> u32 {
        self.entry(id).placed
    }

    pub fn pin(&mut self, id: PrefabId) {
        self.entry_mut(id).pinned = true;
    }

    pub fn live(&self) -> impl Iterator<Item = &Prefab> {
        self.entries
            .iter()
            .filter(|entry| entry.placed > 0 || entry.pinned)
            .map(|entry| &entry.prefab)
    }

    pub fn placements(&self) -> impl Iterator<Item = (&Prefab, u32)> {
        self.entries
            .iter()
            .filter(|entry| entry.placed > 0)
            .map(|entry| (&entry.prefab, entry.placed))
    }

    pub fn placed_entries(&self) -> usize {
        self.entries.iter().filter(|entry| entry.placed > 0).count()
    }

    fn entry(&self, id: PrefabId) -> &Entry {
        &self.entries[id.0 as usize]
    }

    fn entry_mut(&mut self, id: PrefabId) -> &mut Entry {
        &mut self.entries[id.0 as usize]
    }
}

#[cfg(test)]
mod tests;
