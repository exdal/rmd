use dmm::{Coord, Size};

pub(super) fn centered_paste_min(width: u32, height: u32, anchor: Coord, map_size: Size) -> Coord {
    let place = |anchor: u32, extent: u32, limit: u32| {
        let extent = extent.max(1);
        let last = limit.saturating_sub(extent - 1).max(1);

        anchor.saturating_sub((extent - 1) / 2).max(1).min(last)
    };

    Coord::new(
        place(anchor.x, width, map_size.x),
        place(anchor.y, height, map_size.y),
        anchor.z,
    )
}

#[cfg(test)]
mod tests {
    use dmm::{Coord, Size};

    use super::centered_paste_min;

    #[test]
    fn a_pasted_region_lands_centered_on_the_cursor() {
        let size = Size { x: 20, y: 20, z: 1 };

        // An odd extent sits dead centre; an even one leans one tile left/down.
        assert_eq!(
            centered_paste_min(3, 3, Coord::new(10, 10, 1), size),
            Coord::new(9, 9, 1)
        );
        assert_eq!(
            centered_paste_min(4, 4, Coord::new(10, 10, 1), size),
            Coord::new(9, 9, 1)
        );
        assert_eq!(
            centered_paste_min(1, 1, Coord::new(10, 10, 1), size),
            Coord::new(10, 10, 1)
        );
    }

    #[test]
    fn a_pasted_region_is_nudged_back_inside_the_map() {
        let size = Size { x: 10, y: 10, z: 1 };

        // Centring near an edge would hang the region off the map, so it slides
        // back until the whole footprint fits.
        assert_eq!(centered_paste_min(5, 5, Coord::new(1, 1, 1), size), Coord::new(1, 1, 1));
        assert_eq!(
            centered_paste_min(5, 5, Coord::new(10, 10, 1), size),
            Coord::new(6, 6, 1)
        );
        // A region larger than the map still starts at the origin rather than
        // clamping to nothing.
        assert_eq!(
            centered_paste_min(40, 40, Coord::new(5, 5, 1), size),
            Coord::new(1, 1, 1)
        );
    }
}
