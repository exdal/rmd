use base::path::TreePath;
use std::{
    collections::{HashMap, HashSet},
    mem,
    sync::Arc,
};

use dmm::{Coord, Map, MapFormat, Prefab, PrefabInstanceId, Size, Tile, key::Key, tiles_equal};

use crate::document::{instance::PlacedPrefab, instance::PrefabInstance, instance::PrefabLocation};

use table::{PrefabId, PrefabTable};
pub use view::TileView;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct SharedTile(*const [PrefabId]);

#[derive(Debug, Clone)]
struct Cell {
    prefabs: Arc<[PrefabId]>,
    ids: Vec<PrefabInstanceId>,
}

#[derive(Debug, Clone)]
struct LevelCell {
    tile: Arc<Tile>,
    ids: Vec<PrefabInstanceId>,
}

#[derive(Debug, Clone)]
pub struct LevelCells {
    cells: Vec<LevelCell>,
}

impl LevelCells {
    pub fn ids(&self) -> impl Iterator<Item = PrefabInstanceId> + '_ {
        self.cells.iter().flat_map(|cell| cell.ids.iter().copied())
    }

    pub fn tiles(&self) -> impl Iterator<Item = &Tile> + '_ {
        self.cells.iter().map(|cell| cell.tile.as_ref())
    }

    // the one tile every cell shares, like a level from `filled_level`
    pub fn fill(&self) -> Option<&Tile> {
        let (first, rest) = self.cells.split_first()?;

        rest.iter()
            .all(|cell| Arc::ptr_eq(&cell.tile, &first.tile))
            .then_some(&first.tile)
    }
}

/// `cells` run x, then y from the bottom, then z
#[derive(Debug, Clone)]
pub struct Grid {
    size: Size,
    cells: Vec<Cell>,
    locations: HashMap<PrefabInstanceId, PrefabLocation>,
    next_id: u64,
    table: PrefabTable,
}

// ids from two tables aren't comparable, so cells compare by contents
impl PartialEq for Grid {
    fn eq(&self, other: &Self) -> bool {
        self.size == other.size
            && self.cells.len() == other.cells.len()
            && self
                .cells
                .iter()
                .zip(&other.cells)
                .all(|(left, right)| left.ids == right.ids && self.view(left) == other.view(right))
    }
}

impl Grid {
    pub fn from_map(map: &Map) -> Self {
        let mut grid = Self {
            size: map.size,
            cells: Vec::with_capacity(cell_count(map.size)),
            locations: HashMap::new(),
            next_id: 1,
            table: PrefabTable::default(),
        };
        let shared = map
            .dictionary
            .iter()
            .map(|(key, tile)| (*key, grid.intern(tile)))
            .collect::<HashMap<_, _>>();
        let empty = Arc::<[PrefabId]>::from([]);

        for z in 1..=map.size.z {
            for y in 1..=map.size.y {
                for x in 1..=map.size.x {
                    let coord = Coord::new(x, y, z);
                    let prefabs = map.key_at(coord).and_then(|key| shared.get(&key)).unwrap_or(&empty);
                    let ids = (0..prefabs.len()).map(|_| grid.allocate()).collect::<Vec<_>>();
                    for (prefab_index, id) in ids.iter().enumerate() {
                        grid.locations.insert(*id, PrefabLocation { coord, prefab_index });
                    }

                    grid.place_all(prefabs);
                    grid.cells.push(Cell {
                        prefabs: Arc::clone(prefabs),
                        ids,
                    });
                }
            }
        }

        grid
    }

    pub fn size(&self) -> Size {
        self.size
    }

