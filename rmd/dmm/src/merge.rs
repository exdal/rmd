use std::collections::HashMap;

use crate::{Coord, Map, Prefab, Size, Tile, key::Key};

pub fn tiles_equal(left: &Tile, right: &Tile) -> bool {
    left.len() == right.len() && left.iter().zip(right).all(|(left, right)| prefabs_equal(left, right))
}

fn prefabs_equal(left: &Prefab, right: &Prefab) -> bool {
    left.path == right.path
        && left.vars.len() == right.vars.len()
        && left
            .vars
            .iter()
            .zip(&right.vars)
            .all(|((left_name, left), (right_name, right))| left_name == right_name && left.value == right.value)
}

#[derive(Debug, Clone, PartialEq)]
pub struct TileConflict {
    pub coord: Coord,
    pub base: Option<Tile>,
    pub ours: Option<Tile>,
    pub theirs: Option<Tile>,
}

#[derive(Debug, Clone)]
pub struct Merged {
    // conflicting tiles keep our side
    pub map: Map,
    pub conflicts: Vec<TileConflict>,
}

static EMPTY: Tile = Vec::new();

fn cell(map: &Map, coord: Coord) -> Option<(Key, &Tile)> {
    let key = map.key_at(coord)?;

    Some((key, map.dictionary.get(&key).unwrap_or(&EMPTY)))
}

#[derive(Default)]
struct Comparisons(HashMap<(Option<Key>, Option<Key>), bool>);

impl Comparisons {
    fn equal(&mut self, left: Option<(Key, &Tile)>, right: Option<(Key, &Tile)>) -> bool {
        match (left, right) {
            (None, None) => true,
            (Some((left_key, left)), Some((right_key, right))) => *self
                .0
                .entry((Some(left_key), Some(right_key)))
                .or_insert_with(|| tiles_equal(left, right)),
            _ => false,
        }
    }
}

fn merged_size(base: Size, ours: Size, theirs: Size) -> Size {
    if ours == base {
        theirs
    } else if theirs == base || ours == theirs {
        ours
    } else {
        Size {
            x: ours.x.max(theirs.x),
            y: ours.y.max(theirs.y),
            z: ours.z.max(theirs.z),
        }
    }
}

// threeway merge: ours, theirs against base, iterating one tile at a time
pub fn merge3(base: Option<&Map>, ours: &Map, theirs: &Map) -> Merged {
    let empty_base = Map::new(Size::default());
    let base = base.unwrap_or(&empty_base);
    let size = merged_size(base.size, ours.size, theirs.size);

    let mut map = ours.clone();
    let mut grid = vec![vec![vec![Key::default(); size.x as usize]; size.y as usize]; size.z as usize];
    let mut conflicts = Vec::new();
    let mut ours_base = Comparisons::default();
    let mut theirs_base = Comparisons::default();
    let mut ours_theirs = Comparisons::default();
    let mut imported: HashMap<Key, Key> = HashMap::new();
    let mut empty_key = None;

    for z in 1..=size.z {
        for y in 1..=size.y {
            for x in 1..=size.x {
                let coord = Coord::new(x, y, z);
                let base_cell = cell(base, coord);
                let ours_cell = cell(ours, coord);
                let theirs_cell = cell(theirs, coord);

                let take_theirs = if ours_base.equal(ours_cell, base_cell) {
                    true
                } else if theirs_base.equal(theirs_cell, base_cell) || ours_theirs.equal(ours_cell, theirs_cell) {
                    false
                } else {
                    conflicts.push(TileConflict {
                        coord,
                        base: base_cell.map(|(_, tile)| tile.clone()),
                        ours: ours_cell.map(|(_, tile)| tile.clone()),
                        theirs: theirs_cell.map(|(_, tile)| tile.clone()),
                    });
                    false
                };

                let key = match (take_theirs, ours_cell, theirs_cell) {
                    (false, Some((key, _)), _) => key,
                    (true, _, Some((key, tile))) => {
                        *imported.entry(key).or_insert_with(|| map.intern_tile(tile.clone()))
                    },
                    _ => *empty_key.get_or_insert_with(|| map.intern_tile(Vec::new())),
                };

                grid[(z - 1) as usize][(size.y - y) as usize][(x - 1) as usize] = key;
            }
        }
    }

    map.size = size;
    map.grid = grid;
    map.prune_dictionary();

    Merged { map, conflicts }
}

#[cfg(test)]
mod tests {
    use core::{path::TreePath, types::Value};

    use super::{merge3, tiles_equal};
    use crate::{Coord, Map, Prefab, Size, Tile, parser::parse, writer::write};

    fn map(source: &str) -> Map {
        let (map, errors) = parse(source);
        assert!(errors.is_empty(), "{errors:?}");
        map
    }

    fn paths(map: &Map, x: u32, y: u32) -> Vec<String> {
        map.tile_at(Coord::new(x, y, 1))
            .map(|tile| tile.iter().map(|prefab| prefab.path.to_string()).collect())
            .unwrap_or_default()
    }

