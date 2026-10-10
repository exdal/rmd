use core::path::TreePath;
use std::{
    collections::{HashMap, HashSet},
    mem,
    sync::Arc,
};

use dmm::{Coord, Map, MapFormat, Prefab, PrefabInstanceId, Size, Tile, key::Key, merge::tiles_equal};

use crate::document::{PlacedPrefab, PrefabInstance, PrefabLocation};

#[derive(Debug, Clone, PartialEq)]
struct Cell {
    tile: Arc<Tile>,
    ids: Vec<PrefabInstanceId>,
}

#[derive(Debug, Clone)]
pub struct LevelCells {
    cells: Vec<Cell>,
}

impl LevelCells {
    pub fn ids(&self) -> impl Iterator<Item = PrefabInstanceId> + '_ {
        self.cells.iter().flat_map(|cell| cell.ids.iter().copied())
    }

    pub fn tiles(&self) -> impl Iterator<Item = &Tile> + '_ { self.cells.iter().map(|cell| cell.tile.as_ref()) }

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
}

impl PartialEq for Grid {
    fn eq(&self, other: &Self) -> bool { self.size == other.size && self.cells == other.cells }
}

impl Grid {
    pub fn from_map(map: &Map) -> Self {
        let shared = map
            .dictionary
            .iter()
            .map(|(key, tile)| (*key, Arc::new(tile.clone())))
            .collect::<HashMap<_, _>>();
        let empty = Arc::new(Tile::new());
        let mut grid = Self {
            size: map.size,
            cells: Vec::with_capacity(cell_count(map.size)),
            locations: HashMap::new(),
            next_id: 1,
        };

        for z in 1..=map.size.z {
            for y in 1..=map.size.y {
                for x in 1..=map.size.x {
                    let coord = Coord::new(x, y, z);
                    let tile = map.key_at(coord).and_then(|key| shared.get(&key)).unwrap_or(&empty);
                    let ids = (0..tile.len()).map(|_| grid.allocate()).collect::<Vec<_>>();
                    for (prefab_index, id) in ids.iter().enumerate() {
                        grid.locations.insert(*id, PrefabLocation { coord, prefab_index });
                    }

                    grid.cells.push(Cell {
                        tile: Arc::clone(tile),
                        ids,
                    });
                }
            }
        }

        grid
    }

    pub fn size(&self) -> Size { self.size }

    pub fn tile_at(&self, coord: Coord) -> Option<&Tile> { Some(&self.cell(coord)?.tile) }

    pub(crate) fn shared_tile_at(&self, coord: Coord) -> Option<&Arc<Tile>> { Some(&self.cell(coord)?.tile) }

    pub fn ids_at(&self, coord: Coord) -> &[PrefabInstanceId] {
        self.cell(coord).map(|cell| cell.ids.as_slice()).unwrap_or_default()
    }

    pub fn location(&self, id: PrefabInstanceId) -> Option<PrefabLocation> { self.locations.get(&id).copied() }

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

