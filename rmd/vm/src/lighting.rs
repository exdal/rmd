use std::{collections::HashMap, ops::Range};

use crate::world::Position;

const CORNER_SW: usize = 0;
const CORNER_SE: usize = 1;
const CORNER_NW: usize = 2;
const CORNER_NE: usize = 3;
const EPSILON: f32 = 1.0e-6;
const BUCKET: usize = 16;
const MAX_REGIONS: usize = 16;

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
///
/// An edit re-solves only the corners its atoms can reach. Sources are summed in id order, which
/// keeps that bit for bit equal to solving the whole level again.
#[derive(Debug, Clone)]
pub struct LightingMap {
    pub size: [u32; 3],
    pub tiles: Vec<LightTile>,
    atoms: HashMap<u64, LightingAtom>,
    cells: Vec<Cell>,
    points: Vec<[f32; 3]>,
    buckets: Vec<Vec<u64>>,
    reach: Vec<i32>,
    ambient: HashMap<usize, Vec<u64>>,
    dirty: Vec<Vec<Rect>>,
}

impl PartialEq for LightingMap {
    fn eq(&self, other: &Self) -> bool { self.size == other.size && self.tiles == other.tiles }
}

impl LightingMap {
    fn new(size: [i32; 3]) -> Self {
        let size = size.map(|value| u32::try_from(value.max(0)).unwrap_or(0));
        let [width, height, levels] = size.map(|value| value as usize);
        let lengths = width.checked_mul(height).and_then(|plane| {
            let corners = width.checked_add(1)?.checked_mul(height.checked_add(1)?)?;

            Some((plane.checked_mul(levels)?, corners.checked_mul(levels)?))
        });
        let Some((tiles, points)) = lengths else {
            return Self::new([0; 3]);
        };
        let buckets = width
            .div_ceil(BUCKET)
            .saturating_mul(height.div_ceil(BUCKET))
            .saturating_mul(levels);

        Self {
            size,
            tiles: vec![LightTile::default(); tiles],
            atoms: HashMap::new(),
            cells: vec![Cell::default(); tiles],
            points: vec![[0.0; 3]; points],
            buckets: vec![Vec::new(); buckets],
            reach: vec![0; levels],
            ambient: HashMap::new(),
            dirty: vec![Vec::new(); levels],
        }
    }

    pub(crate) fn build(size: [i32; 3], atoms: impl IntoIterator<Item = (u64, LightingAtom)>) -> Self {
        let mut map = Self::new(size);
        for (id, atom) in atoms {
            map.replace(id, Some(atom), false);
        }

        for level in 0..map.reach.len() {
            map.solve(level, map.corner_bounds());
        }

        map
    }

    pub fn level_range(&self, z: u32) -> Option<Range<usize>> {
        if z == 0 || z > self.size[2] {
            return None;
        }

        let start = usize::try_from(z - 1).ok()?.checked_mul(self.plane())?;

        Some(start..start.checked_add(self.plane())?)
    }

    pub fn tile(&self, position: Position) -> Option<&LightTile> {
        tile_index(self.size, position).and_then(|index| self.tiles.get(index))
    }

    pub(crate) fn set(&mut self, id: u64, atom: Option<LightingAtom>) { self.replace(id, atom, true); }

    pub(crate) fn solve_dirty(&mut self) -> Option<Range<usize>> {
        let mut changed: Option<Range<usize>> = None;
        for level in 0..self.dirty.len() {
            let Some(regions) = self.dirty.get_mut(level).map(std::mem::take) else {
                continue;
            };

            for region in merge_regions(regions) {
                if let Some(range) = self.solve(level, region) {
                    changed = Some(match changed {
                        Some(current) => current.start.min(range.start)..current.end.max(range.end),
                        None => range,
                    });
                }
            }
        }

        changed
    }

    fn width(&self) -> usize { self.size[0] as usize }

    fn height(&self) -> usize { self.size[1] as usize }

    fn plane(&self) -> usize { self.width() * self.height() }

    fn corner_bounds(&self) -> Rect {
        Rect {
            x0: 0,
            y0: 0,
            x1: i32::try_from(self.size[0]).unwrap_or(i32::MAX),
            y1: i32::try_from(self.size[1]).unwrap_or(i32::MAX),
        }
    }