    const BASE: &str = "\"a\" = (/turf/floor)\n\"b\" = (/turf/wall)\n\n(1,1,1) = {\"\naab\naaa\n\"}\n";

    #[test]
    fn takes_a_change_made_on_one_side() {
        let base = map(BASE);
        let ours = map(BASE);
        let theirs = map("\"a\" = (/turf/floor)\n\"b\" = (/turf/wall)\n\n(1,1,1) = {\"\nbab\naaa\n\"}\n");

        let merged = merge3(Some(&base), &ours, &theirs);

        assert!(merged.conflicts.is_empty());
        assert_eq!(paths(&merged.map, 1, 2), ["/turf/wall"]);
        assert_eq!(paths(&merged.map, 3, 2), ["/turf/wall"]);
    }

    #[test]
    fn the_same_change_on_both_sides_is_not_a_conflict() {
        let base = map(BASE);
        let changed = "\"a\" = (/turf/floor)\n\"b\" = (/turf/wall)\n\n(1,1,1) = {\"\naab\naab\n\"}\n";

        let merged = merge3(Some(&base), &map(changed), &map(changed));

        assert!(merged.conflicts.is_empty());
        assert_eq!(paths(&merged.map, 3, 1), ["/turf/wall"]);
    }

    #[test]
    fn divergent_changes_conflict_and_keep_our_side() {
        let base = map(BASE);
        let ours = map("\"a\" = (/turf/floor)\n\"b\" = (/turf/wall)\n\n(1,1,1) = {\"\naab\nbaa\n\"}\n");
        let theirs = map("\"a\" = (/turf/floor)\n\"c\" = (/turf/lava)\n\n(1,1,1) = {\"\naaa\ncaa\n\"}\n");

        let merged = merge3(Some(&base), &ours, &theirs);

        assert_eq!(merged.conflicts.len(), 1);
        let conflict = &merged.conflicts[0];
        assert_eq!(conflict.coord, Coord::new(1, 1, 1));
        assert_eq!(
            conflict.theirs.as_ref().map(|tile| tile[0].path.to_string()),
            Some("/turf/lava".to_owned())
        );
        assert_eq!(paths(&merged.map, 1, 1), ["/turf/wall"]);
        // Their clean removal of the wall at (3, 2) still lands
        assert_eq!(paths(&merged.map, 3, 2), ["/turf/floor"]);
    }

    #[test]
    fn keys_are_compared_by_contents_not_by_name() {
        let base = map(BASE);
        let ours = map(BASE);
        // Theirs swapped the key names without changing a single tile
        let theirs = map("\"a\" = (/turf/wall)\n\"b\" = (/turf/floor)\n\n(1,1,1) = {\"\nbba\nbbb\n\"}\n");

        let merged = merge3(Some(&base), &ours, &theirs);

        assert!(merged.conflicts.is_empty());
        assert_eq!(write(&merged.map), write(&ours));
    }

    #[test]
    fn growth_on_one_side_is_kept() {
        let base = map(BASE);
        let ours = map(BASE);
        let theirs = map("\"a\" = (/turf/floor)\n\"b\" = (/turf/wall)\n\n(1,1,1) = {\"\nbbbb\naaba\naaaa\n\"}\n");

        let merged = merge3(Some(&base), &ours, &theirs);

        assert!(merged.conflicts.is_empty());
        assert_eq!(merged.map.size, Size { x: 4, y: 3, z: 1 });
        assert_eq!(paths(&merged.map, 4, 3), ["/turf/wall"]);
        assert_eq!(paths(&merged.map, 3, 2), ["/turf/wall"]);
    }

    #[test]
    fn a_missing_base_conflicts_only_where_the_sides_differ() {
        let ours = map(BASE);
        let theirs = map("\"a\" = (/turf/floor)\n\"b\" = (/turf/wall)\n\n(1,1,1) = {\"\naab\naab\n\"}\n");

        let merged = merge3(None, &ours, &theirs);

        assert_eq!(merged.conflicts.len(), 1);
        assert_eq!(merged.conflicts[0].coord, Coord::new(3, 1, 1));
        assert_eq!(merged.conflicts[0].base, None);
    }

    #[test]
    fn number_spelling_does_not_change_a_tile() {
        let spelled = map("\"a\" = (/obj/item{damage = 1.30},/turf/floor)\n\n(1,1,1) = {\"\na\n\"}\n");
        let canonical: Tile = vec![
            {
                let mut prefab = Prefab::new(TreePath::parse("/obj/item"));
                prefab.set_var("damage".into(), Value::Num(1.3));
                prefab
            },
            Prefab::new(TreePath::parse("/turf/floor")),
        ];

        let tile = spelled.tile_at(Coord::new(1, 1, 1)).unwrap();
        assert_ne!(tile, &canonical);
        assert!(tiles_equal(tile, &canonical));
    }
}
