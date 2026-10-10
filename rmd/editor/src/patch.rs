use dmm::{Coord, Map, Prefab, Size, Tile, parser, writer};
use net::LevelContents;

use crate::grid::{Grid, LevelCells};

// I kinda dont like this patch sending business, but for now its okay
// if I was bikeshedding I would be sending delta compressed binary
// this can stay for now, until it becomes problematic

pub fn encode(map: &Grid, coords: impl IntoIterator<Item = Coord>) -> Option<(Vec<Coord>, String)> {
    let tiles = coords
        .into_iter()
        .filter_map(|coord| map.tile_at(coord).map(|tile| (coord, tile)))
        .collect::<Vec<_>>();

    if tiles.is_empty() {
        return None;
    }

    let patch = write_row(tiles.iter().map(|(_, tile)| tile.as_slice()));

    Some((tiles.into_iter().map(|(coord, _)| coord).collect(), patch))
}

pub fn encode_tile(tile: &[Prefab]) -> String { write_row([tile]) }

pub fn encode_level(level: &LevelCells) -> LevelContents {
    match level.fill() {
        Some(tile) => LevelContents::Fill(encode_tile(tile)),
        None => LevelContents::Tiles(write_row(level.tiles().map(Vec::as_slice))),
    }
}

// contents that don't parse make an empty level, so every copy still numbers its levels alike
pub fn decode_level(map: &mut Grid, contents: &LevelContents) -> LevelCells {
    let size = map.size();
    let count = (size.x * size.y) as usize;
    let decoded = match contents {
        LevelContents::Fill(fill) => decode(fill, 1).map(|mut tiles| map.filled_level(&tiles.remove(0))),
        LevelContents::Tiles(tiles) => decode(tiles, count).map(|tiles| map.level_from_tiles(tiles)),
    };

    decoded.unwrap_or_else(|e| {
        log::warn!("dropping level contents that do not parse: {e}");
        map.level_from_tiles(vec![Tile::new(); count])
    })
}

fn write_row<'a>(tiles: impl IntoIterator<Item = &'a [Prefab]>) -> String {
    let tiles = tiles.into_iter().collect::<Vec<_>>();
    let mut patch = Map::new(Size {
        x: tiles.len() as u32,
        y: 1,
        z: 1,
    });

    for (index, tile) in tiles.into_iter().enumerate() {
        patch.grid[0][0][index] = patch.intern_tile(tile.to_vec());
    }

    writer::write(&patch)
}

pub fn decode(patch: &str, count: usize) -> Result<Vec<Vec<Prefab>>, String> {
    let (map, errors) = parser::parse(patch);
    if let Some(error) = errors.first() {
        return Err(error.to_string());
    }

    if map.size.x as usize != count || map.size.y != 1 || map.size.z != 1 {
        return Err(format!(
            "expected {count} tiles in a row, got {}x{}x{}",
            map.size.x, map.size.y, map.size.z
        ));
    }

    (1..=count as u32)
        .map(|x| {
            map.tile_at(Coord::new(x, 1, 1))
                .cloned()
                .ok_or_else(|| format!("tile {x} is missing"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAP: &str = r#""a" = (/obj/item{name = "a \"quoted\" name"; pixel_x = -4},/turf/open/floor,/area/station)
"b" = (/turf/closed/wall,/area/station)

(1,1,1) = {"
ab
ba
"}
"#;

    #[test]
    fn tiles_survive_the_round_trip_in_order() {
        let (map, errors) = parser::parse(MAP);
        assert!(errors.is_empty());

        let (coords, patch) = encode(
            &Grid::from_map(&map),
            [
                Coord::new(2, 1, 1),
                Coord::new(1, 1, 1),
                Coord::new(9, 9, 9),
                Coord::new(2, 2, 1),
            ],
        )
        .unwrap();
        assert_eq!(coords, [Coord::new(2, 1, 1), Coord::new(1, 1, 1), Coord::new(2, 2, 1)]);

        let tiles = decode(&patch, coords.len()).unwrap();
        for (coord, tile) in coords.iter().zip(&tiles) {
            assert_eq!(Some(tile), map.tile_at(*coord), "{coord:?}");
        }

        assert!(encode(&Grid::from_map(&map), [Coord::new(9, 9, 9)]).is_none());
    }

    #[test]
    fn an_emptied_tile_is_sent_too() {
        let (mut map, _) = parser::parse(MAP);
        let empty = map.intern_tile(Vec::new());
        map.grid[0][0][0] = empty;

        let (coords, patch) = encode(&Grid::from_map(&map), [Coord::new(1, 2, 1), Coord::new(2, 2, 1)]).unwrap();
        assert_eq!(coords, [Coord::new(1, 2, 1), Coord::new(2, 2, 1)]);

        let tiles = decode(&patch, coords.len()).unwrap();
        assert!(tiles[0].is_empty());
        assert_eq!(Some(&tiles[1]), map.tile_at(Coord::new(2, 2, 1)));
    }

    #[test]
    fn a_patch_of_the_wrong_shape_is_refused() {
        let (map, _) = parser::parse(MAP);
        let (_, patch) = encode(&Grid::from_map(&map), [Coord::new(1, 1, 1)]).unwrap();

        assert!(decode(&patch, 2).is_err());
        assert!(decode("not a map", 1).is_err());
    }
}