    fn cell_bounds(&self) -> Rect {
        Rect {
            x0: 1,
            y0: 1,
            ..self.corner_bounds()
        }
    }

    fn cell_index(&self, level: usize, x: i32, y: i32) -> Option<usize> {
        let x = usize::try_from(x.checked_sub(1)?).ok()?;
        let y = usize::try_from(y.checked_sub(1)?).ok()?;

        (x < self.width() && y < self.height() && level < self.reach.len())
            .then(|| level * self.plane() + y * self.width() + x)
    }

    fn cell(&self, level: usize, x: i32, y: i32) -> Option<&Cell> { self.cells.get(self.cell_index(level, x, y)?) }

    fn point(&self, level: usize, x: i32, y: i32) -> Option<[f32; 3]> {
        let x = usize::try_from(x).ok()?;
        let y = usize::try_from(y).ok()?;
        let row = self.width() + 1;
        if x >= row || y > self.height() {
            return None;
        }

        let index = level.checked_mul(row * (self.height() + 1))?.checked_add(y * row + x)?;

        self.points.get(index).copied()
    }

    /// Origins pushed off the map by pixel offsets clamp into the nearest edge bucket.
    fn bucket(&self, level: usize, x: i32, y: i32) -> Option<usize> {
        let columns = self.width().div_ceil(BUCKET);
        let rows = self.height().div_ceil(BUCKET);

        (columns > 0 && rows > 0 && level < self.reach.len())
            .then(|| level * columns * rows + bucket_of(y).min(rows - 1) * columns + bucket_of(x).min(columns - 1))
    }

    /// Ascending, which is the order a full solve sums them in.
    fn sources_near(&self, level: usize, cells: Rect) -> Vec<u64> {
        let reach = self.reach.get(level).copied().unwrap_or(0);
        let (Some(area), Some(first)) = (
            cells.expand(reach).intersect(self.cell_bounds()),
            self.bucket(level, 1, 1),
        ) else {
            return Vec::new();
        };

        let columns = self.width().div_ceil(BUCKET);
        let mut ids = Vec::new();
        for row in bucket_of(area.y0)..=bucket_of(area.y1) {
            let start = first + row * columns;
            let buckets = self
                .buckets
                .get(start + bucket_of(area.x0)..=start + bucket_of(area.x1))
                .unwrap_or_default();
            for bucket in buckets {
                ids.extend_from_slice(bucket);
            }
        }

        ids.sort_unstable();
        ids
    }

    fn replace(&mut self, id: u64, atom: Option<LightingAtom>, track: bool) {
        if let Some(old) = self.atoms.remove(&id) {
            self.apply(id, old, false, track);
        }

        if let Some(atom) = atom.filter(|atom| atom.affects_lighting()) {
            self.atoms.insert(id, atom);
            self.apply(id, atom, true, track);
        }
    }

    fn apply(&mut self, id: u64, atom: LightingAtom, attach: bool, track: bool) {
        let Some(cell) = tile_index(self.size, atom.position) else {
            return;
        };
        let Some(level) = cell.checked_div(self.plane()) else {
            return;
        };
        let Position { x, y, .. } = atom.position;

        if let Some(source) = atom.source {
            let (source_x, source_y, bound) = source.footprint();
            if let Some(bucket) = self
                .bucket(level, source_x, source_y)
                .and_then(|bucket| self.buckets.get_mut(bucket))
            {
                match bucket.binary_search(&id) {
                    Ok(index) if !attach => {
                        bucket.remove(index);
                    },
                    Err(index) if attach => bucket.insert(index, id),
                    _ => {},
                }
            }

            if attach && let Some(reach) = self.reach.get_mut(level) {
                *reach = (*reach).max(bound);
            }

            if track {
                self.mark(level, Rect::reach(source_x, source_y, bound).corners());
            }
        }

        if atom.blocks
            && let Some(state) = self.cells.get_mut(cell)
        {
            let before = state.blocked();
            state.blockers = step(state.blockers, attach);
            if track && before != state.blocked() {
                self.mark_shadows(level, x, y);
            }
        }

        if atom.fullbright
            && let Some(state) = self.cells.get_mut(cell)
        {
            let before = state.fullbright > 0;
            state.fullbright = step(state.fullbright, attach);
            if track && before != (state.fullbright > 0) {
                self.mark(level, Rect::cell(x, y).corners());
                self.mark_edges(level, x, y);
            }
        }

        if atom.ambient != [0.0; 3] {
            let members = self.ambient.entry(cell).or_default();
            match members.binary_search(&id) {
                Ok(index) if !attach => {
                    members.remove(index);
                },
                Err(index) if attach => members.insert(index, id),
                _ => {},
            }

            let mut ambient = [0.0f32; 3];
            for member in members.iter().filter_map(|member| self.atoms.get(member)) {
                for (slot, value) in ambient.iter_mut().zip(member.ambient) {
                    *slot += value;
                }
            }

            if members.is_empty() {
                self.ambient.remove(&cell);
            }

            if let Some(state) = self.cells.get_mut(cell) {
                state.ambient = ambient;
            }

            if track {
                self.mark(level, Rect::cell(x, y).corners());
            }
        }
    }