    pub fn identical(&self, prefab: &Prefab) -> Vec<PrefabInstanceId> {
        let mut matching = HashMap::<*const Tile, Vec<usize>>::new();
        let mut found = Vec::new();

        for z in 1..=self.size.z {
            for y in (1..=self.size.y).rev() {
                for x in 1..=self.size.x {
                    let Some(cell) = self.cell(Coord::new(x, y, z)) else {
                        continue;
                    };

                    let indices = matching.entry(Arc::as_ptr(&cell.tile)).or_insert_with(|| {
                        cell.tile
                            .iter()
                            .enumerate()
                            .filter(|(_, placed)| *placed == prefab)
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

        cell.tile = Arc::default();
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

        let tile = self
            .equal_neighbor(coord, placed)
            .unwrap_or_else(|| Arc::new(placed.iter().map(|placed| placed.prefab().clone()).collect()));
        let cell = &mut self.cells[index];
        cell.ids = placed.iter().map(PlacedPrefab::id).collect();
        cell.tile = tile;
    }

    /// this is an oppurtunistic dedup op, pretty easy and cheap way to compact tiles together to avoid memory
    /// explosions. Most of the maps are made out of empty spaces, we compare origin tile with its origins and return a
    /// clone of matched neigbors Arc
    fn equal_neighbor(&self, coord: Coord, placed: &[PlacedPrefab]) -> Option<Arc<Tile>> {
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
            .find(|cell| {
                cell.tile.len() == placed.len()
                    && cell
                        .tile
                        .iter()
                        .zip(placed)
                        .all(|(prefab, placed)| prefab == placed.prefab())
            })
            .map(|cell| Arc::clone(&cell.tile))
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
            tile: Arc::default(),
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
            .map(|_| Cell {
                tile: Arc::clone(&shared),
                ids: tile.iter().map(|_| self.allocate()).collect(),
            })
            .collect();

        LevelCells { cells }
    }

    // equal tiles side by side share one, like `equal_neighbor` does
    pub(crate) fn level_from_tiles(&mut self, tiles: Vec<Tile>) -> LevelCells {
        let mut cells = Vec::<Cell>::with_capacity(tiles.len());
        for tile in tiles {
            let tile = match cells.last() {
                Some(previous) if *previous.tile == tile => Arc::clone(&previous.tile),
                _ => Arc::new(tile),
            };
            let ids = tile.iter().map(|_| self.allocate()).collect();
            cells.push(Cell { tile, ids });
        }

        LevelCells { cells }
    }

    pub(crate) fn level(&self, z: u32) -> Option<LevelCells> {
        if z == 0 || z > self.size.z {
            return None;
        }

        let plane = cell_count(Size { z: 1, ..self.size });
        let start = (z as usize - 1) * plane;

        Some(LevelCells {
            cells: self.cells[start..start + plane].to_vec(),
        })
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

        let start_offset = (z as usize - 1) * plane;
        // construct cells to start offset, this also pushes higher offsets upwards
        self.cells
            .splice(start_offset..start_offset, level.cells.iter().cloned());
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
        for cell in self.cells.drain(start..start + plane) {
            for id in cell.ids {
                self.locations.remove(&id);
            }
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
        let mut known = HashMap::<Vec<&TreePath>, Vec<(Key, &Tile)>>::new();
        for key in keys {
            let tile = &baseline.dictionary[&key];
            known.entry(paths(tile)).or_default().push((key, tile));
        }

        let mut resolved = HashMap::<(*const Tile, Option<Key>), Key>::new();
        let mut cursor = 0;

        for z in 1..=size.z {
            for y in 1..=size.y {
                for x in 1..=size.x {
                    let coord = Coord::new(x, y, z);
                    let Some(cell) = self.cell(coord) else {
                        continue;
                    };

                    let previous = baseline.key_at(coord);
                    let key = *resolved.entry((Arc::as_ptr(&cell.tile), previous)).or_insert_with(|| {
                        let tile = cell.tile.as_ref();
                        let unchanged = previous.and_then(|key| {
                            let before = baseline.dictionary.get(&key)?;

                            tiles_equal(before, tile).then_some((key, before))
                        });
                        let existing = unchanged.or_else(|| {
                            known
                                .get(&paths(tile))?
                                .iter()
                                .find(|(_, candidate)| tiles_equal(candidate, tile))
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
                        map.dictionary.entry(key).or_insert_with(|| contents.clone());

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

    fn index(&self, coord: Coord) -> Option<usize> { index_in(self.size, coord) }

    fn cell(&self, coord: Coord) -> Option<&Cell> { self.cells.get(self.index(coord)?) }
}

fn cell_count(size: Size) -> usize { size.x as usize * size.y as usize * size.z as usize }

fn index_in(size: Size, coord: Coord) -> Option<usize> {
    let is_inside =
        (1..=size.x).contains(&coord.x) && (1..=size.y).contains(&coord.y) && (1..=size.z).contains(&coord.z);
    if !is_inside {
        return None;
    }

    let (x, y, z) = ((coord.x - 1) as usize, (coord.y - 1) as usize, (coord.z - 1) as usize);

    Some((z * size.y as usize + y) * size.x as usize + x)
}

fn paths(tile: &Tile) -> Vec<&TreePath> { tile.iter().map(|prefab| &prefab.path).collect() }

#[cfg(test)]
mod tests {
    use core::{path::TreePath, types::Value};
    use std::sync::Arc;

    use dmm::{Coord, Map, Prefab, PrefabInstanceId, key::Key, parser, writer};

    use super::Grid;
    use crate::{
        command::Edit,
        document::{MapDocument, PlacedTile},
    };

    // "b" holds the same prefabs as "a"
    const MAP: &str = r#""a" = (/turf/open/floor,/area/station)
"b" = (/turf/open/floor,/area/station)
"q" = (/obj/machinery/power/smes{charge = 2e+005},/turf/open/floor,/area/station)
"z" = (/turf/closed/wall,/area/station)

(1,1,1) = {"
aqz
bza
"}
"#;

    fn map() -> Map {
        let (map, errors) = parser::parse(MAP);
        assert!(errors.is_empty(), "{errors:?}");

        map
    }

    fn key(name: &str) -> Key { Key::parse(name).unwrap() }

    fn replace(document: &mut MapDocument, coord: Coord, tile: Vec<Prefab>) {
        let placed = tile
            .into_iter()
            .map(|prefab| document.instantiate(prefab))
            .collect::<PlacedTile>();
        let mut edit = Edit::new("replace");
        edit.change(document, coord, placed);
        assert!(document.apply(edit));
    }

    #[test]
    fn an_untouched_map_exports_byte_for_byte() {
        let map = map();
        let grid = Grid::from_map(&map);

        assert_eq!(writer::write(&grid.to_map(&map, map.format)), writer::write(&map));
    }

    #[test]
    fn tiles_of_one_key_share_an_allocation() {
        let grid = Grid::from_map(&map());
        let shared = |x, y| grid.shared_tile_at(Coord::new(x, y, 1)).unwrap();

        assert!(Arc::ptr_eq(shared(1, 2), shared(3, 1)));
        assert!(!Arc::ptr_eq(shared(1, 2), shared(1, 1)));
    }

    #[test]
    fn ids_count_up_from_the_bottom_row() {
        let grid = Grid::from_map(&map());
        let ids = |x, y| {
            grid.ids_at(Coord::new(x, y, 1))
                .iter()
                .map(|id| id.get())
                .collect::<Vec<_>>()
        };

        assert_eq!(ids(1, 1), [1, 2]);
        assert_eq!(ids(2, 1), [3, 4]);
        assert_eq!(ids(3, 1), [5, 6]);
        assert_eq!(ids(1, 2), [7, 8]);
        assert_eq!(ids(2, 2), [9, 10, 11]);
        let id = PrefabInstanceId::from_raw(9).unwrap();
        assert_eq!(grid.location(id).unwrap().coord, Coord::new(2, 2, 1));
    }

    #[test]
    fn an_edited_tile_takes_a_new_key_and_the_rest_keep_theirs() {
        let original = map();
        let mut document = MapDocument::new(original.clone(), 1);
        let edited = Coord::new(3, 2, 1);
        replace(
            &mut document,
            edited,
            vec![Prefab::new(TreePath::parse("/turf/open/lava"))],
        );

        let exported = document.to_map();
        for y in 1..=2 {
            for x in 1..=3 {
                let coord = Coord::new(x, y, 1);
                if coord != edited {
                    assert_eq!(exported.key_at(coord), original.key_at(coord), "{coord:?}");
                }
            }
        }

        let new = exported.key_at(edited).unwrap();
        assert!(!original.dictionary.contains_key(&new));
        assert_eq!(
            exported.tile_at(edited).unwrap()[0].path,
            TreePath::parse("/turf/open/lava")
        );
    }

    #[test]
    fn a_tile_edited_back_gets_its_key_and_spelling_back() {
        let mut document = MapDocument::new(map(), 1);
        let coord = Coord::new(2, 2, 1);
        let before = document.map.tile_at(coord).unwrap().clone();
        replace(&mut document, coord, Vec::new());

        let mut smes = Prefab::new(TreePath::parse("/obj/machinery/power/smes"));
        smes.set_var("charge".into(), Value::Num(200000.0));
        let mut restored = before.clone();
        restored[0] = smes;
        replace(&mut document, coord, restored);

        assert_eq!(writer::write(&document.to_map()), writer::write(&map()));
    }

    #[test]
    fn a_copied_tile_reuses_the_key_it_was_copied_from() {
        let mut document = MapDocument::new(map(), 1);
        let smes = document.map.tile_at(Coord::new(2, 2, 1)).unwrap().clone();
        replace(&mut document, Coord::new(1, 1, 1), smes);

        let exported = document.to_map();
        assert_eq!(exported.key_at(Coord::new(1, 1, 1)), Some(key("q")));
        assert!(!exported.dictionary.contains_key(&key("b")));
    }

    #[test]
    fn a_tile_matching_several_keys_always_takes_the_lowest() {
        // each parse seeds a new hash order
        for _ in 0..32 {
            let mut document = MapDocument::new(map(), 1);
            let floor = document.map.tile_at(Coord::new(1, 2, 1)).unwrap().clone();
            replace(&mut document, Coord::new(3, 2, 1), floor);

            assert_eq!(document.to_map().key_at(Coord::new(3, 2, 1)), Some(key("a")));
        }
    }

    #[test]
    fn a_fill_shares_one_tile() {
        let mut document = MapDocument::new(map(), 1);
        let mut edit = Edit::new("fill");
        for x in 1..=3 {
            let lava = vec![document.instantiate(Prefab::new(TreePath::parse("/turf/open/lava")))];
            edit.change(&document, Coord::new(x, 1, 1), lava);
        }
        assert!(document.apply(edit));

        let shared = |x| document.map.shared_tile_at(Coord::new(x, 1, 1)).unwrap();
        assert!(Arc::ptr_eq(shared(1), shared(2)));
        assert!(Arc::ptr_eq(shared(2), shared(3)));
    }

    #[test]
    fn painting_next_to_an_equal_tile_shares_it() {
        let mut document = MapDocument::new(map(), 1);
        for x in 1..=3 {
            replace(
                &mut document,
                Coord::new(x, 1, 1),
                vec![Prefab::new(TreePath::parse("/turf/open/lava"))],
            );
        }

        let shared = |x| document.map.shared_tile_at(Coord::new(x, 1, 1)).unwrap();
        assert!(Arc::ptr_eq(shared(1), shared(2)));
        assert!(Arc::ptr_eq(shared(2), shared(3)));
    }
}
