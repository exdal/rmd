pub mod error;
pub mod key;
pub mod parser;
pub mod writer;

use core::{
    path::TreePath,
    types::{Identifier, Value},
};
use std::{collections::HashMap, num::NonZeroU64};

use crate::key::Key;

/// Stable runtime identity for one prefab placement in a map document.
///
/// Placement IDs are editor state and are not serialized into DMM files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PrefabInstanceId(NonZeroU64);

impl PrefabInstanceId {
    pub const fn from_raw(raw: u64) -> Option<Self> {
        match NonZeroU64::new(raw) {
            Some(raw) => Some(Self(raw)),
            None => None,
        }
    }

    pub const fn get(self) -> u64 { self.0.get() }
}

/// DM counts `y` from the bottom
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Coord {
    pub x: u32,
    pub y: u32,
    pub z: u32,
}

impl Coord {
    pub fn new(x: u32, y: u32, z: u32) -> Self { Self { x, y, z } }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Size {
    pub x: u32,
    pub y: u32,
    pub z: u32,
}

/// `charge = 2e+005`, `damage = 1.30`, `network = list("ss13", "engine")`
#[derive(Debug, Clone, PartialEq)]
pub struct VarValue {
    pub value: Value,
    verbatim: Option<String>,
}

impl VarValue {
    pub fn new(value: Value) -> Self { Self { value, verbatim: None } }

    pub fn verbatim(&self) -> Option<&str> { self.verbatim.as_deref() }
}

impl From<Value> for VarValue {
    fn from(value: Value) -> Self { Self::new(value) }
}

/// Vars stay in file order
#[derive(Debug, Clone, PartialEq)]
pub struct Prefab {
    pub path: TreePath,
    pub vars: Vec<(Identifier, VarValue)>,
}

impl Prefab {
    pub fn new(path: TreePath) -> Self { Self { path, vars: Vec::new() } }

    pub fn var(&self, name: &Identifier) -> Option<&Value> {
        self.vars.iter().find(|(key, _)| key == name).map(|(_, var)| &var.value)
    }

    pub fn set_var(&mut self, name: Identifier, value: Value) { self.set(name, VarValue::new(value)); }

    pub fn remove_var(&mut self, name: &Identifier) -> Option<VarValue> {
        let index = self.vars.iter().position(|(key, _)| key == name)?;

        Some(self.vars.remove(index).1)
    }

    pub(crate) fn set_var_from_source(&mut self, name: Identifier, value: Value, source: &str) {
        let mut canonical = String::with_capacity(source.len());

        let verbatim = match crate::writer::write_value(&mut canonical, &value) {
            Ok(()) if canonical == source => None,
            _ => Some(source.to_string()),
        };

        self.set(name, VarValue { value, verbatim });
    }

    fn set(&mut self, name: Identifier, var: VarValue) {
        match self.vars.iter_mut().find(|(key, _)| *key == name) {
            Some(slot) => slot.1 = var,
            None => self.vars.push((name, var)),
        }
    }
}

pub type Tile = Vec<Prefab>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MapFormat {
    /// BYOND
    #[default]
    Standard,
    /// `dmm2tgm.py`
    Tgm,
}

#[derive(Debug, Clone, Default)]
pub struct Map {
    pub size: Size,
    pub key_length: usize,
    pub dictionary: HashMap<Key, Tile>,
    /// `grid[z][y][x]`
    pub grid: Vec<Vec<Vec<Key>>>,
    pub format: MapFormat,
}

impl Map {
    pub fn new(size: Size) -> Self {
        Self {
            size,
            key_length: 1,
            dictionary: HashMap::new(),
            grid: vec![vec![vec![Key::default(); size.x as usize]; size.y as usize]; size.z as usize],
            format: MapFormat::default(),
        }
    }

    pub fn key_at(&self, coord: Coord) -> Option<Key> {
        let z = self.grid.get(coord.z.checked_sub(1)? as usize)?;
        // DM counts `y` from the bottom
        let row = z.get((self.size.y.checked_sub(coord.y)?) as usize)?;

        row.get(coord.x.checked_sub(1)? as usize).copied()
    }