    pub fn tile_at(&self, coord: Coord) -> Option<TileView<'_>> {
        Some(self.view(self.cell(coord)?))
    }

    pub(crate) fn shared_tile_at(&self, coord: Coord) -> Option<(SharedTile, TileView<'_>)> {
        let cell = self.cell(coord)?;

        Some((SharedTile(Arc::as_ptr(&cell.prefabs)), self.view(cell)))
    }

    pub fn ids_at(&self, coord: Coord) -> &[PrefabInstanceId] {
        self.cell(coord).map(|cell| cell.ids.as_slice()).unwrap_or_default()
    }

    pub fn location(&self, id: PrefabInstanceId) -> Option<PrefabLocation> {
        self.locations.get(&id).copied()
    }

    pub fn prefab_instance(&self, id: PrefabInstanceId) -> Option<PrefabInstance<'_>> {
        let location = self.location(id)?;
        let prefab = self.tile_at(location.coord)?.get(location.prefab_index)?;

        Some(PrefabInstance::new(id, prefab, location))
    }

    pub fn prefab_instances(&self) -> impl Iterator<Item = PrefabInstance<'_>> {
        self.locations.iter().filter_map(|(id, location)| {
            let prefab = self.tile_at(location.coord)?.get(location.prefab_index)?;

            Some(PrefabInstance::new(*id, prefab, *location))
        })
    }

    pub(crate) fn pin(&mut self, prefab: &Prefab) {
        let id = self.table.intern(prefab);
        self.table.pin(id);
    }

    pub fn prefabs_of<'a>(&'a self, path: &'a TreePath) -> impl Iterator<Item = &'a Prefab> + 'a {
        self.table.live().filter(move |prefab| prefab.path == *path)
    }

    pub fn identical(&self, prefab: &Prefab) -> Vec<PrefabInstanceId> {
        let Some(wanted) = self.table.find(prefab) else {
            return Vec::new();
        };

        if self.table.placed(wanted) == 0 {
            return Vec::new();
        }

        let mut matching = HashMap::<*const [PrefabId], Vec<usize>>::new();
        let mut found = Vec::new();

        for z in 1..=self.size.z {
            for y in (1..=self.size.y).rev() {
                for x in 1..=self.size.x {
                    let Some(cell) = self.cell(Coord::new(x, y, z)) else {
                        continue;
                    };

                    let indices = matching.entry(Arc::as_ptr(&cell.prefabs)).or_insert_with(|| {
                        cell.prefabs
                            .iter()
                            .enumerate()
                            .filter(|(_, id)| **id == wanted)
                            .map(|(index, _)| index)
                            .collect()
                    });
                    found.extend(indices.iter().filter_map(|index| cell.ids.get(*index).copied()));
                }
            }
        }

        found
    }

    pub(crate) fn allocate(&mut self) -> PrefabInstanceId {
        let id = PrefabInstanceId::from_raw(self.next_id).expect("prefab instance IDs start at one");
        self.next_id = self.next_id.checked_add(1).expect("prefab instance ID space exhausted");

        id
    }

    pub(crate) fn clear(&mut self, coord: Coord) {
        let Some(index) = self.index(coord) else {
            return;
        };

        let cell = &mut self.cells[index];
        for id in cell.ids.drain(..) {
            self.locations.remove(&id);
        }

        let prefabs = mem::replace(&mut cell.prefabs, Arc::from([]));
        self.unplace_all(&prefabs);
    }

    pub(crate) fn place(&mut self, coord: Coord, placed: &[PlacedPrefab]) {
        let Some(index) = self.index(coord) else {
            return;
        };

        assert!(
            self.cells[index].ids.is_empty(),
            "prefab instances were inserted twice at {coord:?}"
        );

        for (prefab_index, placed) in placed.iter().enumerate() {
            let replaced = self
                .locations
                .insert(placed.id(), PrefabLocation { coord, prefab_index });
            assert!(
                replaced.is_none(),
                "prefab instance ID {} is present twice",
                placed.id().get()
            );
        }

        let prefabs = self.intern(placed.iter().map(PlacedPrefab::prefab));
        self.place_all(&prefabs);
        let prefabs = self.equal_neighbor(coord, &prefabs).unwrap_or(prefabs);
        let cell = &mut self.cells[index];
        cell.ids = placed.iter().map(PlacedPrefab::id).collect();
        cell.prefabs = prefabs;
    }

    /// this is an oppurtunistic dedup op, pretty easy and cheap way to compact tiles together to avoid memory
    /// explosions. Most of the maps are made out of empty spaces, we compare origin tile with its origins and return a
    /// clone of matched neigbors Arc
    fn equal_neighbor(&self, coord: Coord, prefabs: &[PrefabId]) -> Option<Arc<[PrefabId]>> {
        [(-1, 0), (1, 0), (0, -1), (0, 1)]
            .into_iter()
            .filter_map(|(dx, dy)| {
                let neighbor = Coord::new(
                    coord.x.checked_add_signed(dx)?,
                    coord.y.checked_add_signed(dy)?,
                    coord.z,
                );

                self.cell(neighbor)
            })
            .find(|cell| *cell.prefabs == *prefabs)
            .map(|cell| Arc::clone(&cell.prefabs))
    }

    pub(crate) fn resize(&mut self, width: u32, height: u32) {
        if (self.size.x, self.size.y) == (width, height) {
            return;
        }

        let size = Size {
            x: width,
            y: height,
            z: self.size.z,
        };
        let empty = Cell {
            prefabs: Arc::from([]),
            ids: Vec::new(),
        };
        let mut cells = vec![empty; cell_count(size)];
        let (width, height) = (self.size.x as usize, self.size.y as usize);

        for (index, cell) in mem::take(&mut self.cells).into_iter().enumerate() {
            let coord = Coord::new(
                (index % width) as u32 + 1,
                (index / width % height) as u32 + 1,
                (index / (width * height)) as u32 + 1,
            );
            match index_in(size, coord) {
                Some(new) => cells[new] = cell,
                None => {
                    for id in cell.ids {
                        self.locations.remove(&id);
                    }

                    self.unplace_all(&cell.prefabs);
                },
            }
        }

        self.size = size;
        self.cells = cells;
    }

    // every cell shares the one tile
    pub(crate) fn filled_level(&mut self, tile: &[Prefab]) -> LevelCells {
        let shared = Arc::new(tile.to_vec());
        let cells = (0..cell_count(Size { z: 1, ..self.size }))
            .map(|_| LevelCell {
                tile: Arc::clone(&shared),
                ids: tile.iter().map(|_| self.allocate()).collect(),
            })
            .collect();

        LevelCells { cells }
    }

    // equal tiles side by side share one, like `equal_neighbor` does
    pub(crate) fn level_from_tiles(&mut self, tiles: Vec<Tile>) -> LevelCells {
        let mut cells = Vec::<LevelCell>::with_capacity(tiles.len());
        for tile in tiles {
            let tile = match cells.last() {
                Some(previous) if *previous.tile == tile => Arc::clone(&previous.tile),
                _ => Arc::new(tile),
            };
            let ids = tile.iter().map(|_| self.allocate()).collect();
            cells.push(LevelCell { tile, ids });
        }

        LevelCells { cells }
    }

    // cells that shared prefabs share one tile, so `LevelCells::fill` still sees a filled level
    pub(crate) fn level(&self, z: u32) -> Option<LevelCells> {
        if z == 0 || z > self.size.z {
            return None;
        }

        let plane = cell_count(Size { z: 1, ..self.size });
        let start = (z as usize - 1) * plane;
        let mut tiles = HashMap::<*const [PrefabId], Arc<Tile>>::new();
        let cells = self.cells[start..start + plane]
            .iter()
            .map(|cell| LevelCell {
                tile: Arc::clone(
                    tiles
                        .entry(Arc::as_ptr(&cell.prefabs))
                        .or_insert_with(|| Arc::new(self.view(cell).to_tile())),
                ),
                ids: cell.ids.clone(),
            })
            .collect();

        Some(LevelCells { cells })
    }

    pub(crate) fn insert_level(&mut self, z: u32, level: &LevelCells) -> bool {
        let Some(levels) = self.size.z.checked_add(1) else {
            return false;
        };

        let plane = cell_count(Size { z: 1, ..self.size });
        if z == 0 || z > levels || level.cells.len() != plane {
            return false;
        }

        let mut ids = HashSet::new();
        if level
            .ids()
            .any(|id| self.locations.contains_key(&id) || !ids.insert(id))
        {
            return false;
        }

        let mut interned = HashMap::<*const Tile, Arc<[PrefabId]>>::new();
        let mut cells = Vec::with_capacity(level.cells.len());
        for cell in &level.cells {
            let prefabs = match interned.get(&Arc::as_ptr(&cell.tile)) {
                Some(prefabs) => Arc::clone(prefabs),
                None => {
                    let prefabs = self.intern(cell.tile.iter());
                    interned.insert(Arc::as_ptr(&cell.tile), Arc::clone(&prefabs));
                    prefabs
                },
            };
            self.place_all(&prefabs);
            cells.push(Cell {
                prefabs,
                ids: cell.ids.clone(),
            });
        }

        let start_offset = (z as usize - 1) * plane;
        // construct cells to start offset, this also pushes higher offsets upwards
        self.cells.splice(start_offset..start_offset, cells);
        self.size.z = levels;

        // we treat z levels as a stack and if we insert one in between, push the levels up
        for location in self.locations.values_mut().filter(|location| location.coord.z >= z) {
            location.coord.z += 1;
        }

        for (index, cell) in level.cells.iter().enumerate() {
            let coord = Coord::new(index as u32 % self.size.x + 1, index as u32 / self.size.x + 1, z);
            for (prefab_index, id) in cell.ids.iter().enumerate() {
                self.locations.insert(*id, PrefabLocation { coord, prefab_index });
            }
        }

        true
    }

    pub(crate) fn delete_level(&mut self, z: u32) -> bool {
        if self.size.z <= 1 || z == 0 || z > self.size.z {
            return false;
        }

        let plane = cell_count(Size { z: 1, ..self.size });
        let start = (z as usize - 1) * plane;
        for cell in self.cells.drain(start..start + plane).collect::<Vec<_>>() {
            for id in cell.ids {
                self.locations.remove(&id);
            }

            self.unplace_all(&cell.prefabs);
        }

        self.size.z -= 1;

        // dito
        for location in self.locations.values_mut().filter(|location| location.coord.z > z) {
            location.coord.z -= 1;
        }

        true
    }

    pub fn to_map(&self, baseline: &Map, format: MapFormat) -> Map {
        let size = self.size;
        let mut map = Map::new(size);
        map.key_length = baseline.key_length.max(1);
        map.format = format;

        // sorted, so of several keys holding equal tiles the lowest is always the one reused
        let mut keys = baseline.dictionary.keys().copied().collect::<Vec<_>>();
        keys.sort_unstable();
        let mut known = HashMap::<Vec<&TreePath>, Vec<(Key, TileView<'_>)>>::new();
        for key in keys {
            let tile = TileView::from(&baseline.dictionary[&key]);
            known.entry(paths(tile)).or_default().push((key, tile));
        }

        let mut resolved = HashMap::<(*const [PrefabId], Option<Key>), Key>::new();
        let mut cursor = 0;

        for z in 1..=size.z {
            for y in 1..=size.y {
                for x in 1..=size.x {
                    let coord = Coord::new(x, y, z);
                    let Some(cell) = self.cell(coord) else {
                        continue;
                    };

                    let previous = baseline.key_at(coord);
                    let key = *resolved
                        .entry((Arc::as_ptr(&cell.prefabs), previous))
                        .or_insert_with(|| {
                            let tile = self.view(cell);
                            let unchanged = previous.and_then(|key| {
                                let before = TileView::from(baseline.dictionary.get(&key)?);

                                tiles_equal(before, tile).then_some((key, before))
                            });
                            let existing = unchanged.or_else(|| {
                                known
                                    .get(&paths(tile))?
                                    .iter()
                                    .find(|(_, candidate)| tiles_equal(*candidate, tile))
                                    .copied()
                            });

                            let (key, contents) = existing.unwrap_or_else(|| {
                                while baseline.dictionary.contains_key(&Key(cursor))
                                    || map.dictionary.contains_key(&Key(cursor))
                                {
                                    cursor += 1;
                                }

                                let key = Key(cursor);
                                known.entry(paths(tile)).or_default().push((key, tile));

                                (key, tile)
                            });
                            map.dictionary.entry(key).or_insert_with(|| contents.to_tile());

                            key
                        });

                    map.grid[(z - 1) as usize][(size.y - y) as usize][(x - 1) as usize] = key;
                }
            }
        }

        map.prune_dictionary();
        map.reassign_overflowing_keys();

        map
    }

    #[cfg(test)]
    pub(crate) fn assert_placement_counts(&self) {
        let mut counted = HashMap::<PrefabId, u32>::new();
        for id in self.cells.iter().flat_map(|cell| cell.prefabs.iter()) {
            *counted.entry(*id).or_default() += 1;
        }

        for (id, count) in &counted {
            assert_eq!(self.table.placed(*id), *count, "{:?}", self.table.get(*id));
        }

        assert_eq!(
            self.table.placed_entries(),
            counted.len(),
            "a prefab no cell holds counts as placed"
        );
    }

    fn intern<'a>(&mut self, prefabs: impl IntoIterator<Item = &'a Prefab>) -> Arc<[PrefabId]> {
        prefabs.into_iter().map(|prefab| self.table.intern(prefab)).collect()
    }

    fn place_all(&mut self, prefabs: &[PrefabId]) {
        for id in prefabs {
            self.table.place(*id);
        }
    }

    fn unplace_all(&mut self, prefabs: &[PrefabId]) {
        for id in prefabs {
            self.table.unplace(*id);
        }
    }

    fn view<'a>(&'a self, cell: &'a Cell) -> TileView<'a> {
        TileView::interned(&self.table, &cell.prefabs)
    }

    fn index(&self, coord: Coord) -> Option<usize> {
        index_in(self.size, coord)
    }

    fn cell(&self, coord: Coord) -> Option<&Cell> {
        self.cells.get(self.index(coord)?)
    }
}

fn cell_count(size: Size) -> usize {
    size.x as usize * size.y as usize * size.z as usize
}

fn index_in(size: Size, coord: Coord) -> Option<usize> {
    let is_inside =
        (1..=size.x).contains(&coord.x) && (1..=size.y).contains(&coord.y) && (1..=size.z).contains(&coord.z);
    if !is_inside {
        return None;
    }

    let (x, y, z) = ((coord.x - 1) as usize, (coord.y - 1) as usize, (coord.z - 1) as usize);

    Some((z * size.y as usize + y) * size.x as usize + x)
}

fn paths(tile: TileView<'_>) -> Vec<&TreePath> {
    tile.iter().map(|prefab| &prefab.path).collect()
}

mod table;
mod view;

#[cfg(test)]
mod tests;