    fn mark(&mut self, level: usize, corners: Rect) {
        let Some(corners) = corners.intersect(self.corner_bounds()) else {
            return;
        };

        if let Some(regions) = self.dirty.get_mut(level) {
            regions.push(corners);
        }
    }

    /// A blocker at `(x, y)` shades whatever lies behind it for every source that reaches it.
    fn mark_shadows(&mut self, level: usize, x: i32, y: i32) {
        for id in self.sources_near(level, Rect::cell(x, y)) {
            let Some(source) = self.atoms.get(&id).and_then(|atom| atom.source) else {
                continue;
            };

            let (source_x, source_y, bound) = source.footprint();
            if Rect::reach(source_x, source_y, bound).contains(x, y) {
                self.mark(level, Rect::reach(source_x, source_y, bound).corners());
            }
        }
    }

    /// `demir_light_edge_only` sources beside `(x, y)` depend on whether it is fullbright.
    fn mark_edges(&mut self, level: usize, x: i32, y: i32) {
        for id in self.sources_near(level, Rect::cell(x, y)) {
            let Some(source) = self.atoms.get(&id).and_then(|atom| atom.source) else {
                continue;
            };

            let (source_x, source_y, bound) = source.footprint();
            if source.edge_only && source_x.abs_diff(x) + source_y.abs_diff(y) == 1 {
                self.mark(level, Rect::reach(source_x, source_y, bound).corners());
            }
        }
    }

    fn solve(&mut self, level: usize, corners: Rect) -> Option<Range<usize>> {
        let corners = corners.intersect(self.corner_bounds())?;
        let targets = corners.cells().intersect(self.cell_bounds())?;
        let span = usize::try_from(corners.x1 - corners.x0).ok()? + 1;
        let rows = usize::try_from(corners.y1 - corners.y0).ok()? + 1;
        let count = span.checked_mul(rows)?;
        let local = |x: i32, y: i32| {
            corners.contains(x, y).then(|| {
                usize::try_from(y - corners.y0).unwrap_or(0) * span + usize::try_from(x - corners.x0).unwrap_or(0)
            })
        };

        let mut regular = vec![[0.0f32; 3]; count];
        let mut peaks = vec![[0.0f32; 3]; count];
        let mut stamps = vec![0usize; count];
        for (stamp, id) in self.sources_near(level, targets).into_iter().enumerate() {
            let stamp = stamp + 1;
            let Some(source) = self.atoms.get(&id).and_then(|atom| atom.source) else {
                continue;
            };

            let (source_x, source_y, bound) = source.footprint();
            let Some(area) = Rect::reach(source_x, source_y, bound).intersect(targets) else {
                continue;
            };

            if source.edge_only && !self.touches_dark_cell(level, source_x, source_y) {
                continue;
            }

            for target_y in area.y0..=area.y1 {
                for target_x in area.x0..=area.x1 {
                    if self.cell(level, target_x, target_y).is_none_or(Cell::blocked)
                        || !self.line_of_sight(level, source_x, source_y, target_x, target_y)
                    {
                        continue;
                    }

                    let neighbors = [
                        (target_x - 1, target_y - 1),
                        (target_x, target_y - 1),
                        (target_x - 1, target_y),
                        (target_x, target_y),
                    ];
                    for (corner_x, corner_y) in neighbors {
                        let Some(index) = local(corner_x, corner_y) else {
                            continue;
                        };
                        let (Some(seen), Some(regular), Some(peak)) =
                            (stamps.get_mut(index), regular.get_mut(index), peaks.get_mut(index))
                        else {
                            continue;
                        };

                        if *seen == stamp {
                            continue;
                        }

                        *seen = stamp;
                        let strength =
                            source.strength(corner_x as f32 - source.origin[0], corner_y as f32 - source.origin[1]);
                        if strength == 0.0 || !strength.is_finite() {
                            continue;
                        }

                        let contribution = source.color.map(|channel| channel * strength);
                        if source.peak {
                            for (slot, value) in peak.iter_mut().zip(contribution) {
                                *slot = slot.max(value);
                            }
                        } else {
                            for (slot, value) in regular.iter_mut().zip(contribution) {
                                *slot += value;
                            }
                        }
                    }
                }
            }
        }

        let row = self.width() + 1;
        let base = level.checked_mul(row * (self.height() + 1))?;
        let first = usize::try_from(corners.y0).ok()? * row + usize::try_from(corners.x0).ok()?;
        for line in 0..rows {
            for column in 0..span {
                let index = line * span + column;
                let (Some(regular), Some(peak)) = (regular.get(index), peaks.get(index)) else {
                    continue;
                };

                if let Some(slot) = self.points.get_mut(base + first + line * row + column) {
                    *slot = normalize_color([regular[0] + peak[0], regular[1] + peak[1], regular[2] + peak[2]]);
                }
            }
        }

        for y in targets.y0..=targets.y1 {
            for x in targets.x0..=targets.x1 {
                let tile = self.sample(level, x, y);
                if let Some(slot) = self.cell_index(level, x, y).and_then(|index| self.tiles.get_mut(index)) {
                    *slot = tile;
                }
            }
        }

        let start = self.cell_index(level, targets.x0, targets.y0)?;
        let end = self.cell_index(level, targets.x1, targets.y1)? + 1;

        Some(start..end)
    }

