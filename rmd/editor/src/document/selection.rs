use base::coord::Coord;

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

    pub fn enclosing(coords: impl IntoIterator<Item = Coord>) -> Option<Self> {
        let mut coords = coords.into_iter();
        let first = coords.next()?;

        Some(coords.fold(Self::from_drag(first, first), |bounds, coord| Self {
            min: Coord::new(bounds.min.x.min(coord.x), bounds.min.y.min(coord.y), bounds.min.z),
            max: Coord::new(bounds.max.x.max(coord.x), bounds.max.y.max(coord.y), bounds.max.z),
        }))
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

    pub fn width(&self) -> u32 {
        self.max.x - self.min.x + 1
    }

    pub fn height(&self) -> u32 {
        self.max.y - self.min.y + 1
    }

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
