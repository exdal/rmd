pub mod error;
pub mod key;
pub mod merge;
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

        let key = self.free_key();
        self.dictionary.insert(key, tile);

        key
    }

    fn free_key(&mut self) -> Key {
        self.key_length = self.key_length.max(1);

        loop {
            let capacity = Key::capacity(self.key_length);
            if let Some(key) = (0..capacity).map(Key).find(|key| !self.dictionary.contains_key(key)) {
                return key;
            }

            if Key::capacity(self.key_length + 1) == capacity {
                let next = self
                    .dictionary
                    .keys()
                    .map(|key| key.0)
                    .max()
                    .map_or(0, |max| max.saturating_add(1));
                return Key(next);
            }

            self.key_length += 1;
        }
    }

    pub fn reassign_overflowing_keys(&mut self) {
        let capacity = Key::capacity(self.key_length);
        let mut overflowing: Vec<Key> = self
            .dictionary
            .keys()
            .copied()
            .filter(|key| key.0 >= capacity)
            .collect();
        if overflowing.is_empty() {
            return;
        }

        overflowing.sort();

        let mut remap = HashMap::new();
        for old in overflowing {
            let Some(tile) = self.dictionary.remove(&old) else {
                continue;
            };
            let new = self.free_key();
            self.dictionary.insert(new, tile);
            remap.insert(old, new);
        }

        for key in self.grid.iter_mut().flatten().flatten() {
            if let Some(new) = remap.get(key) {
                *key = *new;
            }
        }
    }

    /// Keeps the bottom-left corner in place and fills new cells with `fill`
    pub fn resize(&mut self, width: u32, height: u32, fill: Key) {
        let old_height = self.size.y as usize;
        let height_cells = height as usize;
        for level in &mut self.grid {
            // rows are stored top first, so the top is where rows come and go
            if height_cells >= old_height {
                let added = vec![vec![fill; self.size.x as usize]; height_cells - old_height];
                level.splice(0..0, added);
            } else {
                level.drain(0..old_height - height_cells);
            }
            for row in level.iter_mut() {
                row.resize(width as usize, fill);
            }
        }
        self.size.x = width;
        self.size.y = height;
    }

    pub fn prune_dictionary(&mut self) {
        let used: std::collections::HashSet<Key> = self.grid.iter().flatten().flatten().copied().collect();

        self.dictionary.retain(|key, _| used.contains(key));
    }

    pub fn dedupe_dictionary(&mut self) {
        let mut keys = self.dictionary.keys().copied().collect::<Vec<_>>();
        keys.sort_unstable();
        let mut remap = HashMap::new();

        {
            let mut kept = HashMap::<Vec<&TreePath>, Vec<Key>>::new();
            for key in keys {
                let tile = &self.dictionary[&key];
                let candidates = kept
                    .entry(tile.iter().map(|prefab| &prefab.path).collect())
                    .or_default();
                match candidates.iter().find(|candidate| self.dictionary[*candidate] == *tile) {
                    Some(candidate) => {
                        remap.insert(key, *candidate);
                    },
                    None => candidates.push(key),
                }
            }
        }

        if remap.is_empty() {
            return;
        }

        for key in self.grid.iter_mut().flatten().flatten() {
            if let Some(kept) = remap.get(key) {
                *key = *kept;
            }
        }

        self.prune_dictionary();
    }
}

#[cfg(test)]
mod tests {
    use core::path::TreePath;

    use crate::{Map, Prefab, Size, key::Key};

    fn tile(path: &str) -> Vec<Prefab> { vec![Prefab::new(TreePath::parse(path))] }

    #[test]
    fn resizing_keeps_the_bottom_left_corner() {
        let mut map = Map::new(Size { x: 2, y: 2, z: 2 });
        let wall = map.intern_tile(tile("/turf/wall"));
        let floor = map.intern_tile(tile("/turf/floor"));
        let space = map.intern_tile(tile("/turf/space"));
        for level in &mut map.grid {
            *level = vec![vec![floor, floor], vec![wall, floor]];
        }
        let corner = crate::Coord::new(1, 1, 2);
        assert_eq!(map.key_at(corner), Some(wall));

        map.resize(3, 4, space);
        assert_eq!((map.size.x, map.size.y, map.size.z), (3, 4, 2));
        assert_eq!(map.key_at(corner), Some(wall));
        assert_eq!(map.key_at(crate::Coord::new(2, 2, 2)), Some(floor));
        assert_eq!(map.key_at(crate::Coord::new(3, 1, 2)), Some(space));
        assert_eq!(map.key_at(crate::Coord::new(1, 4, 1)), Some(space));
        assert!(map.grid.iter().flatten().all(|row| row.len() == 3));
        assert!(map.grid.iter().all(|level| level.len() == 4));

        map.resize(1, 1, space);
        assert_eq!(map.grid, vec![vec![vec![wall]]; 2]);
    }

    #[test]
    fn dedupe_dictionary_keeps_the_lowest_key() {
        let mut map = Map::new(Size { x: 3, y: 1, z: 1 });
        map.dictionary.insert(Key(5), tile("/turf/wall"));
        map.dictionary.insert(Key(2), tile("/turf/wall"));
        map.dictionary.insert(Key(9), tile("/turf/floor"));
        map.grid[0][0] = vec![Key(5), Key(2), Key(9)];

        map.dedupe_dictionary();

        assert_eq!(map.grid[0][0], [Key(2), Key(2), Key(9)]);
        assert_eq!(map.dictionary.len(), 2);
        assert!(!map.dictionary.contains_key(&Key(5)));
    }

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
    fn intern_tile_fills_gaps() {
        let mut map = Map::new(Size { x: 1, y: 1, z: 1 });
        map.key_length = 3;
        map.dictionary.insert(Key(0), tile("/turf/wall"));
        map.dictionary.insert(Key::parse("ylZ").unwrap(), tile("/turf/floor"));

        assert_eq!(map.intern_tile(tile("/turf/space")), Key(1));
    }

    #[test]
    fn intern_tile_never_exceeds_ymo() {
        let mut map = Map::new(Size { x: 1, y: 1, z: 1 });
        map.key_length = 3;
        for raw in 0..Key::capacity(3) - 1 {
            map.dictionary.insert(Key(raw), Vec::new());
        }

        let last = map.intern_tile(tile("/turf/space"));

        assert_eq!(last, Key(Key::capacity(3) - 1));
        assert!(last <= Key::parse("ymo").unwrap());
    }

    #[test]
    fn intern_tile_widens_once_every_short_key_is_taken() {
        let mut map = Map::new(Size { x: 1, y: 1, z: 1 });
        for raw in 0..52 {
            map.dictionary.insert(Key(raw), Vec::new());
        }

        assert_eq!(map.intern_tile(tile("/turf/space")), Key(52));
        assert_eq!(map.key_length, 2);
    }

    #[test]
    fn reassign_overflowing_keys_moves_keys_into_gaps() {
        let mut map = Map::new(Size { x: 2, y: 1, z: 1 });
        map.key_length = 3;
        let overflowing = Key::parse("yog").unwrap();
        map.dictionary.insert(Key(0), tile("/turf/wall"));
        map.dictionary.insert(overflowing, tile("/turf/floor"));
        map.grid[0][0] = vec![Key(0), overflowing];

        map.reassign_overflowing_keys();

        let moved = map.grid[0][0][1];
        assert_eq!(moved, Key(1));
        assert_eq!(map.dictionary.get(&moved), Some(&tile("/turf/floor")));
        assert!(!map.dictionary.contains_key(&overflowing));
        assert_eq!(map.grid[0][0][0], Key(0));
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
