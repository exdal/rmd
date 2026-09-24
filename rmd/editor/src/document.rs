use core::types::{Identifier, Value};
use std::{
    collections::{BTreeMap, HashMap},
    io::Write,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

pub use dmm::PrefabInstanceId;
use dmm::{Coord, Map, MapFormat, Prefab, key::Key};

use crate::{
    command::{Edit, EditGroupId, History},
    focus::AreaFocus,
    tool::{BlockSelectionMode, SelectionMask},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DocumentId(u64);

impl DocumentId {
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);

        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }

    pub const fn get(self) -> u64 { self.0 }
}

impl Default for DocumentId {
    fn default() -> Self { Self::new() }
}

pub struct MapDocument {
    id: DocumentId,
    pub path: Option<PathBuf>,
    pub map: Map,
    pub history: History,
    pub z: u32,
    pub selection: Option<Selection>,
    pub selection_mode: BlockSelectionMode,
    needs_initial_save: bool,
    /// Contents differ from the file on disk without any history, like a merge result
    pending_write: bool,
    selected_instance: Option<PrefabInstanceId>,
    instances: PrefabInstances,
    key_usage: HashMap<Key, usize>,
    focus: Option<AreaFocus>,
    saved_level_count: u32,
    retained_level_count: u32,
    generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrefabLocation {
    pub coord: Coord,
    pub prefab_index: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlacedPrefab {
    id: PrefabInstanceId,
    prefab: Prefab,
}

impl PlacedPrefab {
    pub fn id(&self) -> PrefabInstanceId { self.id }

    pub fn prefab(&self) -> &Prefab { &self.prefab }

    pub fn prefab_mut(&mut self) -> &mut Prefab { &mut self.prefab }
}

pub type PlacedTile = Vec<PlacedPrefab>;

#[derive(Debug, Clone, PartialEq)]
pub enum VarMutation {
    Set(Identifier, Value),
    Remove(Identifier),
}

#[derive(Debug)]
pub(crate) struct PrefabInstances {
    levels: Vec<PrefabLevel>,
    locations: HashMap<PrefabInstanceId, PrefabLocation>,
    next_id: u64,
}

#[derive(Debug, Default)]
struct PrefabLevel {
    by_position: HashMap<(u32, u32), Vec<PrefabInstanceId>>,
}

impl PrefabInstances {
    fn from_map(map: &Map) -> Self {
        let mut instances = Self {
            levels: (0..map.size.z).map(|_| PrefabLevel::default()).collect(),
            locations: HashMap::new(),
            next_id: 1,
        };

        for z in 1..=map.size.z {
            for y in 1..=map.size.y {
                for x in 1..=map.size.x {
                    let coord = Coord::new(x, y, z);
                    let count = map.tile_at(coord).map_or(0, Vec::len);
                    let ids = (0..count).map(|_| instances.allocate()).collect();

                    instances.insert(coord, ids);
                }
            }
        }

        instances
    }

    pub(crate) fn allocate(&mut self) -> PrefabInstanceId {
        let id = PrefabInstanceId::from_raw(self.next_id).expect("prefab instance IDs start at one");
        self.next_id = self.next_id.checked_add(1).expect("prefab instance ID space exhausted");

        id
    }

    pub(crate) fn ids_at(&self, coord: Coord) -> &[PrefabInstanceId] {
        coord
            .z
            .checked_sub(1)
            .and_then(|z| self.levels.get(z as usize))
            .and_then(|level| level.by_position.get(&(coord.x, coord.y)))
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub(crate) fn location(&self, id: PrefabInstanceId) -> Option<PrefabLocation> { self.locations.get(&id).copied() }

    pub(crate) fn remove(&mut self, coord: Coord) {
        let Some(ids) = coord
            .z
            .checked_sub(1)
            .and_then(|z| self.levels.get_mut(z as usize))
            .and_then(|level| level.by_position.remove(&(coord.x, coord.y)))
        else {
            return;
        };

        for id in ids {
            self.locations.remove(&id);
        }
    }

    pub(crate) fn insert(&mut self, coord: Coord, ids: Vec<PrefabInstanceId>) {
        if ids.is_empty() {
            return;
        }
        let Some(level) = coord.z.checked_sub(1).and_then(|z| self.levels.get_mut(z as usize)) else {
            return;
        };

        for (prefab_index, id) in ids.iter().copied().enumerate() {
            let replaced = self.locations.insert(id, PrefabLocation { coord, prefab_index });
            assert!(replaced.is_none(), "prefab instance ID {} is present twice", id.get());
        }

        let replaced = level.by_position.insert((coord.x, coord.y), ids);
        assert!(replaced.is_none(), "prefab instances were inserted twice at {coord:?}");
    }

    fn append_level(&mut self) { self.levels.push(PrefabLevel::default()); }

    fn truncate_levels(&mut self, level_count: u32) {
        let keep = (level_count as usize).min(self.levels.len());
        for level in self.levels.drain(keep..) {
            for ids in level.by_position.into_values() {
                for id in ids {
                    self.locations.remove(&id);
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub min: Coord,
    pub max: Coord,
}

impl Selection {
    pub fn from_drag(anchor: Coord, cursor: Coord) -> Self {
        Self {
            min: Coord::new(anchor.x.min(cursor.x), anchor.y.min(cursor.y), anchor.z),
            max: Coord::new(anchor.x.max(cursor.x), anchor.y.max(cursor.y), anchor.z),
        }
    }

    pub fn is_well_formed(&self) -> bool {
        self.min.x >= 1
            && self.min.y >= 1
            && self.min.z >= 1
            && self.max.x >= self.min.x
            && self.max.y >= self.min.y
            && self.max.z == self.min.z
    }

    pub fn contains(&self, coord: Coord) -> bool {
        coord.z == self.min.z
            && (self.min.x..=self.max.x).contains(&coord.x)
            && (self.min.y..=self.max.y).contains(&coord.y)
    }

    pub fn width(&self) -> u32 { self.max.x - self.min.x + 1 }

    pub fn height(&self) -> u32 { self.max.y - self.min.y + 1 }

    pub fn with_min(&self, min: Coord) -> Option<Self> {
        if min.z != self.min.z || self.max.z != self.min.z || self.max.x < self.min.x || self.max.y < self.min.y {
            return None;
        }

        Some(Self {
            min,
            max: Coord::new(
                min.x.checked_add(self.width().checked_sub(1)?)?,
                min.y.checked_add(self.height().checked_sub(1)?)?,
                min.z,
            ),
        })
    }

    pub fn iter(self) -> impl Iterator<Item = Coord> {
        let (min, max) = (self.min, self.max);

        (min.y..=max.y).flat_map(move |y| (min.x..=max.x).map(move |x| Coord::new(x, y, min.z)))
    }
}

impl MapDocument {
    pub fn new(map: Map, z: u32) -> Self {
        let level_count = map.size.z;
        let instances = PrefabInstances::from_map(&map);
        let mut key_usage = HashMap::new();
        for key in map.grid.iter().flatten().flatten() {
            *key_usage.entry(*key).or_insert(0) += 1;
        }

        Self {
            id: DocumentId::new(),
            path: None,
            map,
            history: History::new(),
            z,
            selection: None,
            selection_mode: BlockSelectionMode::Full,
            needs_initial_save: false,
            pending_write: false,
            selected_instance: None,
            instances,
            key_usage,
            focus: None,
            saved_level_count: level_count,
            retained_level_count: level_count,
            generation: 0,
        }
    }

    pub fn id(&self) -> DocumentId { self.id }

    pub fn generation(&self) -> u64 { self.generation }

    pub fn selection_mask(&self) -> Option<SelectionMask> {
        self.selection
            .filter(|bounds| bounds.min.z == self.z)
            .map(|bounds| SelectionMask {
                bounds,
                mode: self.selection_mode,
            })
    }

    pub fn open(path: impl Into<PathBuf>, map: Map, z: u32) -> Self {
        Self {
            path: Some(path.into()),
            ..Self::new(map, z)
        }
    }

    pub fn open_modified(path: impl Into<PathBuf>, map: Map, z: u32) -> Self {
        Self {
            path: Some(path.into()),
            pending_write: true,
            ..Self::new(map, z)
        }
    }

    pub fn replace_map(&mut self, map: Map, pending_write: bool) {
        // note: this operation drops the history

        let z = self.z.clamp(1, map.size.z.max(1));
        let replaced = Self {
            id: self.id,
            path: self.path.take(),
            pending_write,
            generation: self.generation + 1,
            ..Self::new(map, z)
        };

        *self = replaced;
    }

    pub fn create(path: impl Into<PathBuf>, map: Map, z: u32) -> Self {
        Self {
            path: Some(path.into()),
            needs_initial_save: true,
            ..Self::new(map, z)
        }
    }

    pub fn title(&self) -> String {
        let name = self
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "untitled".to_string());

        if self.is_dirty() { format!("{name} *") } else { name }
    }

    pub fn is_dirty(&self) -> bool {
        self.needs_initial_save
            || self.pending_write
            || self.history.is_dirty()
            || self.map.size.z != self.saved_level_count
    }

    pub fn needs_initial_save(&self) -> bool { self.needs_initial_save }

    pub fn instance_ids_at(&self, coord: Coord) -> &[PrefabInstanceId] { self.instances.ids_at(coord) }

    pub fn instance_location(&self, id: PrefabInstanceId) -> Option<PrefabLocation> { self.instances.location(id) }

    pub fn prefab_instance(&self, id: PrefabInstanceId) -> Option<(&Prefab, PrefabLocation)> {
        let location = self.instance_location(id)?;
        let prefab = self.map.tile_at(location.coord)?.get(location.prefab_index)?;

        Some((prefab, location))
    }

    pub fn prefab_instances(&self) -> impl Iterator<Item = (PrefabInstanceId, &Prefab, PrefabLocation)> {
        self.instances.locations.iter().filter_map(|(id, location)| {
            let prefab = self.map.tile_at(location.coord)?.get(location.prefab_index)?;

            Some((*id, prefab, *location))
        })
    }

    pub fn selected_instance(&self) -> Option<PrefabInstanceId> {
        self.selected_instance
            .filter(|id| self.instance_location(*id).is_some())
    }

    pub fn select_instance(&mut self, selected: Option<PrefabInstanceId>) {
        self.selected_instance = selected.filter(|id| self.instance_location(*id).is_some());
    }

    pub fn placed_tile(&self, coord: Coord) -> Option<PlacedTile> {
        let tile = self.map.tile_at(coord)?;
        let ids = self.instance_ids_at(coord);

        if ids.len() != tile.len() {
            return None;
        }

        Some(
            ids.iter()
                .copied()
                .zip(tile.iter().cloned())
                .map(|(id, prefab)| PlacedPrefab { id, prefab })
                .collect(),
        )
    }

    pub fn instantiate(&mut self, prefab: Prefab) -> PlacedPrefab {
        PlacedPrefab {
            id: self.instances.allocate(),
            prefab,
        }
    }

    /// Set one variable on a placed prefab as an undoable map edit.
    ///
    /// Returns `None` when the placement no longer exists and otherwise reports
    /// whether the stored prefab changed.
    pub fn set_instance_var(&mut self, id: PrefabInstanceId, name: Identifier, value: Value) -> Option<bool> {
        let label = format!("set {name}");

        self.edit_instance_vars(id, label, &[VarMutation::Set(name, value)], None)
    }

    /// Apply several variable mutations to one placement as a single map edit.
    /// Reusing a group ID replaces the final state of the previous edit in that
    /// group while retaining its original state, which makes live drags one undo.
    pub fn edit_instance_vars(
        &mut self, id: PrefabInstanceId, label: impl Into<String>, mutations: &[VarMutation],
        group: Option<EditGroupId>,
    ) -> Option<bool> {
        self.edit_instances(&[id], label, None, mutations, group)
    }

    /// this function keeps the ID stable
    pub fn replace_instance_path(
        &mut self, id: PrefabInstanceId, label: impl Into<String>, path: core::path::TreePath,
        mutations: &[VarMutation], group: Option<EditGroupId>,
    ) -> Option<bool> {
        self.edit_instances(&[id], label, Some(&path), mutations, group)
    }

    /// Changes every placement in `ids` the same way, as one map edit
    pub fn edit_instances(
        &mut self, ids: &[PrefabInstanceId], label: impl Into<String>, path: Option<&core::path::TreePath>,
        mutations: &[VarMutation], group: Option<EditGroupId>,
    ) -> Option<bool> {
        let mut by_tile = BTreeMap::<Coord, Vec<usize>>::new();
        for id in ids {
            let location = self.instance_location(*id)?;
            by_tile.entry(location.coord).or_default().push(location.prefab_index);
        }

        let mut edit = Edit::new(label);
        for (coord, indices) in by_tile {
            let mut after = self.placed_tile(coord)?;
            let mut changed = false;
            for index in indices {
                let instance = after.get_mut(index)?;
                let before = instance.prefab().clone();
                if let Some(path) = path {
                    instance.prefab_mut().path = path.clone();
                }
                for mutation in mutations {
                    match mutation {
                        VarMutation::Set(name, value) => instance.prefab_mut().set_var(name.clone(), value.clone()),
                        VarMutation::Remove(name) => {
                            instance.prefab_mut().remove_var(name);
                        },
                    }
                }
                changed |= instance.prefab() != &before;
            }
            if changed {
                edit.change(self, coord, after);
            }
        }
        if edit.is_empty() {
            return Some(false);
        }

        Some(self.apply_grouped(edit, group))
    }

    /// Every placement of exactly `prefab`, on every level
    pub fn identical_instances(&self, prefab: &Prefab) -> Vec<PrefabInstanceId> {
        let matching = self
            .key_usage
            .keys()
            .filter_map(|key| {
                let indices = self
                    .map
                    .dictionary
                    .get(key)?
                    .iter()
                    .enumerate()
                    .filter(|(_, placed)| *placed == prefab)
                    .map(|(index, _)| index)
                    .collect::<Vec<_>>();

                (!indices.is_empty()).then_some((*key, indices))
            })
            .collect::<HashMap<_, _>>();
        let mut found = Vec::new();

        if matching.is_empty() {
            return found;
        }

        for (z, level) in self.map.grid.iter().enumerate() {
            for (row, keys) in level.iter().enumerate() {
                for (column, key) in keys.iter().enumerate() {
                    let Some(indices) = matching.get(key) else {
                        continue;
                    };
                    let coord = Coord::new(column as u32 + 1, self.map.size.y - row as u32, z as u32 + 1);
                    let ids = self.instances.ids_at(coord);
                    found.extend(indices.iter().filter_map(|index| ids.get(*index).copied()));
                }
            }
        }

        found
    }

    pub fn move_instance(
        &mut self, id: PrefabInstanceId, to_coord: Coord, label: impl Into<String>, mutations: &[VarMutation],
        group: Option<EditGroupId>,
    ) -> Option<bool> {
        let location = self.instance_location(id)?;
        let size = &self.map.size;
        let in_bounds = to_coord.z == location.coord.z
            && (1..=size.x.max(1)).contains(&to_coord.x)
            && (1..=size.y.max(1)).contains(&to_coord.y);
        if to_coord == location.coord || !in_bounds {
            return self.edit_instance_vars(id, label, mutations, group);
        }

        let mut source = self.placed_tile(location.coord)?;
        if location.prefab_index >= source.len() {
            return None;
        }
        let mut moved = source.remove(location.prefab_index);
        for mutation in mutations {
            match mutation {
                VarMutation::Set(name, value) => moved.prefab_mut().set_var(name.clone(), value.clone()),
                VarMutation::Remove(name) => {
                    moved.prefab_mut().remove_var(name);
                },
            }
        }
        let mut destination = self.placed_tile(to_coord).unwrap_or_default();
        destination.push(moved);

        let mut edit = Edit::new(label);
        edit.change(self, location.coord, source);
        edit.change(self, to_coord, destination);

        Some(self.apply_grouped(edit, group))
    }

    pub fn set_focus(&mut self, focus: Option<AreaFocus>) { self.focus = focus; }

    pub fn focus(&self) -> Option<&AreaFocus> { self.focus.as_ref() }

    pub fn allows_edit_at(&self, coord: Coord) -> bool { self.focus.as_ref().is_none_or(|focus| focus.allows(coord)) }

    pub fn apply(&mut self, edit: Edit) -> bool { self.apply_grouped(edit, None) }

    pub fn apply_grouped(&mut self, edit: Edit, group: Option<EditGroupId>) -> bool {
        if self.focus.as_ref().is_some_and(|focus| !focus.allows_edit(&edit)) {
            return false;
        }

        if let Some(z) = edit.changes.iter().map(|change| change.coord.z).max() {
            self.retained_level_count = self.retained_level_count.max(z);
        }

        self.history
            .apply_grouped(&mut self.map, &mut self.instances, &mut self.key_usage, edit, group);
        self.generation += 1;
        self.clear_stale_instance_selection();

        true
    }

    pub fn undo(&mut self) -> bool { self.undo_with_affected().is_some() }

    pub fn undo_with_affected(&mut self) -> Option<Vec<PrefabInstanceId>> {
        let affected = self
            .history
            .undo(&mut self.map, &mut self.instances, &mut self.key_usage)
            .map(Edit::affected_instances);
        self.generation += 1;
        self.clear_stale_instance_selection();

        affected
    }

    pub fn redo(&mut self) -> bool { self.redo_with_affected().is_some() }

    pub fn redo_with_affected(&mut self) -> Option<Vec<PrefabInstanceId>> {
        let affected = self
            .history
            .redo(&mut self.map, &mut self.instances, &mut self.key_usage)
            .map(Edit::affected_instances);
        self.generation += 1;
        self.clear_stale_instance_selection();

        affected
    }

    pub fn undo_label(&self) -> Option<&str> { self.history.undo_label() }

    pub fn redo_label(&self) -> Option<&str> { self.history.redo_label() }

    pub fn append_level(&mut self, tile: &[Prefab]) -> Option<u32> {
        let z = self.map.size.z.checked_add(1)?;
        let key = self.map.intern_tile(tile.to_vec());
        let width = self.map.size.x as usize;
        let height = self.map.size.y as usize;

        self.map.grid.push(vec![vec![key; width]; height]);
        self.map.size.z = z;
        self.generation += 1;
        *self.key_usage.entry(key).or_insert(0) += width.saturating_mul(height);
        self.instances.append_level();

        for y in 1..=self.map.size.y {
            for x in 1..=self.map.size.x {
                let ids = tile.iter().map(|_| self.instances.allocate()).collect();
                self.instances.insert(Coord::new(x, y, z), ids);
            }
        }

        Some(z)
    }

    pub fn save(&mut self) -> std::io::Result<()> {
        let Some(path) = self.path.clone() else {
            return Err(std::io::Error::other("document has no path"));
        };
        let format = self.map.format;

        self.save_as(path, format)
    }

    pub fn save_as(&mut self, path: impl Into<PathBuf>, format: MapFormat) -> std::io::Result<()> {
        let path = path.into();
        let retained_level_count = self.retained_level_count.min(self.map.size.z);
        let mut saved_map = self.map.clone();
        saved_map.grid.truncate(retained_level_count as usize);
        saved_map.size.z = retained_level_count;
        saved_map.prune_dictionary();
        saved_map.reassign_overflowing_keys();
        let contents = dmm::writer::MapWriter::new(&saved_map).with_format(format).write();

        if self.needs_initial_save {
            let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path)?;
            file.write_all(contents.as_bytes())?;
        } else {
            std::fs::write(&path, contents)?;
        }

        if retained_level_count < self.map.size.z {
            self.truncate_levels(retained_level_count);
        }

        self.map.prune_dictionary();
        self.map.format = format;
        self.path = Some(path);
        self.needs_initial_save = false;
        self.pending_write = false;
        self.saved_level_count = self.map.size.z;
        self.retained_level_count = self.map.size.z;
        self.history.mark_saved();

        Ok(())
    }

    pub fn clamp(&self, x: u32, y: u32) -> Coord {
        Coord::new(
            x.clamp(1, self.map.size.x.max(1)),
            y.clamp(1, self.map.size.y.max(1)),
            self.z,
        )
    }

    fn clear_stale_instance_selection(&mut self) {
        if self.selected_instance().is_none() {
            self.selected_instance = None;
        }
    }

    fn truncate_levels(&mut self, level_count: u32) {
        self.instances.truncate_levels(level_count);
        self.map.grid.truncate(level_count as usize);
        self.map.size.z = level_count;
        self.generation += 1;
        if self.z > level_count {
            self.z = level_count.max(1);
            self.selection = None;
            self.focus = None;
        }
        self.clear_stale_instance_selection();

        self.key_usage.clear();
        for key in self.map.grid.iter().flatten().flatten() {
            *self.key_usage.entry(*key).or_insert(0) += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use core::{path::TreePath, types::Value};
    use std::collections::HashSet;

    use dmm::{Map, Prefab, Size, writer::MapWriter};

    use super::{Coord, MapDocument, VarMutation};
    use crate::{
        command::{Edit, EditGroupId},
        focus::AreaFocus,
    };

    fn shared_tile_map() -> Map {
        let mut map = Map::new(Size { x: 2, y: 1, z: 2 });
        let key = map.intern_tile(vec![
            Prefab::new(TreePath::parse("/turf/floor")),
            Prefab::new(TreePath::parse("/obj/table")),
        ]);

        for level in &mut map.grid {
            for cell in &mut level[0] {
                *cell = key;
            }
        }

        map
    }

    #[test]
    fn shared_dictionary_prefabs_receive_unique_placement_ids() {
        let document = MapDocument::new(shared_tile_map(), 1);
        let mut ids = Vec::new();

        for z in 1..=2 {
            for x in 1..=2 {
                ids.extend_from_slice(document.instance_ids_at(Coord::new(x, 1, z)));
            }
        }

        assert_eq!(ids.len(), 8);
        assert_eq!(ids.iter().copied().collect::<HashSet<_>>().len(), ids.len());
    }

    #[test]
    fn identical_instances_are_found_on_every_level_and_edited_as_one_undo() {
        let mut document = MapDocument::new(shared_tile_map(), 1);
        let table = Prefab::new(TreePath::parse("/obj/table"));
        let tables = [(1, 1), (2, 1), (1, 2), (2, 2)]
            .map(|(x, z)| document.instance_ids_at(Coord::new(x, 1, z))[1])
            .into_iter()
            .collect::<HashSet<_>>();

        let found = document.identical_instances(&table);
        assert_eq!(found.iter().copied().collect::<HashSet<_>>(), tables);
        assert_eq!(document.identical_instances(&table).len(), 4);

        let name = VarMutation::Set("name".into(), Value::Text(String::from("Oak")));
        assert_eq!(
            document.edit_instances(&found, "set name", None, &[name], None),
            Some(true)
        );
        let mut oak = table.clone();
        oak.set_var("name".into(), Value::Text(String::from("Oak")));
        assert!(document.identical_instances(&table).is_empty());
        assert_eq!(
            document.identical_instances(&oak).into_iter().collect::<HashSet<_>>(),
            tables
        );

        assert!(document.undo());
        assert_eq!(document.identical_instances(&table).len(), 4);
        assert!(!document.undo(), "the whole edit was one step");
    }

    #[test]
    fn identical_instances_include_repeats_within_one_tile() {
        let table = Prefab::new(TreePath::parse("/obj/table"));
        let mut map = Map::new(Size { x: 1, y: 2, z: 1 });
        let doubled = map.intern_tile(vec![table.clone(), table.clone()]);
        let floor = map.intern_tile(vec![Prefab::new(TreePath::parse("/turf/floor"))]);
        map.grid[0][0][0] = doubled;
        map.grid[0][1][0] = floor;
        let document = MapDocument::new(map, 1);

        assert_eq!(
            document.identical_instances(&table),
            document.instance_ids_at(Coord::new(1, 2, 1))
        );
    }

    #[test]
    fn placement_ids_are_not_serialized() {
        let map = shared_tile_map();
        let before = MapWriter::new(&map).write();
        let document = MapDocument::new(map, 1);

        assert_eq!(MapWriter::new(&document.map).write(), before);
    }

    #[test]
    fn deleted_instance_is_removed_from_selection() {
        let mut document = MapDocument::new(shared_tile_map(), 1);
        let coord = Coord::new(1, 1, 1);
        let selected = document.instance_ids_at(coord)[1];
        document.select_instance(Some(selected));
        assert_eq!(document.selected_instance(), Some(selected));

        let mut after = document.placed_tile(coord).unwrap();
        after.remove(1);
        let mut edit = Edit::new("delete selected prefab");
        edit.change(&document, coord, after);
        document.apply(edit);

        assert_eq!(document.selected_instance(), None);
    }

    #[test]
    fn setting_an_instance_var_changes_only_that_placement_and_is_undoable() {
        let mut document = MapDocument::new(shared_tile_map(), 1);
        let changed = Coord::new(1, 1, 1);
        let untouched = Coord::new(2, 1, 1);
        let id = document.instance_ids_at(changed)[1];
        let untouched_id = document.instance_ids_at(untouched)[1];
        document.select_instance(Some(id));

        assert_eq!(
            document.set_instance_var(id, "name".into(), core::types::Value::Text("selected".into())),
            Some(true),
        );
        assert_eq!(document.selected_instance(), Some(id));
        assert_eq!(document.instance_ids_at(changed)[1], id);
        assert_eq!(document.instance_ids_at(untouched)[1], untouched_id);
        assert_eq!(
            document
                .prefab_instance(id)
                .and_then(|(prefab, _)| prefab.var(&"name".into())),
            Some(&core::types::Value::Text("selected".into())),
        );
        assert_eq!(
            document
                .prefab_instance(untouched_id)
                .and_then(|(prefab, _)| prefab.var(&"name".into())),
            None,
        );
        assert!(document.is_dirty());
        assert!(
            MapWriter::new(&document.map)
                .write()
                .contains("/obj/table{name = \"selected\"}")
        );

        assert_eq!(
            document.set_instance_var(id, "name".into(), core::types::Value::Text("selected".into())),
            Some(false),
        );

        assert!(document.undo());
        assert_eq!(
            document
                .prefab_instance(id)
                .and_then(|(prefab, _)| prefab.var(&"name".into())),
            None
        );
        assert_eq!(document.selected_instance(), Some(id));

        assert!(document.redo());
        assert_eq!(
            document
                .prefab_instance(id)
                .and_then(|(prefab, _)| prefab.var(&"name".into())),
            Some(&core::types::Value::Text("selected".into())),
        );
    }

    #[test]
    fn a_live_variable_edit_group_is_one_undo_step() {
        let mut document = MapDocument::new(shared_tile_map(), 1);
        let id = document.instance_ids_at(Coord::new(1, 1, 1))[1];
        let group = EditGroupId::new();

        for value in 1..=1_000 {
            assert_eq!(
                document.edit_instance_vars(
                    id,
                    "change pixel offset",
                    &[VarMutation::Set(
                        "pixel_x".into(),
                        core::types::Value::Num(value as f32),
                    )],
                    Some(group),
                ),
                Some(true),
            );
        }

        // Intermediate drag values must not accumulate unreachable map keys.
        assert_eq!(document.map.dictionary.len(), 2);

        assert_eq!(
            document
                .prefab_instance(id)
                .and_then(|(prefab, _)| prefab.var(&"pixel_x".into())),
            Some(&core::types::Value::Num(1_000.0)),
        );
        assert!(document.undo());
        assert_eq!(
            document
                .prefab_instance(id)
                .and_then(|(prefab, _)| prefab.var(&"pixel_x".into())),
            None,
        );
        assert!(!document.undo());
        assert!(document.redo());
        assert_eq!(
            document
                .prefab_instance(id)
                .and_then(|(prefab, _)| prefab.var(&"pixel_x".into())),
            Some(&core::types::Value::Num(1_000.0)),
        );
    }

    #[test]
    fn moving_an_instance_applies_mutations_and_undo_restores_both() {
        let mut document = MapDocument::new(shared_tile_map(), 1);
        let source = Coord::new(1, 1, 1);
        let destination = Coord::new(2, 1, 1);
        let id = document.instance_ids_at(source)[1];
        let source_before = document.placed_tile(source).unwrap();
        let destination_before = document.placed_tile(destination).unwrap();

        assert_eq!(
            document.move_instance(
                id,
                destination,
                "re-anchor table",
                &[VarMutation::Set("pixel_x".into(), core::types::Value::Num(-32.0))],
                None,
            ),
            Some(true),
        );
        assert_eq!(document.instance_location(id).unwrap().coord, destination);
        assert_eq!(
            document.prefab_instance(id).unwrap().0.var(&"pixel_x".into()),
            Some(&core::types::Value::Num(-32.0)),
        );
        assert_eq!(
            document.placed_tile(source).unwrap(),
            source_before
                .iter()
                .filter(|placed| placed.id() != id)
                .cloned()
                .collect::<Vec<_>>(),
        );
        assert_eq!(
            document.placed_tile(destination).unwrap().len(),
            destination_before.len() + 1
        );

        assert!(document.undo());
        assert_eq!(document.instance_location(id).unwrap().coord, source);
        assert_eq!(document.instance_location(id).unwrap().prefab_index, 1);
        assert_eq!(document.prefab_instance(id).unwrap().0.var(&"pixel_x".into()), None);
        assert_eq!(document.placed_tile(source).unwrap(), source_before);
        assert_eq!(document.placed_tile(destination).unwrap(), destination_before);
    }

    #[test]
    fn moving_onto_the_same_or_out_of_bounds_tile_only_edits_variables() {
        let mut document = MapDocument::new(shared_tile_map(), 1);
        let source = Coord::new(1, 1, 1);
        let id = document.instance_ids_at(source)[1];

        assert_eq!(
            document.move_instance(
                id,
                source,
                "change pixel offset",
                &[VarMutation::Set("pixel_x".into(), core::types::Value::Num(4.0))],
                None,
            ),
            Some(true),
        );
        assert_eq!(
            document.move_instance(id, Coord::new(9, 9, 1), "off the map", &[], None),
            Some(false)
        );
        assert_eq!(document.instance_location(id).unwrap().coord, source);
        assert_eq!(
            document.prefab_instance(id).unwrap().0.var(&"pixel_x".into()),
            Some(&core::types::Value::Num(4.0)),
        );
    }

    #[test]
    fn replacing_an_instance_path_preserves_its_id_variables_and_undo_history() {
        let mut document = MapDocument::new(shared_tile_map(), 1);
        let coord = Coord::new(1, 1, 1);
        let id = document.instance_ids_at(coord)[1];
        document
            .set_instance_var(id, "name".into(), core::types::Value::Text("custom".into()))
            .unwrap();
        document
            .set_instance_var(id, "dir".into(), core::types::Value::Num(8.0))
            .unwrap();
        let replacement = TreePath::parse("/obj/table/directional/north");

        assert_eq!(
            document.replace_instance_path(
                id,
                "set directional type",
                replacement.clone(),
                &[VarMutation::Remove("dir".into())],
                None,
            ),
            Some(true)
        );
        let (prefab, location) = document.prefab_instance(id).unwrap();
        assert_eq!(location.coord, coord);
        assert_eq!(prefab.path, replacement);
        assert_eq!(
            prefab.var(&"name".into()),
            Some(&core::types::Value::Text("custom".into()))
        );
        assert_eq!(prefab.var(&"dir".into()), None);

        assert!(document.undo());
        let prefab = document.prefab_instance(id).unwrap().0;
        assert_eq!(prefab.path, TreePath::parse("/obj/table"));
        assert_eq!(
            prefab.var(&"name".into()),
            Some(&core::types::Value::Text("custom".into()))
        );
        assert_eq!(prefab.var(&"dir".into()), Some(&core::types::Value::Num(8.0)));
        assert!(document.redo());
        assert_eq!(document.prefab_instance(id).unwrap().0.path, replacement);
    }

    #[test]
    fn grouped_instance_path_replacements_are_one_undo_step() {
        let mut document = MapDocument::new(shared_tile_map(), 1);
        let id = document.instance_ids_at(Coord::new(1, 1, 1))[1];
        let group = EditGroupId::new();

        assert_eq!(
            document.replace_instance_path(
                id,
                "set direction",
                TreePath::parse("/obj/table/directional/north"),
                &[],
                Some(group),
            ),
            Some(true)
        );
        assert_eq!(
            document.replace_instance_path(
                id,
                "set direction",
                TreePath::parse("/obj/table/directional/east"),
                &[],
                Some(group),
            ),
            Some(true)
        );

        assert!(document.undo());
        assert_eq!(
            document.prefab_instance(id).unwrap().0.path,
            TreePath::parse("/obj/table")
        );
        assert!(!document.undo());
    }

    #[test]
    fn grouped_instance_path_replacements_collapse_when_returned_to_the_start() {
        let mut document = MapDocument::new(shared_tile_map(), 1);
        let id = document.instance_ids_at(Coord::new(1, 1, 1))[1];
        let group = EditGroupId::new();

        document
            .replace_instance_path(
                id,
                "set direction",
                TreePath::parse("/obj/table/directional/north"),
                &[],
                Some(group),
            )
            .unwrap();
        document
            .replace_instance_path(id, "set direction", TreePath::parse("/obj/table"), &[], Some(group))
            .unwrap();

        assert!(!document.history.can_undo());
    }

    #[test]
    fn variable_mutations_can_set_and_reset_a_pair_atomically() {
        let mut document = MapDocument::new(shared_tile_map(), 1);
        let id = document.instance_ids_at(Coord::new(1, 1, 1))[1];

        assert_eq!(
            document.edit_instance_vars(
                id,
                "set pixel offset",
                &[
                    VarMutation::Set("pixel_x".into(), core::types::Value::Num(4.0)),
                    VarMutation::Set("pixel_y".into(), core::types::Value::Num(-2.0)),
                ],
                None,
            ),
            Some(true),
        );
        assert_eq!(
            document.edit_instance_vars(
                id,
                "reset pixel offset",
                &[
                    VarMutation::Remove("pixel_x".into()),
                    VarMutation::Remove("pixel_y".into()),
                ],
                None,
            ),
            Some(true),
        );

        let prefab = document.prefab_instance(id).unwrap().0;
        assert_eq!(prefab.var(&"pixel_x".into()), None);
        assert_eq!(prefab.var(&"pixel_y".into()), None);
    }
    #[test]
    fn a_focused_area_confines_every_edit_without_blocking_undo() {
        let mut map = Map::new(Size { x: 3, y: 1, z: 1 });
        let key = map.intern_tile(vec![
            Prefab::new(TreePath::parse("/turf/floor")),
            Prefab::new(TreePath::parse("/obj/table")),
        ]);
        map.grid[0][0] = vec![key, key, key];
        let mut document = MapDocument::new(map, 1);
        let inside = Coord::new(1, 1, 1);
        let outside = Coord::new(3, 1, 1);
        let table = document.instance_ids_at(outside)[1];

        // an edit made before the focus was taken has to stay reversible
        assert_eq!(
            document.set_instance_var(table, "name".into(), Value::Text("early".into())),
            Some(true),
        );

        document.set_focus(Some(AreaFocus::new(
            inside,
            Prefab::new(TreePath::parse("/area/station")),
            document.instance_ids_at(inside)[0],
            [inside, Coord::new(2, 1, 1)].into_iter().collect(),
        )));
        assert!(document.allows_edit_at(inside));
        assert!(!document.allows_edit_at(outside));

        let outside_before = document.placed_tile(outside);
        let mut refused = Edit::new("delete outside the region");
        refused.change(&document, outside, Vec::new());
        assert!(!document.apply(refused));
        assert_eq!(document.placed_tile(outside), outside_before);
        assert_eq!(
            document.set_instance_var(table, "name".into(), Value::Text("late".into())),
            Some(false),
        );

        let mut allowed = Edit::new("delete inside the region");
        allowed.change(&document, inside, Vec::new());
        assert!(document.apply(allowed));
        assert_eq!(document.placed_tile(inside), Some(Vec::new()));

        // a move that leaves the region would otherwise clear its source and drop the instance
        let moved = document.instance_ids_at(Coord::new(2, 1, 1))[1];
        assert_eq!(
            document.move_instance(moved, outside, "move out", &[], None),
            Some(false)
        );
        assert_eq!(
            document.instance_location(moved).map(|at| at.coord),
            Some(Coord::new(2, 1, 1))
        );

        assert!(document.undo());
        assert!(document.undo());
        assert_eq!(
            document
                .prefab_instance(table)
                .map(|(prefab, _)| prefab.var(&"name".into())),
            Some(None),
        );
    }

    #[test]
    fn save_as_writes_the_chosen_format_and_retargets_the_document() {
        let dir = std::env::temp_dir().join(format!("rmd-save-as-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let original = dir.join("original.dmm");
        let exported = dir.join("exported.dmm");

        let mut document = MapDocument::open(&original, shared_tile_map(), 1);
        let coord = Coord::new(1, 1, 1);
        let mut edit = Edit::new("clear a tile");
        edit.change(&document, coord, Vec::new());
        assert!(document.apply(edit));
        assert!(document.is_dirty());

        document.save_as(&exported, dmm::MapFormat::Tgm).expect("save as tgm");

        let written = std::fs::read_to_string(&exported).expect("written map");
        assert!(written.starts_with("//MAP CONVERTED BY dmm2tgm.py"), "{written}");
        assert_eq!(document.path.as_deref(), Some(exported.as_path()));
        assert_eq!(document.map.format, dmm::MapFormat::Tgm);
        assert!(!document.is_dirty());
        assert!(!original.exists(), "the original path must not be written to");

        // the format sticks, so a plain save keeps writing TGM
        std::fs::remove_file(&exported).expect("remove export");
        document.save().expect("save");
        assert!(
            std::fs::read_to_string(&exported)
                .expect("written map")
                .starts_with("//MAP CONVERTED BY dmm2tgm.py")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_removes_untouched_appended_levels() {
        let dir = std::env::temp_dir().join(format!("rmd-prune-levels-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let target = dir.join("map.dmm");
        let mut document = MapDocument::new(shared_tile_map(), 2);
        let retained_ids = document.instance_ids_at(Coord::new(1, 1, 2)).to_vec();
        let fill = [Prefab::new(TreePath::parse("/turf"))];

        assert_eq!(document.append_level(&fill), Some(3));
        document.z = 3;
        assert_eq!(document.map.size.z, 3);
        assert!(document.is_dirty());
        assert_eq!(document.instance_ids_at(Coord::new(1, 1, 3)).len(), 1);

        document.save_as(&target, dmm::MapFormat::Standard).expect("save");

        let (written, errors) = dmm::parser::parse(&std::fs::read_to_string(&target).expect("written map"));
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(written.size.z, 2);
        assert_eq!(document.map.size.z, 2);
        assert_eq!(document.z, 2);
        assert_eq!(document.instance_ids_at(Coord::new(1, 1, 2)), retained_ids);
        assert!(document.instance_ids_at(Coord::new(1, 1, 3)).is_empty());
        assert!(!document.is_dirty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_retains_the_highest_appended_level_that_received_an_edit() {
        let dir = std::env::temp_dir().join(format!("rmd-retain-levels-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let target = dir.join("map.dmm");
        let mut document = MapDocument::new(shared_tile_map(), 2);
        let fill = [Prefab::new(TreePath::parse("/turf"))];
        assert_eq!(document.append_level(&fill), Some(3));
        assert_eq!(document.append_level(&fill), Some(4));

        let coord = Coord::new(1, 1, 3);
        let mut after = document.placed_tile(coord).expect("appended tile");
        after.push(document.instantiate(Prefab::new(TreePath::parse("/obj/marker"))));
        let mut edit = Edit::new("touch appended level");
        edit.change(&document, coord, after);
        assert!(document.apply(edit));
        assert!(document.undo(), "undo still leaves the level marked as touched");
        document.z = 4;

        document.save_as(&target, dmm::MapFormat::Standard).expect("save");

        let (written, errors) = dmm::parser::parse(&std::fs::read_to_string(&target).expect("written map"));
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(written.size.z, 3);
        assert_eq!(document.map.size.z, 3);
        assert_eq!(document.z, 3);
        assert!(!document.is_dirty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn failed_save_does_not_remove_untouched_appended_levels() {
        let missing = std::env::temp_dir().join(format!("rmd-missing-save-directory-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&missing);
        let target = missing.join("map.dmm");
        let mut document = MapDocument::new(shared_tile_map(), 2);
        let fill = [Prefab::new(TreePath::parse("/turf"))];
        assert_eq!(document.append_level(&fill), Some(3));
        document.z = 3;

        document
            .save_as(&target, dmm::MapFormat::Standard)
            .expect_err("the parent directory does not exist");

        assert_eq!(document.map.size.z, 3);
        assert_eq!(document.z, 3);
        assert_eq!(document.instance_ids_at(Coord::new(1, 1, 3)).len(), 1);
        assert!(document.is_dirty());
    }

    #[test]
    fn a_created_document_is_dirty_and_will_not_replace_its_first_target() {
        let dir = std::env::temp_dir().join(format!("rmd-created-document-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let target = dir.join("new-map.dmm");
        let _ = std::fs::remove_file(&target);
        let mut document = MapDocument::create(&target, shared_tile_map(), 1);

        assert_eq!(document.path.as_deref(), Some(target.as_path()));
        assert!(document.is_dirty());

        std::fs::write(&target, "existing map").expect("existing target");
        let error = document.save().expect_err("the first save must not overwrite");
        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "existing map");
        assert!(document.is_dirty());

        std::fs::remove_file(&target).expect("remove collision");
        document.save().expect("first save");
        assert!(!document.is_dirty());

        std::fs::write(&target, "replace me").expect("replace target contents");
        document.save().expect("subsequent save");
        assert_ne!(std::fs::read_to_string(&target).unwrap(), "replace me");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
