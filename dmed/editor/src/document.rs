use std::{collections::HashMap, path::PathBuf};

use dmm::{Coord, Map, Prefab};

use crate::command::{Edit, History};

pub struct MapDocument {
    pub path: Option<PathBuf>,
    pub map: Map,
    pub history: History,
    pub z: u32,
    pub selection: Option<Selection>,
    selected_instance: Option<PrefabInstanceId>,
    instances: PrefabInstances,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PrefabInstanceId(u64);

impl PrefabInstanceId {
    pub const fn get(self) -> u64 { self.0 }
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

#[derive(Debug)]
pub(crate) struct PrefabInstances {
    by_coord: HashMap<Coord, Vec<PrefabInstanceId>>,
    locations: HashMap<PrefabInstanceId, PrefabLocation>,
    next_id: u64,
}

impl PrefabInstances {
    fn from_map(map: &Map) -> Self {
        let mut instances = Self {
            by_coord: HashMap::new(),
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
        let id = PrefabInstanceId(self.next_id);
        self.next_id = self.next_id.checked_add(1).expect("prefab instance ID space exhausted");

        id
    }

    pub(crate) fn ids_at(&self, coord: Coord) -> &[PrefabInstanceId] {
        self.by_coord.get(&coord).map(Vec::as_slice).unwrap_or_default()
    }

    pub(crate) fn location(&self, id: PrefabInstanceId) -> Option<PrefabLocation> { self.locations.get(&id).copied() }

    pub(crate) fn remove(&mut self, coord: Coord) {
        let Some(ids) = self.by_coord.remove(&coord) else {
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

        for (prefab_index, id) in ids.iter().copied().enumerate() {
            let replaced = self.locations.insert(id, PrefabLocation { coord, prefab_index });
            assert!(replaced.is_none(), "prefab instance ID {} is present twice", id.get());
        }

        self.by_coord.insert(coord, ids);
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

    pub fn contains(&self, coord: Coord) -> bool {
        coord.z == self.min.z
            && (self.min.x..=self.max.x).contains(&coord.x)
            && (self.min.y..=self.max.y).contains(&coord.y)
    }

    pub fn iter(&self) -> impl Iterator<Item = Coord> + '_ {
        let z = self.min.z;

        (self.min.y..=self.max.y).flat_map(move |y| (self.min.x..=self.max.x).map(move |x| Coord::new(x, y, z)))
    }
}

impl MapDocument {
    pub fn new(map: Map, z: u32) -> Self {
        let instances = PrefabInstances::from_map(&map);

        Self {
            path: None,
            map,
            history: History::new(),
            z,
            selection: None,
            selected_instance: None,
            instances,
        }
    }

    pub fn open(path: impl Into<PathBuf>, map: Map, z: u32) -> Self {
        Self {
            path: Some(path.into()),
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

    pub fn is_dirty(&self) -> bool { self.history.is_dirty() }

    pub fn instance_ids_at(&self, coord: Coord) -> &[PrefabInstanceId] { self.instances.ids_at(coord) }

    pub fn instance_location(&self, id: PrefabInstanceId) -> Option<PrefabLocation> { self.instances.location(id) }

    pub fn prefab_instance(&self, id: PrefabInstanceId) -> Option<(&Prefab, PrefabLocation)> {
        let location = self.instance_location(id)?;
        let prefab = self.map.tile_at(location.coord)?.get(location.prefab_index)?;

        Some((prefab, location))
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

    pub fn apply(&mut self, edit: Edit) {
        self.history.apply(&mut self.map, &mut self.instances, edit);
        self.clear_stale_instance_selection();
    }

    pub fn undo(&mut self) -> bool {
        let changed = self.history.undo(&mut self.map, &mut self.instances).is_some();
        self.clear_stale_instance_selection();

        changed
    }

    pub fn redo(&mut self) -> bool {
        let changed = self.history.redo(&mut self.map, &mut self.instances).is_some();
        self.clear_stale_instance_selection();

        changed
    }

    pub fn save(&mut self) -> std::io::Result<()> {
        let Some(path) = self.path.clone() else {
            return Err(std::io::Error::other("document has no path"));
        };

        self.map.prune_dictionary();
        dmm::writer::MapWriter::new(&self.map).save(path)?;
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
}

#[cfg(test)]
mod tests {
    use core::path::TreePath;
    use std::collections::HashSet;

    use dmm::{Map, Prefab, Size, writer::MapWriter};

    use super::{Coord, MapDocument};
    use crate::command::Edit;

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
}
