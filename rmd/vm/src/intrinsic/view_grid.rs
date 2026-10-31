#[derive(Clone, Copy)]
pub(crate) enum Sight {
    Through,
    Line,
}

#[derive(Clone, Copy)]
pub(crate) struct ViewTile {
    pub(crate) is_opaque: bool,
    pub(crate) vis: i32,
    pub(crate) vis2: i32,
}

/// OpenDream's `ViewAlgorithm`, Lummox's description of BYOND's view() at
/// https://www.byond.com/forum/post/2130277#comment20659267. Like OpenDream it skips the light and
/// infrared steps, so dark tiles stay visible.
pub(crate) struct ViewGrid {
    width: i32,
    height: i32,
    pub(crate) tiles: Vec<Option<ViewTile>>,
}

impl ViewGrid {
    pub(crate) fn new(width: i32, height: i32) -> Self {
        Self {
            width,
            height,
            tiles: vec![None; (width.max(0) * height.max(0)) as usize],
        }
    }

    fn index(&self, x: i32, y: i32) -> Option<usize> {
        let is_inside = x >= 0 && y >= 0 && x < self.width && y < self.height;
        is_inside.then(|| (y * self.width + x) as usize)
    }

    pub(crate) fn get(&self, x: i32, y: i32) -> Option<ViewTile> {
        self.index(x, y).and_then(|index| self.tiles[index])
    }

    fn get_mut(&mut self, x: i32, y: i32) -> Option<&mut ViewTile> {
        self.index(x, y).and_then(|index| self.tiles[index].as_mut())
    }

    pub(crate) fn calculate(&mut self) {
        let (eye_x, eye_y) = (self.width / 2, self.height / 2);
        let highest_max_delta = (self.width - eye_x).max(self.height - eye_y);
        let highest_sum_delta = (self.width - eye_x) + (self.height - eye_y);

        for d in 0..highest_max_delta {
            let (lower_x, upper_x) = (eye_x - d - 1, eye_x + d + 1);
            let (lower_y, upper_y) = (eye_y - d - 1, eye_y + d + 1);
            for x in lower_x.max(0)..=upper_x.min(self.width - 1) {
                let is_edge = x == lower_x || x == upper_x;
                for y in lower_y.max(0)..=upper_y.min(self.height - 1) {
                    if !is_edge && y != lower_y && y != upper_y {
                        continue;
                    }

                    if self.neighbors_match(x, y, |tile| tile.vis2 == d)
                        && let Some(tile) = self.get_mut(x, y)
                    {
                        tile.vis2 = if tile.is_opaque { -1 } else { d + 1 };
                    }
                }
            }
        }

        for d in 0..highest_sum_delta {
            for x in (eye_x - d - 1).max(0)..=(eye_x + d + 1).min(self.width - 1) {
                let offset_y = (d + 1) - (eye_x - x).abs();
                self.update_straight(x, eye_y - offset_y, d);
                if offset_y != 0 {
                    self.update_straight(x, eye_y + offset_y, d);
                }
            }
        }

        if let Some(eye) = self.get_mut(eye_x, eye_y) {
            eye.vis = 1;
        }

        let mut boundaries = Vec::new();
        for x in 0..self.width {
            for y in 0..self.height {
                let Some(tile) = self.get(x, y) else {
                    continue;
                };

                if !tile.is_opaque || tile.vis != 0 {
                    continue;
                }

                let is_seen = |x: i32, y: i32| self.get(x, y).is_some_and(|tile| tile.vis != 0);
                let is_horizontal_wall = x != 0 && x != self.width - 1 && is_seen(x - 1, y) && is_seen(x + 1, y);
                let is_vertical_wall = y != 0 && y != self.height - 1 && is_seen(x, y - 1) && is_seen(x, y + 1);
                let is_corner = [(1, 1), (1, -1), (-1, -1), (-1, 1)]
                    .into_iter()
                    .any(|(dx, dy)| self.is_corner(x, y, dx, dy));
                if is_horizontal_wall || is_vertical_wall || is_corner {
                    boundaries.push((x, y));
                }
            }
        }

        for (x, y) in boundaries {
            if let Some(tile) = self.get_mut(x, y) {
                tile.vis = -1;
            }
        }
    }

    fn update_straight(&mut self, x: i32, y: i32, d: i32) {
        if self.get(x, y).is_none() || !self.neighbors_match(x, y, |tile| tile.vis == d) {
            return;
        }

        if let Some(tile) = self.get_mut(x, y) {
            if tile.is_opaque {
                tile.vis = -1;
            } else if tile.vis2 != 0 {
                tile.vis = d + 1;
            }
        }
    }

    fn neighbors_match(&self, x: i32, y: i32, matches: impl Fn(ViewTile) -> bool) -> bool {
        for neighbor_x in x - 1..=x + 1 {
            for neighbor_y in y - 1..=y + 1 {
                if (neighbor_x, neighbor_y) == (x, y) {
                    continue;
                }

                if self.get(neighbor_x, neighbor_y).is_some_and(&matches) {
                    return true;
                }
            }
        }

        false
    }

    fn is_corner(&self, x: i32, y: i32, dx: i32, dy: i32) -> bool {
        let (Some(diagonal), Some(first), Some(second)) =
            (self.get(x + dx, y + dy), self.get(x, y + dy), self.get(x + dx, y))
        else {
            return false;
        };

        diagonal.vis != 0
            && first.vis != 0
            && second.vis != 0
            && first.is_opaque
            && second.is_opaque
            && !diagonal.is_opaque
    }
}

/// OpenDream's `DantomSpiral`: rings outward from the centre, each one left column, then the rows
/// below and above, then right column.
pub(crate) fn dantom_spiral(center: (i32, i32), size: (i32, i32)) -> Vec<(i32, i32)> {
    let (width, height) = size;
    let bottom = height / 2;
    let top = (height - 1) / 2;
    let left = width / 2;
    let right = (width - 1) / 2;
    let mut tiles = Vec::new();

    for ring in 1..=bottom.max(left) {
        let column_bottom = center.1 - ring.min(bottom);
        let column_top = center.1 + ring.min(top);
        if ring <= left {
            tiles.extend((column_bottom..=column_top).map(|y| (center.0 - ring, y)));
        }

        if ring <= bottom {
            let first = center.0 - ring.min(left + 1) + 1;
            let last = center.0 + ring.min(right + 1) - 1;
            for x in first..=last {
                tiles.push((x, center.1 - ring));
                if ring <= top {
                    tiles.push((x, center.1 + ring));
                }
            }
        }

        if ring <= right {
            tiles.extend((column_bottom..=column_top).map(|y| (center.0 + ring, y)));
        }
    }

    tiles
}