    fn sample(&self, level: usize, x: i32, y: i32) -> LightTile {
        let cell = self.cell(level, x, y).copied().unwrap_or_default();
        if cell.fullbright > 0 {
            return LightTile { corners: [[1.0; 3]; 4] };
        }

        let corner = |corner_x: i32, corner_y: i32| {
            let point = self.point(level, corner_x, corner_y).unwrap_or_default();
            [
                (point[0] + cell.ambient[0]).clamp(0.0, 1.0),
                (point[1] + cell.ambient[1]).clamp(0.0, 1.0),
                (point[2] + cell.ambient[2]).clamp(0.0, 1.0),
            ]
        };

        let mut corners = [[0.0; 3]; 4];
        corners[CORNER_SW] = corner(x - 1, y - 1);
        corners[CORNER_SE] = corner(x, y - 1);
        corners[CORNER_NW] = corner(x - 1, y);
        corners[CORNER_NE] = corner(x, y);

        LightTile { corners }
    }

    fn touches_dark_cell(&self, level: usize, x: i32, y: i32) -> bool {
        [(0, 1), (1, 0), (0, -1), (-1, 0)].into_iter().any(|(dx, dy)| {
            self.cell(level, x.saturating_add(dx), y.saturating_add(dy))
                .is_some_and(|cell| cell.fullbright == 0)
        })
    }

    fn line_of_sight(&self, level: usize, from_x: i32, from_y: i32, to_x: i32, to_y: i32) -> bool {
        let (from_x, from_y, to_x, to_y) = (i64::from(from_x), i64::from(from_y), i64::from(to_x), i64::from(to_y));
        let dx = (to_x - from_x).abs();
        let dy = (to_y - from_y).abs();
        let sx = (to_x - from_x).signum();
        let sy = (to_y - from_y).signum();
        let mut x = from_x;
        let mut y = from_y;
        let mut error = dx - dy;

        while x != to_x || y != to_y {
            let doubled = error * 2;
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

            let blocked = i32::try_from(x)
                .ok()
                .zip(i32::try_from(y).ok())
                .and_then(|(x, y)| self.cell(level, x, y))
                .is_some_and(Cell::blocked);

            if blocked {
                return false;
            }
        }

        true
    }
}

fn bucket_of(coordinate: i32) -> usize { usize::try_from(coordinate.saturating_sub(1)).unwrap_or(0) / BUCKET }

fn step(count: u32, attach: bool) -> u32 {
    if attach {
        count.saturating_add(1)
    } else {
        count.saturating_sub(1)
    }
}