    pub fn tile_at(&self, coord: Coord) -> Option<&Tile> { self.dictionary.get(&self.key_at(coord)?) }

    pub fn intern_tile(&mut self, tile: Tile) -> Key {
        if let Some((key, _)) = self.dictionary.iter().find(|(_, existing)| **existing == tile) {
            return *key;
        }

        // Keys stay sparse after pruning
        let next = self
            .dictionary
            .keys()
            .map(|key| key.0)
            .max()
            .map_or(0, |max| max.saturating_add(1));
        let key = Key(next);

        self.dictionary.insert(key, tile);

        // Never narrow keys
        self.key_length = self.key_length.max(Key::length_for((key.0 as usize).saturating_add(1)));

        key
    }

    pub fn prune_dictionary(&mut self) {
        let used: std::collections::HashSet<Key> = self.grid.iter().flatten().flatten().copied().collect();

        self.dictionary.retain(|key, _| used.contains(key));
    }
}

#[cfg(test)]
mod tests {
    use core::path::TreePath;

    use crate::{Map, Prefab, Size, key::Key};

    fn tile(path: &str) -> Vec<Prefab> { vec![Prefab::new(TreePath::parse(path))] }

    #[test]
    fn intern_tile_does_not_collide_after_pruning() {
        let mut map = Map::new(Size { x: 1, y: 1, z: 1 });
        let kept = map.intern_tile(tile("/turf/wall"));
        map.intern_tile(tile("/turf/floor"));
        map.intern_tile(tile("/turf/space"));

        if let Some(slot) = map
            .grid
            .first_mut()
            .and_then(|z| z.first_mut())
            .and_then(|row| row.first_mut())
        {
            *slot = kept;
        }

        map.prune_dictionary();

        let fresh = map.intern_tile(tile("/turf/lava"));

        assert_ne!(fresh, kept);
        assert_eq!(map.dictionary.len(), 2);
        assert_eq!(map.dictionary.get(&kept), Some(&tile("/turf/wall")));
    }

    #[test]
    fn intern_tile_never_narrows_the_key_width() {
        let mut map = Map::new(Size { x: 1, y: 1, z: 1 });
        map.key_length = 3;
        map.intern_tile(tile("/turf/wall"));

        assert_eq!(map.key_length, 3);
    }

    #[test]
    fn prefab_vars_keep_insertion_order_and_replace_in_place() {
        let mut prefab = Prefab::new(TreePath::parse("/obj/t"));
        prefab.set_var("name".into(), core::types::Value::Num(1.0));
        prefab.set_var("id_tag".into(), core::types::Value::Num(2.0));
        prefab.set_var("name".into(), core::types::Value::Num(3.0));

        let names: Vec<&str> = prefab.vars.iter().map(|(name, _)| name.as_str()).collect();

        assert_eq!(names, ["name", "id_tag"]);
        assert_eq!(prefab.var(&"name".into()), Some(&core::types::Value::Num(3.0)));
    }

    #[test]
    fn removing_a_prefab_var_preserves_the_order_of_the_rest() {
        let mut prefab = Prefab::new(TreePath::parse("/obj/t"));
        prefab.set_var("first".into(), core::types::Value::Num(1.0));
        prefab.set_var("middle".into(), core::types::Value::Num(2.0));
        prefab.set_var("last".into(), core::types::Value::Num(3.0));

        assert_eq!(
            prefab.remove_var(&"middle".into()).map(|var| var.value),
            Some(core::types::Value::Num(2.0)),
        );
        assert_eq!(
            prefab.vars.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(),
            ["first", "last"],
        );
        assert_eq!(prefab.remove_var(&"missing".into()), None);
    }

    #[test]
    fn key_at_reads_the_bottom_row_as_y_one() {
        let mut map = Map::new(Size { x: 1, y: 2, z: 1 });

        if let Some(row) = map.grid.first_mut().and_then(|z| z.get_mut(1)) {
            row[0] = Key(7);
        }

        assert_eq!(map.key_at(crate::Coord::new(1, 1, 1)), Some(Key(7)));
    }
}
