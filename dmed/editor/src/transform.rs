use dmm::Coord;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TileAnchor {
    pub coord: Coord,
    pub adjust: [i32; 2],
}

pub(crate) fn anchor_to_tile(anchor: [f32; 2], origin: Coord, tile_size: u32, size: (u32, u32)) -> TileAnchor {
    let (x, adjust_x) = anchor_axis(anchor[0], origin.x, tile_size, size.0);
    let (y, adjust_y) = anchor_axis(anchor[1], origin.y, tile_size, size.1);

    TileAnchor {
        coord: Coord::new(x, y, origin.z),
        adjust: [adjust_x, adjust_y],
    }
}

pub(crate) fn anchor_axis(desired: f32, origin: u32, tile_size: u32, limit: u32) -> (u32, i32) {
    let tile = tile_size.max(1);
    let limit = limit.max(1) as i32;
    let nearest = if desired.is_finite() {
        (desired / tile as f32).round() as i32
    } else {
        origin as i32 - 1
    }
    .clamp(0, limit - 1);
    let target = nearest as u32 + 1;

    (target, (origin as i32 - target as i32) * tile as i32)
}

#[cfg(test)]
mod tests {
    use super::{Coord, anchor_axis, anchor_to_tile};

    #[test]
    fn positions_inside_the_cell_keep_the_current_tile() {
        assert_eq!(anchor_axis(0.0, 1, 32, 10), (1, 0));
        assert_eq!(anchor_axis(15.0, 1, 32, 10), (1, 0));
        assert_eq!(anchor_axis(35.0, 2, 32, 10), (2, 0));
        // An anchor hanging past the map edge falls back to the edge cell.
        assert_eq!(anchor_axis(-15.0, 2, 32, 10), (1, 32));
    }

    #[test]
    fn halfway_and_beyond_move_to_the_neighbouring_cell() {
        // fmod 16 at 32px: 16 and up belong to the next tile, -16 and below to
        // the previous one.
        assert_eq!(anchor_axis(16.0, 1, 32, 10), (2, -32));
        assert_eq!(anchor_axis(40.0, 1, 32, 10), (2, -32));
        assert_eq!(anchor_axis(-16.0, 2, 32, 10), (1, 32));
    }

    #[test]
    fn positions_past_the_map_stay_in_the_edge_cell() {
        assert_eq!(anchor_axis(900.0, 3, 32, 3), (3, 0));
        assert_eq!(anchor_axis(-900.0, 1, 32, 3), (1, 0));
    }

    #[test]
    fn reanchoring_keeps_the_rendered_position() {
        let origin = Coord::new(1, 1, 2);
        let anchored = anchor_to_tile([40.0, 80.0], origin, 32, (10, 10));

        assert_eq!(anchored.coord, Coord::new(2, 4, 2));
        assert_eq!(anchored.adjust, [-32, -96]);
        for axis in 0..2 {
            let before = [40.0, 80.0][axis];
            let origin_cell = [origin.x, origin.y][axis] as f32 - 1.0;
            let offset = before - origin_cell * 32.0 + anchored.adjust[axis] as f32;
            let rendered = ([anchored.coord.x, anchored.coord.y][axis] as f32 - 1.0) * 32.0 + offset;

            assert_eq!(rendered, before);
        }
    }
}
