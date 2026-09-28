pub const NORTH: u32 = 1;
pub const SOUTH: u32 = 2;
pub const EAST: u32 = 4;
pub const WEST: u32 = 8;
pub const NORTHEAST: u32 = NORTH | EAST;
pub const NORTHWEST: u32 = NORTH | WEST;
pub const SOUTHEAST: u32 = SOUTH | EAST;
pub const SOUTHWEST: u32 = SOUTH | WEST;

pub const FLOAT_LAYER: f32 = -1.0;
pub const FLOAT_PLANE: f32 = -32767.0;

pub const RESET_COLOR: u32 = 2;
pub const RESET_ALPHA: u32 = 4;
pub const RESET_TRANSFORM: u32 = 8;
pub const KEEP_TOGETHER: u32 = 32;
pub const KEEP_APART: u32 = 64;