fn merge_regions(mut regions: Vec<Rect>) -> Vec<Rect> {
    let mut merged: Vec<Rect> = Vec::with_capacity(regions.len());
    while let Some(mut region) = regions.pop() {
        while let Some(index) = merged.iter().position(|other| other.intersect(region).is_some()) {
            region = region.union(merged.swap_remove(index));
        }
        merged.push(region);
    }

    if merged.len() > MAX_REGIONS {
        let first = merged.first().copied();
        return first
            .map(|first| vec![merged.iter().fold(first, |bounds, region| bounds.union(*region))])
            .unwrap_or_default();
    }

    merged
}

/// Inclusive bounds, in one-based cells or zero-based corners depending on the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rect {
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
}

impl Rect {
    fn cell(x: i32, y: i32) -> Self {
        Self {
            x0: x,
            y0: y,
            x1: x,
            y1: y,
        }
    }

    fn reach(x: i32, y: i32, bound: i32) -> Self { Self::cell(x, y).expand(bound) }

    fn expand(self, by: i32) -> Self {
        Self {
            x0: self.x0.saturating_sub(by),
            y0: self.y0.saturating_sub(by),
            x1: self.x1.saturating_add(by),
            y1: self.y1.saturating_add(by),
        }
    }

    /// The corners of these cells.
    fn corners(self) -> Self {
        Self {
            x0: self.x0.saturating_sub(1),
            y0: self.y0.saturating_sub(1),
            ..self
        }
    }

    /// The cells touching these corners.
    fn cells(self) -> Self {
        Self {
            x1: self.x1.saturating_add(1),
            y1: self.y1.saturating_add(1),
            ..self
        }
    }

    fn contains(self, x: i32, y: i32) -> bool { (self.x0..=self.x1).contains(&x) && (self.y0..=self.y1).contains(&y) }

    fn intersect(self, other: Self) -> Option<Self> {
        let rect = Self {
            x0: self.x0.max(other.x0),
            y0: self.y0.max(other.y0),
            x1: self.x1.min(other.x1),
            y1: self.y1.min(other.y1),
        };

        (rect.x0 <= rect.x1 && rect.y0 <= rect.y1).then_some(rect)
    }

