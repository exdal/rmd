use std::{collections::HashSet, ops::Range};

use crate::world::Position;

const CORNER_SW: usize = 0;
const CORNER_SE: usize = 1;
const CORNER_NW: usize = 2;
const CORNER_NE: usize = 3;
const EPSILON: f32 = 1.0e-6;

/// The four static-light samples for one map tile, ordered southwest, southeast,
/// northwest, northeast.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightTile {
    pub corners: [[f32; 3]; 4],
}

impl Default for LightTile {
    fn default() -> Self { Self { corners: [[0.0; 3]; 4] } }
}

/// A dense, z-major static lightmap. Tile coordinates are one-based at the API
/// boundary and use the same x/y/z convention as [`Position`].
#[derive(Debug, Clone, PartialEq)]
pub struct LightingMap {
    pub size: [u32; 3],
    pub tiles: Vec<LightTile>,
}

impl LightingMap {
    pub(crate) fn new(size: [i32; 3]) -> Self {
        let size = size.map(|value| u32::try_from(value.max(0)).unwrap_or(0));
        let len = usize::try_from(size[0])
            .ok()
            .and_then(|width| {
                usize::try_from(size[1])
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .and_then(|plane| {
                usize::try_from(size[2])
                    .ok()
                    .and_then(|levels| plane.checked_mul(levels))
            })
            .unwrap_or(0);

        Self {
            size,
            tiles: vec![LightTile::default(); len],
        }
    }

    pub fn level_range(&self, z: u32) -> Option<Range<usize>> {
        if z == 0 || z > self.size[2] {
            return None;
        }

        let plane = usize::try_from(self.size[0])
            .ok()?
            .checked_mul(usize::try_from(self.size[1]).ok()?)?;
        let start = usize::try_from(z - 1).ok()?.checked_mul(plane)?;

        Some(start..start.checked_add(plane)?)
    }

    pub fn tile(&self, position: Position) -> Option<&LightTile> {
        tile_index(self.size, position).and_then(|index| self.tiles.get(index))
    }

    pub(crate) fn solve_all<'a>(&mut self, atoms: impl Iterator<Item = &'a LightingAtom> + Clone) {
        for z in 1..=self.size[2] {
            self.solve_level(z, atoms.clone().filter(|atom| atom.position.z == z as i32));
        }
    }

    pub(crate) fn solve_levels<'a>(
        &mut self, atoms: impl Iterator<Item = &'a LightingAtom> + Clone, levels: &HashSet<u32>,
    ) -> Option<Range<usize>> {
        let mut changed: Option<Range<usize>> = None;
        let mut levels = levels.iter().copied().collect::<Vec<_>>();
        levels.sort_unstable();

        for z in levels {
            let Some(range) = self.level_range(z) else {
                continue;
            };
            self.solve_level(z, atoms.clone().filter(|atom| atom.position.z == z as i32));
            changed = Some(match changed {
                Some(current) => current.start.min(range.start)..current.end.max(range.end),
                None => range,
            });
        }

        changed
    }

