/// DM counts `y` from the bottom
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Coord {
    pub x: u32,
    pub y: u32,
    pub z: u32,
}

impl Coord {
    pub fn new(x: u32, y: u32, z: u32) -> Self {
        Self { x, y, z }
    }
}