    fn union(self, other: Self) -> Self {
        Self {
            x0: self.x0.min(other.x0),
            y0: self.y0.min(other.y0),
            x1: self.x1.max(other.x1),
            y1: self.y1.max(other.y1),
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

    /// The one-based cell holding the origin, and how many cells past it the light can land.
    fn footprint(self) -> (i32, i32, i32) {
        (
            (self.origin[0].floor() as i32).saturating_add(1),
            (self.origin[1].floor() as i32).saturating_add(1),
            (self.range.ceil().max(0.0) as i32).saturating_add(1),
        )
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
    blockers: u32,
    fullbright: u32,
    ambient: [f32; 3],
}

impl Cell {
    fn blocked(&self) -> bool { self.blockers > 0 }
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

    fn solved(size: [i32; 3], atoms: &[LightingAtom]) -> LightingMap {
        LightingMap::build(size, (0u64..).zip(atoms.iter().copied()))
    }

    fn blocker(position: Position) -> LightingAtom {
        LightingAtom {
            position,
            source: None,
            blocks: true,
            ambient: [0.0; 3],
            fullbright: false,
        }
    }

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
        let map = solved([3, 3, 1], &atoms);

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
        let map = solved([4, 3, 1], &atoms);

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
        let one = solved([2, 1, 1], &[left]);
        let both = solved([2, 1, 1], &[left, right]);

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
        let map = solved([2, 1, 1], &atoms);

        assert_eq!(map.tile(Position::new(1, 1, 1)).unwrap().corners[0], [0.1, 0.2, 0.3]);
        assert_eq!(map.tile(Position::new(2, 1, 1)).unwrap().corners, [[1.0; 3]; 4]);
    }

    #[test]
    fn colors_parse_alpha_and_byond_directions() {
        assert_eq!(parse_color(Some("#ff000080")), [128.0 / 255.0, 0.0, 0.0]);
        assert_eq!(direction_angle(5.0), 45.0);
        assert_eq!(direction_angle(8.0), 270.0);
    }

    fn assert_edit_matches_full_solve(
        map: &mut LightingMap, atoms: &mut std::collections::BTreeMap<u64, LightingAtom>, id: u64,
        atom: Option<LightingAtom>,
    ) {
        let before = map.tiles.clone();
        match atom {
            Some(atom) => atoms.insert(id, atom),
            None => atoms.remove(&id),
        };
        map.set(id, atom);
        let changed = map.solve_dirty().unwrap_or(0..0);

        let expected = LightingMap::build(
            [map.size[0] as i32, map.size[1] as i32, map.size[2] as i32],
            atoms.iter().map(|(id, atom)| (*id, *atom)),
        );
        assert!(map.tiles == expected.tiles, "edit to {id} diverged from a full solve");
        for (index, (before, after)) in before.iter().zip(&map.tiles).enumerate() {
            assert!(
                changed.contains(&index) || before == after,
                "tile {index} changed outside {changed:?}"
            );
        }
    }

    #[test]
    fn incremental_edits_match_a_full_solve() {
        let mut atoms = std::collections::BTreeMap::new();
        let mut next = 0u64;
        let mut add = |atoms: &mut std::collections::BTreeMap<u64, LightingAtom>, atom| {
            next += 1;
            atoms.insert(next, atom);
            next
        };

        for z in 1..=2 {
            for y in 1..=30 {
                add(&mut atoms, blocker(Position::new(20, y, z)));
            }
            for x in 22..=40 {
                for y in 1..=30 {
                    let mut sun = source(Position::new(x, y, z));
                    if let Some(light) = sun.source.as_mut() {
                        light.peak = true;
                        light.height = -0.5;
                        light.color = [0.9, 0.8, 0.7];
                    }
                    add(&mut atoms, sun);
                }
            }
        }
        let lamp = add(&mut atoms, source(Position::new(10, 10, 1)));
        let mut edge = source(Position::new(5, 25, 1));
        if let Some(light) = edge.source.as_mut() {
            light.edge_only = true;
            light.range = 5.0;
        }
        add(&mut atoms, edge);
        for (x, y) in [(5, 26), (5, 24), (4, 25)] {
            add(
                &mut atoms,
                LightingAtom {
                    blocks: false,
                    fullbright: true,
                    ..blocker(Position::new(x, y, 1))
                },
            );
        }
        let ambient = add(
            &mut atoms,
            LightingAtom {
                blocks: false,
                ambient: [0.1, 0.0, 0.2],
                ..blocker(Position::new(12, 12, 1))
            },
        );

        let mut map = LightingMap::build([40, 30, 2], atoms.iter().map(|(id, atom)| (*id, *atom)));
        let mut moved = source(Position::new(11, 10, 1));
        assert_edit_matches_full_solve(&mut map, &mut atoms, lamp, Some(moved));

        if let Some(light) = moved.source.as_mut() {
            light.origin[0] += 0.5;
            light.range = 7.0;
            light.color = [0.2, 0.4, 1.0];
        }
        assert_edit_matches_full_solve(&mut map, &mut atoms, lamp, Some(moved));
        assert_edit_matches_full_solve(&mut map, &mut atoms, lamp, Some(source(Position::new(19, 15, 1))));

        let wall = next + 1;
        assert_edit_matches_full_solve(&mut map, &mut atoms, wall, Some(blocker(Position::new(17, 15, 1))));
        assert_edit_matches_full_solve(&mut map, &mut atoms, wall, Some(blocker(Position::new(20, 5, 2))));
        assert_edit_matches_full_solve(&mut map, &mut atoms, 1, None);
        assert_edit_matches_full_solve(&mut map, &mut atoms, ambient, None);
        assert_edit_matches_full_solve(
            &mut map,
            &mut atoms,
            next + 2,
            Some(LightingAtom {
                blocks: false,
                fullbright: true,
                ..blocker(Position::new(6, 25, 1))
            }),
        );
        assert_edit_matches_full_solve(&mut map, &mut atoms, lamp, None);
    }

    #[test]
    fn moving_a_light_rewrites_only_its_neighborhood() {
        let mut map = solved([200, 200, 1], &[source(Position::new(100, 100, 1))]);
        map.set(0, Some(source(Position::new(101, 100, 1))));
        let changed = map.solve_dirty().expect("the move relights tiles");

        assert!(changed.len() < 200 * 12, "{changed:?}");
        assert_eq!(map, solved([200, 200, 1], &[source(Position::new(101, 100, 1))]));
    }
}