    fn solve_level<'a>(&mut self, z: u32, atoms: impl Iterator<Item = &'a LightingAtom>) {
        let width = self.size[0] as usize;
        let height = self.size[1] as usize;
        let Some(range) = self.level_range(z) else {
            return;
        };

        if width == 0 || height == 0 {
            return;
        }

        let mut cells = vec![Cell::default(); width.saturating_mul(height)];
        let mut sources = Vec::new();
        for atom in atoms {
            let Some(index) = cell_index(width, height, atom.position.x, atom.position.y) else {
                continue;
            };
            let cell = &mut cells[index];
            cell.blocked |= atom.blocks;
            cell.fullbright |= atom.fullbright;
            for (slot, value) in cell.ambient.iter_mut().zip(atom.ambient) {
                *slot += value;
            }
            if atom.source.is_some() {
                sources.push(*atom);
            }
        }

        let corner_width = width + 1;
        let corner_count = corner_width.saturating_mul(height + 1);
        let mut regular = vec![[0.0f32; 3]; corner_count];
        let mut peaks = vec![[0.0f32; 3]; corner_count];
        for atom in sources {
            let Some(source) = atom.source else {
                continue;
            };

            let center_x = source.origin[0];
            let center_y = source.origin[1];
            let source_x = center_x.floor() as i32 + 1;
            let source_y = center_y.floor() as i32 + 1;
            if source.edge_only && !touches_dark_cell(&cells, width, height, source_x, source_y) {
                continue;
            }

            let bound = source.range.ceil().max(0.0) as i32 + 1;
            let mut affected = HashSet::new();
            for target_y in source_y.saturating_sub(bound)..=source_y.saturating_add(bound) {
                for target_x in source_x.saturating_sub(bound)..=source_x.saturating_add(bound) {
                    let Some(target) = cell_index(width, height, target_x, target_y) else {
                        continue;
                    };
                    if cells[target].blocked
                        || !line_of_sight(&cells, width, height, source_x, source_y, target_x, target_y)
                    {
                        continue;
                    }

                    let x = usize::try_from(target_x - 1).unwrap_or(0);
                    let y = usize::try_from(target_y - 1).unwrap_or(0);
                    for (corner_x, corner_y) in [(x, y), (x + 1, y), (x, y + 1), (x + 1, y + 1)] {
                        affected.insert(corner_y * corner_width + corner_x);
                    }
                }
            }

            for corner in affected {
                let corner_x = (corner % corner_width) as f32;
                let corner_y = (corner / corner_width) as f32;
                let strength = source.strength(corner_x - center_x, corner_y - center_y);
                if strength == 0.0 || !strength.is_finite() {
                    continue;
                }
                let contribution = source.color.map(|channel| channel * strength);
                if source.peak {
                    for (slot, value) in peaks[corner].iter_mut().zip(contribution) {
                        *slot = slot.max(value);
                    }
                } else {
                    for (slot, value) in regular[corner].iter_mut().zip(contribution) {
                        *slot += value;
                    }
                }
            }
        }

        let mut points = vec![[0.0f32; 3]; corner_count];
        for ((point, regular), peak) in points.iter_mut().zip(regular).zip(peaks) {
            *point = normalize_color([regular[0] + peak[0], regular[1] + peak[1], regular[2] + peak[2]]);
        }

        for y in 0..height {
            for x in 0..width {
                let cell = cells[y * width + x];
                let tile = if cell.fullbright {
                    LightTile { corners: [[1.0; 3]; 4] }
                } else {
                    let sample = |corner: usize| {
                        let point = match corner {
                            CORNER_SW => y * corner_width + x,
                            CORNER_SE => y * corner_width + x + 1,
                            CORNER_NW => (y + 1) * corner_width + x,
                            CORNER_NE => (y + 1) * corner_width + x + 1,
                            _ => unreachable!(),
                        };
                        [
                            (points[point][0] + cell.ambient[0]).clamp(0.0, 1.0),
                            (points[point][1] + cell.ambient[1]).clamp(0.0, 1.0),
                            (points[point][2] + cell.ambient[2]).clamp(0.0, 1.0),
                        ]
                    };
                    LightTile {
                        corners: [
                            sample(CORNER_SW),
                            sample(CORNER_SE),
                            sample(CORNER_NW),
                            sample(CORNER_NE),
                        ],
                    }
                };
                if let Some(slot) = self.tiles.get_mut(range.start + y * width + x) {
                    *slot = tile;
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct LightingAtom {
    pub position: Position,
    pub source: Option<LightSource>,
    pub blocks: bool,
    pub ambient: [f32; 3],
    pub fullbright: bool,
}

impl LightingAtom {
    pub(crate) fn affects_lighting(self) -> bool {
        self.source.is_some() || self.blocks || self.fullbright || self.ambient.iter().any(|value| *value != 0.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct LightSource {
    /// Zero-based map coordinates, where integer coordinates are tile corners.
    pub origin: [f32; 2],
    pub range: f32,
    pub inner_range: f32,
    pub power: f32,
    pub color: [f32; 3],
    pub angle: f32,
    pub direction: f32,
    pub height: f32,
    pub curve: f32,
    pub peak: bool,
    pub edge_only: bool,
    pub quadratic: f32,
    pub constant: f32,
}

impl LightSource {
    fn strength(self, dx: f32, dy: f32) -> f32 {
        let planar_squared = dx * dx + dy * dy;
        let distance_squared = (planar_squared + self.height).max(0.0);
        let distance = distance_squared.sqrt();
        if distance > self.range || !self.in_cone(dx, dy) {
            return 0.0;
        }

        let cone = self.cone_strength(dx, dy);
        if self.quadratic != 0.0 {
            let value = self.constant + self.quadratic / distance_squared.max(EPSILON);
            let limit = self.power.abs();
            return value.clamp(-limit, limit) * cone;
        }

        let span = (self.range - self.inner_range).max(EPSILON);
        let normalized = ((distance - self.inner_range) / span).clamp(0.0, 1.0);
        (1.0 - normalized).powf(self.curve.max(EPSILON)) * self.power * cone
    }

    fn in_cone(self, dx: f32, dy: f32) -> bool { self.cone_strength(dx, dy) > 0.0 }

    fn cone_strength(self, dx: f32, dy: f32) -> f32 {
        if self.angle >= 360.0 || self.angle <= 0.0 || (dx == 0.0 && dy == 0.0) {
            return 1.0;
        }
        let coordinate = dx.atan2(dy).to_degrees().rem_euclid(360.0);
        let mut delta = (self.direction - coordinate).abs();
        if delta > 180.0 {
            delta = 360.0 - delta;
        }
        (1.0 - (delta - self.angle * 0.5).max(0.0) / 30.0).max(0.0)
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct Cell {
    blocked: bool,
    fullbright: bool,
    ambient: [f32; 3],
}

fn tile_index(size: [u32; 3], position: Position) -> Option<usize> {
    let width = usize::try_from(size[0]).ok()?;
    let height = usize::try_from(size[1]).ok()?;
    let z = usize::try_from(position.z.checked_sub(1)?).ok()?;
    let y = usize::try_from(position.y.checked_sub(1)?).ok()?;
    let x = usize::try_from(position.x.checked_sub(1)?).ok()?;
    if x >= width || y >= height || z >= size[2] as usize {
        return None;
    }
    z.checked_mul(width.checked_mul(height)?)?
        .checked_add(y.checked_mul(width)?)?
        .checked_add(x)
}

fn cell_index(width: usize, height: usize, x: i32, y: i32) -> Option<usize> {
    let x = usize::try_from(x.checked_sub(1)?).ok()?;
    let y = usize::try_from(y.checked_sub(1)?).ok()?;
    (x < width && y < height).then_some(y * width + x)
}

fn touches_dark_cell(cells: &[Cell], width: usize, height: usize, x: i32, y: i32) -> bool {
    [(0, 1), (1, 0), (0, -1), (-1, 0)]
        .into_iter()
        .any(|(dx, dy)| cell_index(width, height, x + dx, y + dy).is_some_and(|index| !cells[index].fullbright))
}

fn line_of_sight(cells: &[Cell], width: usize, height: usize, from_x: i32, from_y: i32, to_x: i32, to_y: i32) -> bool {
    let dx = (to_x - from_x).abs();
    let dy = (to_y - from_y).abs();
    let sx = (to_x - from_x).signum();
    let sy = (to_y - from_y).signum();
    let mut x = from_x;
    let mut y = from_y;
    let mut error = dx - dy;

    while x != to_x || y != to_y {
        let doubled = error.saturating_mul(2);
        if doubled > -dy {
            error -= dy;
            x += sx;
        }
        if doubled < dx {
            error += dx;
            y += sy;
        }
        if x == to_x && y == to_y {
            break;
        }
        if cell_index(width, height, x, y).is_some_and(|index| cells[index].blocked) {
            return false;
        }
    }

    true
}

fn normalize_color(mut color: [f32; 3]) -> [f32; 3] {
    let largest = color.iter().copied().fold(0.0f32, f32::max);
    if largest > 1.0 {
        for channel in &mut color {
            *channel /= largest;
        }
    }
    color.map(|channel| channel.max(0.0))
}

pub(crate) fn parse_color(text: Option<&str>) -> [f32; 3] {
    let Some(text) = text.map(str::trim) else {
        return [1.0; 3];
    };
    let Some(hex) = text.strip_prefix('#') else {
        return match text.to_ascii_lowercase().as_str() {
            "black" => [0.0, 0.0, 0.0],
            "red" => [1.0, 0.0, 0.0],
            "green" => [0.0, 0.75, 0.0],
            "lime" => [0.0, 1.0, 0.0],
            "blue" => [0.0, 0.0, 1.0],
            "yellow" => [1.0, 1.0, 0.0],
            "cyan" | "aqua" => [0.0, 1.0, 1.0],
            "magenta" | "fuchsia" => [1.0, 0.0, 1.0],
            "white" => [1.0; 3],
            _ => [1.0; 3],
        };
    };
    let bytes = match hex.len() {
        3 | 4 => {
            let mut out = [255u8; 4];
            for (slot, digit) in out.iter_mut().zip(hex.chars()) {
                let Some(value) = digit.to_digit(16) else {
                    return [1.0; 3];
                };
                *slot = value as u8 * 17;
            }
            out
        },
        6 | 8 => {
            let mut out = [255u8; 4];
            for (slot, pair) in out.iter_mut().zip(hex.as_bytes().chunks_exact(2)) {
                let Ok(pair) = std::str::from_utf8(pair) else {
                    return [1.0; 3];
                };
                let Ok(value) = u8::from_str_radix(pair, 16) else {
                    return [1.0; 3];
                };
                *slot = value;
            }
            out
        },
        _ => return [1.0; 3],
    };
    let alpha = f32::from(bytes[3]) / 255.0;
    [
        f32::from(bytes[0]) / 255.0 * alpha,
        f32::from(bytes[1]) / 255.0 * alpha,
        f32::from(bytes[2]) / 255.0 * alpha,
    ]
}

pub(crate) fn direction_angle(direction: f32) -> f32 {
    match direction as i32 {
        1 => 0.0,
        5 => 45.0,
        4 => 90.0,
        6 => 135.0,
        2 => 180.0,
        10 => 225.0,
        8 => 270.0,
        9 => 315.0,
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(position: Position) -> LightingAtom {
        LightingAtom {
            position,
            source: Some(LightSource {
                origin: [position.x as f32 - 0.5, position.y as f32 - 0.5],
                range: 3.0,
                inner_range: 0.0,
                power: 1.0,
                color: [1.0; 3],
                angle: 360.0,
                direction: 0.0,
                height: 0.0,
                curve: 1.0,
                peak: false,
                edge_only: false,
                quadratic: 0.0,
                constant: 0.0,
            }),
            blocks: false,
            ambient: [0.0; 3],
            fullbright: false,
        }
    }

    #[test]
    fn radial_light_falls_off_across_shared_corners() {
        let atoms = [source(Position::new(2, 2, 1))];
        let mut map = LightingMap::new([3, 3, 1]);
        let levels = HashSet::from([1]);
        map.solve_levels(atoms.iter(), &levels);

        let center = map.tile(Position::new(2, 2, 1)).unwrap();
        let edge = map.tile(Position::new(3, 3, 1)).unwrap();
        assert!(center.corners[CORNER_SW][0] > edge.corners[CORNER_NE][0]);
        assert_eq!(center.corners[CORNER_NE], edge.corners[CORNER_SW]);
    }

    #[test]
    fn blockers_shadow_cells_behind_them() {
        let atoms = [
            source(Position::new(1, 2, 1)),
            LightingAtom {
                position: Position::new(2, 2, 1),
                source: None,
                blocks: true,
                ambient: [0.0; 3],
                fullbright: false,
            },
        ];
        let mut map = LightingMap::new([4, 3, 1]);
        map.solve_levels(atoms.iter(), &HashSet::from([1]));

        let shadow = map.tile(Position::new(3, 2, 1)).unwrap();
        assert!(shadow.corners[CORNER_SW][0] > 0.0);
        assert_eq!(shadow.corners[CORNER_SE], [0.0; 3]);
        assert_eq!(shadow.corners[CORNER_NE], [0.0; 3]);
    }

    #[test]
    fn peak_sources_do_not_sum_with_each_other() {
        let mut left = source(Position::new(1, 1, 1));
        left.source.as_mut().unwrap().peak = true;
        let mut right = source(Position::new(2, 1, 1));
        right.source.as_mut().unwrap().peak = true;
        let mut one = LightingMap::new([2, 1, 1]);
        one.solve_levels([&left].into_iter(), &HashSet::from([1]));
        let mut both = LightingMap::new([2, 1, 1]);
        both.solve_levels([&left, &right].into_iter(), &HashSet::from([1]));

        assert!(both.tile(Position::new(1, 1, 1)).unwrap().corners[CORNER_SW][0] <= 1.0);
        assert!(
            both.tile(Position::new(1, 1, 1)).unwrap().corners[CORNER_SW][0]
                >= one.tile(Position::new(1, 1, 1)).unwrap().corners[CORNER_SW][0]
        );
    }

    #[test]
    fn fullbright_and_ambient_are_cell_local() {
        let atoms = [
            LightingAtom {
                position: Position::new(1, 1, 1),
                source: None,
                blocks: false,
                ambient: [0.1, 0.2, 0.3],
                fullbright: false,
            },
            LightingAtom {
                position: Position::new(2, 1, 1),
                source: None,
                blocks: false,
                ambient: [0.0; 3],
                fullbright: true,
            },
        ];
        let mut map = LightingMap::new([2, 1, 1]);
        map.solve_levels(atoms.iter(), &HashSet::from([1]));

        assert_eq!(map.tile(Position::new(1, 1, 1)).unwrap().corners[0], [0.1, 0.2, 0.3]);
        assert_eq!(map.tile(Position::new(2, 1, 1)).unwrap().corners, [[1.0; 3]; 4]);
    }

    #[test]
    fn colors_parse_alpha_and_byond_directions() {
        assert_eq!(parse_color(Some("#ff000080")), [128.0 / 255.0, 0.0, 0.0]);
        assert_eq!(direction_angle(5.0), 45.0);
        assert_eq!(direction_angle(8.0), 270.0);
    }
}
