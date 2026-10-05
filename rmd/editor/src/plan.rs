use std::{
    fmt,
    fs,
    io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use serde::{Deserialize, Serialize};

use crate::icons::materialdesignicons::{ICON_CURSOR_DEFAULT, ICON_ERASER, ICON_FORMAT_TEXT, ICON_RECTANGLE_OUTLINE};

pub const DEFAULT_PLAN_SIZE: u32 = 255;
pub const MAX_PLAN_SIZE: u32 = 1024;
pub const PLAN_EXTENSION: &str = "rmdp";
pub const DEFAULT_LABEL_SIZE: f32 = 4.0;
pub const LABEL_SIZE_RANGE: (f32, f32) = (1.0, 32.0);
const PLAN_VERSION: u32 = 1;
const HISTORY_LIMIT: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlanId(u64);

impl PlanId {
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);

        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }

    pub const fn get(self) -> u64 { self.0 }
}

impl Default for PlanId {
    fn default() -> Self { Self::new() }
}

/// 0 based and starts from bottom left
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Plan {
    pub version: u32,
    pub width: u32,
    pub height: u32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub rects: Vec<PlanRect>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<PlanLabel>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanRect {
    pub area: String,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// these however anchored from top left because we believe in sane coordinate origin
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanLabel {
    pub text: String,
    pub x: u32,
    pub y: u32,
    pub text_size: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlanTool {
    #[default]
    Select,
    Rectangle,
    Erase,
    Text,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handle {
    North,
    South,
    East,
    West,
    NorthEast,
    NorthWest,
    SouthEast,
    SouthWest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanElement {
    Rect(usize),
    Label(usize),
}

#[derive(Debug)]
pub enum PlanError {
    Io(io::Error),
    Parse(toml::de::Error),
    Write(toml::ser::Error),
    NoPath,
}

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Parse(e) => write!(f, "could not read the plan: {e}"),
            Self::Write(e) => write!(f, "could not write the plan: {e}"),
            Self::NoPath => write!(f, "the plan has no file yet"),
        }
    }
}

impl std::error::Error for PlanError {}

impl From<io::Error> for PlanError {
    fn from(e: io::Error) -> Self { Self::Io(e) }
}

impl PlanTool {
    pub const ALL: [Self; 4] = [Self::Select, Self::Rectangle, Self::Erase, Self::Text];

    pub fn label(self) -> &'static str {
        match self {
            Self::Select => "Select",
            Self::Rectangle => "Rectangle",
            Self::Erase => "Erase",
            Self::Text => "Text",
        }
    }

    pub fn icon(self) -> char {
        match self {
            Self::Select => ICON_CURSOR_DEFAULT,
            Self::Rectangle => ICON_RECTANGLE_OUTLINE,
            Self::Erase => ICON_ERASER,
            Self::Text => ICON_FORMAT_TEXT,
        }
    }
}

impl Handle {
    pub const ALL: [Self; 8] = [
        Self::North,
        Self::South,
        Self::East,
        Self::West,
        Self::NorthEast,
        Self::NorthWest,
        Self::SouthEast,
        Self::SouthWest,
    ];

    const fn is_north(self) -> bool { matches!(self, Self::North | Self::NorthEast | Self::NorthWest) }

    const fn is_south(self) -> bool { matches!(self, Self::South | Self::SouthEast | Self::SouthWest) }

    const fn is_east(self) -> bool { matches!(self, Self::East | Self::NorthEast | Self::SouthEast) }

    const fn is_west(self) -> bool { matches!(self, Self::West | Self::NorthWest | Self::SouthWest) }

    pub const fn anchor(self) -> [f32; 2] {
        let x = if self.is_east() {
            1.0
        } else if self.is_west() {
            0.0
        } else {
            0.5
        };
        let y = if self.is_north() {
            1.0
        } else if self.is_south() {
            0.0
        } else {
            0.5
        };

        [x, y]
    }
}

impl PlanRect {
    pub fn from_tiles(area: impl Into<String>, a: [u32; 2], b: [u32; 2]) -> Self {
        Self {
            area: area.into(),
            x: a[0].min(b[0]),
            y: a[1].min(b[1]),
            width: a[0].abs_diff(b[0]) + 1,
            height: a[1].abs_diff(b[1]) + 1,
        }
    }

    pub const fn right(&self) -> u32 { self.x + self.width }

    pub const fn top(&self) -> u32 { self.y + self.height }

    pub const fn corners(&self) -> [[u32; 2]; 4] {
        let right = self.right() - 1;
        let top = self.top() - 1;

        // bottom left      bottom right     top left       top right
        [[self.x, self.y], [right, self.y], [self.x, top], [right, top]]
    }

    /// `[left, bottom, right, top]` with the right and top edges exclusive
    const fn span(&self) -> [i64; 4] { [self.x as i64, self.y as i64, self.right() as i64, self.top() as i64] }

    fn with_span(&self, span: [i64; 4]) -> Self {
        Self {
            x: span[0] as u32,
            y: span[1] as u32,
            width: (span[2] - span[0]) as u32,
            height: (span[3] - span[1]) as u32,
            ..self.clone()
        }
    }

    pub const fn overlaps(&self, other: &PlanRect) -> bool { spans_overlap(self.span(), other.span()) }

    pub fn contains(&self, point: [f32; 2]) -> bool {
        point[0] >= self.x as f32
            && point[0] < self.right() as f32
            && point[1] >= self.y as f32
            && point[1] < self.top() as f32
    }

    pub fn moved(&self, delta: [i64; 2], canvas: [u32; 2]) -> Self {
        let shift = |start: u32, length: u32, delta: i64, limit: u32| {
            (i64::from(start) + delta).clamp(0, i64::from(limit.saturating_sub(length))) as u32
        };

        Self {
            x: shift(self.x, self.width, delta[0], canvas[0]),
            y: shift(self.y, self.height, delta[1], canvas[1]),
            ..self.clone()
        }
    }

    /// never flips the rect or shrinks it below one tile
    pub fn resized(&self, handle: Handle, boundary: [i64; 2], canvas: [u32; 2]) -> Self {
        let (mut left, mut right) = (i64::from(self.x), i64::from(self.right()));
        let (mut bottom, mut top) = (i64::from(self.y), i64::from(self.top()));

        if handle.is_east() {
            right = boundary[0].clamp(left + 1, i64::from(canvas[0]).max(left + 1));
        } else if handle.is_west() {
            left = boundary[0].clamp(0, right - 1);
        }

        if handle.is_north() {
            top = boundary[1].clamp(bottom + 1, i64::from(canvas[1]).max(bottom + 1));
        } else if handle.is_south() {
            bottom = boundary[1].clamp(0, top - 1);
        }

        Self {
            x: left as u32,
            y: bottom as u32,
            width: (right - left) as u32,
            height: (top - bottom) as u32,
            ..self.clone()
        }
    }
}

const fn spans_overlap(a: [i64; 4], b: [i64; 4]) -> bool { a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3] }

/// walks one tile at a time so a fast drag can't jump over a thin room
fn step_toward(from: i64, to: i64, fits: impl Fn(i64) -> bool) -> i64 {
    let step = (to - from).signum();
    let mut at = from;
    while at != to && fits(at + step) {
        at += step;
    }

    at
}

impl PlanLabel {
    pub fn new(text: impl Into<String>, at: [u32; 2]) -> Self {
        Self {
            text: text.into(),
            x: at[0],
            y: at[1],
            text_size: DEFAULT_LABEL_SIZE,
            color: None,
        }
    }

    pub fn moved(&self, delta: [i64; 2], canvas: [u32; 2]) -> Self {
        let shift = |start: u32, delta: i64, limit: u32| (i64::from(start) + delta).clamp(0, i64::from(limit)) as u32;

        Self {
            x: shift(self.x, delta[0], canvas[0]),
            y: shift(self.y, delta[1], canvas[1]),
            ..self.clone()
        }
    }
}

impl Default for Plan {
    fn default() -> Self { Self::new(DEFAULT_PLAN_SIZE, DEFAULT_PLAN_SIZE) }
}

impl Plan {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            version: PLAN_VERSION,
            width: width.clamp(1, MAX_PLAN_SIZE),
            height: height.clamp(1, MAX_PLAN_SIZE),
            rects: Vec::new(),
            labels: Vec::new(),
        }
    }

    pub const fn size(&self) -> [u32; 2] { [self.width, self.height] }

    pub fn rect_at(&self, point: [f32; 2]) -> Option<usize> {
        // the rect drawn last wins

        self.rects.iter().rposition(|rect| rect.contains(point))
    }

    pub fn content_extent(&self) -> [u32; 2] {
        let rects = self.rects.iter().map(|rect| [rect.right(), rect.top()]);
        let labels = self.labels.iter().map(|label| [label.x, label.y]);

        rects
            .chain(labels)
            .fold([1, 1], |extent, [x, y]| [extent[0].max(x), extent[1].max(y)])
    }

    pub fn resize_canvas(&mut self, size: [u32; 2]) -> bool {
        let minimum = self.content_extent();
        let width = size[0].clamp(minimum[0], MAX_PLAN_SIZE);
        let height = size[1].clamp(minimum[1], MAX_PLAN_SIZE);
        let changed = [width, height] != self.size();

        self.width = width;
        self.height = height;

        changed
    }

    pub fn remove(&mut self, element: PlanElement) -> bool {
        match element {
            PlanElement::Rect(index) if index < self.rects.len() => {
                self.rects.remove(index);
            },
            PlanElement::Label(index) if index < self.labels.len() => {
                self.labels.remove(index);
            },
            _ => return false,
        }

        true
    }

    pub fn overlaps_any(&self, rect: &PlanRect, except: Option<usize>) -> bool {
        self.span_blocked(rect.span(), except)
    }

    fn span_blocked(&self, span: [i64; 4], except: Option<usize>) -> bool {
        self.rects
            .iter()
            .enumerate()
            .any(|(index, rect)| Some(index) != except && spans_overlap(span, rect.span()))
    }

    pub fn slide_rect(&self, index: usize, target: &PlanRect) -> PlanRect {
        let Some(current) = self.rects.get(index) else {
            return target.clone();
        };

        // a room that already overlaps, from an older file, moves freely so it can be pulled apart
        if self.overlaps_any(current, Some(index)) {
            return target.clone();
        }

        let mut span = current.span();
        let size = [span[2] - span[0], span[3] - span[1]];
        for _ in 0..2 {
            for axis in 0..2 {
                let start = span;
                let at = step_toward(start[axis], target.span()[axis], |value| {
                    let mut candidate = start;
                    candidate[axis] = value;
                    candidate[axis + 2] = value + size[axis];

                    !self.span_blocked(candidate, Some(index))
                });
                span[axis] = at;
                span[axis + 2] = at + size[axis];
            }
        }

        current.with_span(span)
    }

    pub fn grow_rect(&self, index: usize, target: &PlanRect) -> PlanRect {
        let Some(current) = self.rects.get(index) else {
            return target.clone();
        };

        if self.overlaps_any(current, Some(index)) {
            return target.clone();
        }

        let mut span = current.span();
        let goal = target.span();
        for _ in 0..2 {
            for edge in 0..4 {
                let start = span;
                span[edge] = step_toward(start[edge], goal[edge], |value| {
                    let mut candidate = start;
                    candidate[edge] = value;

                    candidate[0] < candidate[2]
                        && candidate[1] < candidate[3]
                        && !self.span_blocked(candidate, Some(index))
                });
            }
        }

        current.with_span(span)
    }

    pub fn normalize(&mut self) {
        self.version = PLAN_VERSION;
        self.width = self.width.clamp(1, MAX_PLAN_SIZE);
        self.height = self.height.clamp(1, MAX_PLAN_SIZE);

        let (width, height) = (self.width, self.height);
        self.rects.retain(|rect| {
            !rect.area.is_empty() && rect.width > 0 && rect.height > 0 && rect.x < width && rect.y < height
        });
        for rect in &mut self.rects {
            rect.width = rect.width.min(width - rect.x);
            rect.height = rect.height.min(height - rect.y);
        }

        self.labels.retain(|label| !label.text.trim().is_empty());
        for label in &mut self.labels {
            label.x = label.x.min(width);
            label.y = label.y.min(height);
            label.text_size = if label.text_size.is_finite() {
                label.text_size.clamp(LABEL_SIZE_RANGE.0, LABEL_SIZE_RANGE.1)
            } else {
                DEFAULT_LABEL_SIZE
            };
        }
    }

    pub fn from_toml(text: &str) -> Result<Self, PlanError> {
        let mut plan: Self = toml::from_str(text).map_err(PlanError::Parse)?;
        plan.normalize();

        Ok(plan)
    }

    pub fn to_toml(&self) -> Result<String, PlanError> { toml::to_string_pretty(self).map_err(PlanError::Write) }

    pub fn from_area_grid(width: u32, height: u32, cells: &[Option<u32>], areas: &[String]) -> Self {
        let (columns, rows) = (width as usize, height as usize);
        let cell = |x: usize, y: usize| cells.get(y * columns + x).copied().flatten();
        let mut covered = vec![false; columns * rows];
        let mut plan = Self::new(width, height);

        for y in 0..rows {
            for x in 0..columns {
                let Some(area) = cell(x, y).filter(|_| !covered[y * columns + x]) else {
                    continue;
                };

                let open = |x: usize, y: usize, covered: &[bool]| cell(x, y) == Some(area) && !covered[y * columns + x];
                let right = x + (x..columns).take_while(|column| open(*column, y, &covered)).count();
                let top = y
                    + (y..rows)
                        .take_while(|row| (x..right).all(|column| open(column, *row, &covered)))
                        .count();
                for row in y..top {
                    covered[row * columns + x..row * columns + right].fill(true);
                }

                if let Some(name) = areas.get(area as usize) {
                    plan.rects.push(PlanRect {
                        area: name.clone(),
                        x: x as u32,
                        y: y as u32,
                        width: (right - x) as u32,
                        height: (top - y) as u32,
                    });
                }
            }
        }

        plan.normalize();

        plan
    }
}

struct PlanStep {
    label: &'static str,
    plan: Plan,
}

#[derive(Default)]
struct PlanHistory {
    undo: Vec<PlanStep>,
    redo: Vec<PlanStep>,
}

pub struct PlanDocument {
    id: PlanId,
    pub path: Option<PathBuf>,
    plan: Plan,
    saved: Plan,
    history: PlanHistory,
}

impl Default for PlanDocument {
    fn default() -> Self { Self::new() }
}

impl PlanDocument {
    pub fn new() -> Self { Self::from_plan(Plan::default(), None) }

    pub fn untitled(plan: Plan) -> Self {
        Self {
            saved: Plan::default(),
            ..Self::from_plan(plan, None)
        }
    }

    fn from_plan(plan: Plan, path: Option<PathBuf>) -> Self {
        Self {
            id: PlanId::new(),
            path,
            saved: plan.clone(),
            plan,
            history: PlanHistory::default(),
        }
    }

    pub fn open(path: &Path) -> Result<Self, PlanError> {
        let plan = Plan::from_toml(&fs::read_to_string(path)?)?;

        Ok(Self::from_plan(plan, Some(path.to_path_buf())))
    }

    pub fn save(&mut self) -> Result<(), PlanError> {
        let path = self.path.clone().ok_or(PlanError::NoPath)?;
        fs::write(path, self.plan.to_toml()?)?;
        self.saved = self.plan.clone();

        Ok(())
    }

    pub fn save_as(&mut self, mut path: PathBuf) -> Result<(), PlanError> {
        if path.extension().is_none() {
            path.set_extension(PLAN_EXTENSION);
        }

        fs::write(&path, self.plan.to_toml()?)?;
        self.path = Some(path);
        self.saved = self.plan.clone();

        Ok(())
    }

    pub const fn id(&self) -> PlanId { self.id }

    pub const fn plan(&self) -> &Plan { &self.plan }

    pub const fn plan_mut(&mut self) -> &mut Plan { &mut self.plan }

    pub fn edit(&mut self, label: &'static str, apply: impl FnOnce(&mut Plan)) -> bool {
        let before = self.plan.clone();
        apply(&mut self.plan);

        self.commit(label, before)
    }

    pub fn commit(&mut self, label: &'static str, before: Plan) -> bool {
        if before == self.plan {
            return false;
        }

        self.history.undo.push(PlanStep { label, plan: before });
        if self.history.undo.len() > HISTORY_LIMIT {
            self.history.undo.remove(0);
        }

        self.history.redo.clear();

        true
    }

    pub fn restore(&mut self, before: Plan) { self.plan = before; }

    pub fn undo(&mut self) -> bool {
        let Some(step) = self.history.undo.pop() else {
            return false;
        };

        let current = std::mem::replace(&mut self.plan, step.plan);
        self.history.redo.push(PlanStep {
            label: step.label,
            plan: current,
        });

        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(step) = self.history.redo.pop() else {
            return false;
        };

        let current = std::mem::replace(&mut self.plan, step.plan);
        self.history.undo.push(PlanStep {
            label: step.label,
            plan: current,
        });

        true
    }

    pub fn undo_label(&self) -> Option<&'static str> { self.history.undo.last().map(|step| step.label) }

    pub fn redo_label(&self) -> Option<&'static str> { self.history.redo.last().map(|step| step.label) }

    pub fn is_dirty(&self) -> bool { self.plan != self.saved }

    pub fn title(&self) -> String {
        let name = self
            .path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "untitled plan".to_string());

        if self.is_dirty() { format!("{name} *") } else { name }
    }
}

#[derive(Default)]
pub struct PlanStore {
    documents: Vec<PlanDocument>,
    focused: Option<PlanId>,
    /// area type path the rectangle tool draws with
    pub brush: Option<String>,
    pub tool: PlanTool,
}

impl PlanStore {
    pub fn documents(&self) -> &[PlanDocument] { &self.documents }

    pub fn ids(&self) -> impl Iterator<Item = PlanId> + '_ { self.documents.iter().map(PlanDocument::id) }

    pub fn is_empty(&self) -> bool { self.documents.is_empty() }

    pub fn get(&self, id: PlanId) -> Option<&PlanDocument> { self.documents.iter().find(|document| document.id == id) }

    pub fn get_mut(&mut self, id: PlanId) -> Option<&mut PlanDocument> {
        self.documents.iter_mut().find(|document| document.id == id)
    }

    pub fn open(&mut self, document: PlanDocument) -> PlanId {
        let id = document.id;
        self.documents.push(document);
        self.focused = Some(id);

        id
    }

    pub fn close(&mut self, id: PlanId) -> Option<PlanDocument> {
        let index = self.documents.iter().position(|document| document.id == id)?;
        if self.focused == Some(id) {
            self.focused = None;
        }

        Some(self.documents.remove(index))
    }

    pub const fn focused(&self) -> Option<PlanId> { self.focused }

    pub fn set_focused(&mut self, id: Option<PlanId>) { self.focused = id.filter(|id| self.get(*id).is_some()); }

    pub fn focused_document(&self) -> Option<&PlanDocument> { self.focused.and_then(|id| self.get(id)) }

    pub fn focused_document_mut(&mut self) -> Option<&mut PlanDocument> { self.focused.and_then(|id| self.get_mut(id)) }

    pub fn document_for_path(&self, path: &Path) -> Option<PlanId> {
        let wanted = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());

        self.documents
            .iter()
            .find(|document| {
                document.path.as_deref().is_some_and(|candidate| {
                    fs::canonicalize(candidate).unwrap_or_else(|_| candidate.to_path_buf()) == wanted
                })
            })
            .map(PlanDocument::id)
    }

    pub fn has_unsaved_changes(&self) -> bool { self.documents.iter().any(PlanDocument::is_dirty) }

    pub fn dirty_count(&self) -> usize { self.documents.iter().filter(|document| document.is_dirty()).count() }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CANVAS: [u32; 2] = [20, 10];

    fn room(x: u32, y: u32, width: u32, height: u32) -> PlanRect {
        PlanRect {
            area: "/area/station".to_owned(),
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn drawing_in_any_direction_gives_the_same_rect() {
        let expected = room(2, 3, 4, 2);

        assert_eq!(PlanRect::from_tiles("/area/station", [2, 3], [5, 4]), expected);
        assert_eq!(PlanRect::from_tiles("/area/station", [5, 4], [2, 3]), expected);
        assert_eq!(PlanRect::from_tiles("/area/station", [2, 4], [5, 3]), expected);
        assert_eq!(PlanRect::from_tiles("/area/station", [5, 3], [2, 4]), expected);
    }

    #[test]
    fn corners_are_inclusive_tiles() {
        assert_eq!(room(2, 3, 4, 2).corners(), [[2, 3], [5, 3], [2, 4], [5, 4]]);
        assert_eq!(room(0, 0, 1, 1).corners(), [[0, 0]; 4]);
    }

    #[test]
    fn moving_stays_on_the_canvas() {
        let rect = room(2, 3, 4, 2);

        assert_eq!(rect.moved([3, 1], CANVAS), room(5, 4, 4, 2));
        assert_eq!(rect.moved([-10, -10], CANVAS), room(0, 0, 4, 2));
        assert_eq!(rect.moved([100, 100], CANVAS), room(16, 8, 4, 2));
    }

    #[test]
    fn resizing_follows_each_handle() {
        let rect = room(2, 3, 4, 2);

        assert_eq!(rect.resized(Handle::East, [9, 0], CANVAS), room(2, 3, 7, 2));
        assert_eq!(rect.resized(Handle::West, [0, 0], CANVAS), room(0, 3, 6, 2));
        assert_eq!(rect.resized(Handle::North, [0, 8], CANVAS), room(2, 3, 4, 5));
        assert_eq!(rect.resized(Handle::South, [0, 1], CANVAS), room(2, 1, 4, 4));
        assert_eq!(rect.resized(Handle::NorthEast, [7, 6], CANVAS), room(2, 3, 5, 3));
        assert_eq!(rect.resized(Handle::SouthWest, [1, 2], CANVAS), room(1, 2, 5, 3));
    }

    #[test]
    fn resizing_never_flips_or_leaves_the_canvas() {
        let rect = room(2, 3, 4, 2);

        assert_eq!(rect.resized(Handle::East, [-5, 0], CANVAS), room(2, 3, 1, 2));
        assert_eq!(rect.resized(Handle::West, [30, 0], CANVAS), room(5, 3, 1, 2));
        assert_eq!(rect.resized(Handle::North, [0, 99], CANVAS), room(2, 3, 4, 7));
        assert_eq!(rect.resized(Handle::South, [0, -4], CANVAS), room(2, 0, 4, 5));
    }

    #[test]
    fn the_topmost_rect_is_hit_first() {
        let mut plan = Plan::new(20, 20);
        plan.rects = vec![room(0, 0, 10, 10), room(5, 5, 10, 10)];

        assert_eq!(plan.rect_at([6.5, 6.5]), Some(1));
        assert_eq!(plan.rect_at([1.0, 1.0]), Some(0));
        assert_eq!(plan.rect_at([10.0, 10.0]), Some(1));
        assert_eq!(plan.rect_at([15.0, 15.0]), None);
    }

    #[test]
    fn the_canvas_cannot_shrink_past_its_content() {
        let mut plan = Plan::new(50, 50);
        plan.rects.push(room(10, 5, 10, 10));
        plan.labels.push(PlanLabel::new("title", [30, 2]));

        assert!(plan.resize_canvas([5, 5]));
        assert_eq!(plan.size(), [30, 15]);
        assert!(plan.resize_canvas([5000, 40]));
        assert_eq!(plan.size(), [MAX_PLAN_SIZE, 40]);
        assert!(!plan.resize_canvas([MAX_PLAN_SIZE, 40]));
    }

    #[test]
    fn rooms_that_share_an_edge_do_not_overlap() {
        assert!(!room(0, 0, 2, 2).overlaps(&room(2, 0, 2, 2)));
        assert!(!room(0, 0, 2, 2).overlaps(&room(0, 2, 2, 2)));
        assert!(room(0, 0, 3, 3).overlaps(&room(2, 2, 2, 2)));
        assert!(room(0, 0, 4, 4).overlaps(&room(1, 1, 1, 1)));
    }

    fn plan_with(rects: Vec<PlanRect>) -> Plan {
        let mut plan = Plan::new(20, 20);
        plan.rects = rects;

        plan
    }

    #[test]
    fn a_moved_room_stops_flush_against_its_neighbour() {
        let plan = plan_with(vec![room(0, 0, 2, 2), room(6, 0, 2, 2)]);

        assert_eq!(plan.slide_rect(0, &room(10, 0, 2, 2)), room(4, 0, 2, 2));
        assert_eq!(plan.slide_rect(0, &room(3, 0, 2, 2)), room(3, 0, 2, 2));
    }

    #[test]
    fn a_diagonal_move_slides_along_a_wall() {
        let plan = plan_with(vec![room(0, 0, 2, 2), room(4, 0, 2, 10)]);

        assert_eq!(plan.slide_rect(0, &room(8, 5, 2, 2)), room(2, 5, 2, 2));
    }

    #[test]
    fn a_long_drag_never_jumps_over_a_thin_room() {
        let plan = plan_with(vec![room(0, 0, 2, 2), room(5, 0, 1, 20)]);

        assert_eq!(plan.slide_rect(0, &room(15, 0, 2, 2)), room(3, 0, 2, 2));
    }

    #[test]
    fn an_overlapping_room_from_an_old_file_moves_freely() {
        let plan = plan_with(vec![room(0, 0, 4, 4), room(1, 1, 2, 2)]);

        assert_eq!(plan.slide_rect(1, &room(10, 10, 2, 2)), room(10, 10, 2, 2));
    }

    #[test]
    fn a_resized_edge_stops_at_the_neighbour() {
        let plan = plan_with(vec![room(0, 0, 2, 2), room(5, 0, 2, 2)]);

        assert_eq!(plan.grow_rect(0, &room(0, 0, 9, 2)), room(0, 0, 5, 2));
        assert_eq!(plan.grow_rect(0, &room(0, 0, 9, 6)), room(0, 0, 5, 6));
        assert_eq!(plan.grow_rect(1, &room(1, 0, 6, 2)), room(2, 0, 5, 2));
    }

    #[test]
    fn hand_edited_files_are_repaired() {
        let plan = Plan::from_toml(
            r#"
            width = 10
            height = 10

            [[rects]]
            area = "/area/station"
            x = 8
            y = 0
            width = 5
            height = 3

            [[rects]]
            area = "/area/station"
            x = 12
            y = 0
            width = 1
            height = 1

            [[rects]]
            area = "/area/station"
            x = 0
            y = 0
            width = 0
            height = 1

            [[labels]]
            text = "  "
            x = 0
            y = 0
            text_size = 4.0

            [[labels]]
            text = "Title"
            x = 40
            y = 3
            text_size = 900.0
            "#,
        )
        .expect("plan");

        assert_eq!(plan.rects, vec![room(8, 0, 2, 3)]);
        assert_eq!(plan.labels.len(), 1);
        assert_eq!([plan.labels[0].x, plan.labels[0].y], [10, 3]);
        assert_eq!(plan.labels[0].text_size, LABEL_SIZE_RANGE.1);
    }

    #[test]
    fn plans_survive_a_toml_round_trip() {
        let mut plan = Plan::new(64, 32);
        plan.rects.push(room(1, 2, 3, 4));
        plan.labels.push(PlanLabel {
            color: Some("#ff8800".to_owned()),
            ..PlanLabel::new("Boutique Station", [5, 30])
        });

        assert_eq!(Plan::from_toml(&plan.to_toml().expect("toml")).expect("plan"), plan);
        assert_eq!(Plan::from_toml("").expect("empty plan"), Plan::default());
    }

    #[test]
    fn edits_undo_and_redo() {
        let mut document = PlanDocument::new();

        assert!(document.edit("Draw room", |plan| plan.rects.push(room(0, 0, 2, 2))));
        assert!(document.edit("Add label", |plan| plan.labels.push(PlanLabel::new("a", [1, 1]))));
        assert_eq!(document.undo_label(), Some("Add label"));
        assert!(document.undo());
        assert!(document.plan().labels.is_empty());
        assert_eq!(document.redo_label(), Some("Add label"));
        assert!(document.redo());
        assert_eq!(document.plan().labels.len(), 1);
        assert!(!document.redo());
    }

    #[test]
    fn a_new_edit_drops_the_redo_steps() {
        let mut document = PlanDocument::new();
        document.edit("Draw room", |plan| plan.rects.push(room(0, 0, 2, 2)));
        document.undo();
        document.edit("Draw room", |plan| plan.rects.push(room(4, 4, 2, 2)));

        assert!(!document.redo());
    }

    #[test]
    fn undoing_to_the_saved_plan_makes_it_clean() {
        let mut document = PlanDocument::new();

        assert!(!document.is_dirty());
        document.edit("Draw room", |plan| plan.rects.push(room(0, 0, 2, 2)));
        assert!(document.is_dirty());
        assert!(document.title().ends_with(" *"));
        document.undo();
        assert!(!document.is_dirty());
    }

    #[test]
    fn a_gesture_that_changes_nothing_leaves_no_step() {
        let mut document = PlanDocument::new();
        let before = document.plan().clone();

        assert!(!document.commit("Move room", before));
        assert!(!document.edit("Move room", |_| {}));
        assert_eq!(document.undo_label(), None);
    }

    #[test]
    fn history_is_capped() {
        let mut document = PlanDocument::new();
        for x in 0..HISTORY_LIMIT as u32 + 10 {
            document.edit("Draw room", |plan| plan.rects.push(room(x % 200, 0, 1, 1)));
        }

        let mut steps = 0;
        while document.undo() {
            steps += 1;
        }

        assert_eq!(steps, HISTORY_LIMIT);
    }

    #[test]
    fn saving_writes_the_extension_and_reopens() {
        let dir = std::env::temp_dir().join(format!("rmd-plan-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("dir");
        let mut document = PlanDocument::new();
        document.edit("Draw room", |plan| plan.rects.push(room(3, 3, 2, 2)));

        document.save_as(dir.join("station")).expect("save");

        let path = document.path.clone().expect("path");
        assert_eq!(path.extension().and_then(|ext| ext.to_str()), Some(PLAN_EXTENSION));
        assert!(!document.is_dirty());
        assert_eq!(PlanDocument::open(&path).expect("open").plan(), document.plan());
        fs::remove_dir_all(dir).ok();
    }

    fn areas() -> Vec<String> { vec![String::from("/area/a"), String::from("/area/b")] }

    fn covered_tiles(plan: &Plan) -> u32 { plan.rects.iter().map(|rect| rect.width * rect.height).sum() }

    fn overlaps(plan: &Plan) -> bool {
        plan.rects.iter().enumerate().any(|(index, a)| {
            plan.rects[index + 1..]
                .iter()
                .any(|b| a.x < b.right() && b.x < a.right() && a.y < b.top() && b.y < a.top())
        })
    }

    #[test]
    fn an_l_shaped_area_becomes_two_exact_rects() {
        // rows from the bottom: AA.., AAAA, AAAA
        let a = Some(0);
        let cells = [a, a, None, None, a, a, a, a, a, a, a, a];
        let plan = Plan::from_area_grid(4, 3, &cells, &areas());

        assert_eq!(plan.size(), [4, 3]);
        assert_eq!(plan.rects.len(), 2);
        assert_eq!(covered_tiles(&plan), 10);
        assert!(!overlaps(&plan));
        assert_eq!(plan.rects[0], PlanRect::from_tiles("/area/a", [0, 0], [1, 2]));
        assert_eq!(plan.rects[1], PlanRect::from_tiles("/area/a", [2, 1], [3, 2]));
    }

    #[test]
    fn separate_regions_and_areas_stay_apart() {
        let (a, b) = (Some(0), Some(1));
        let cells = [a, None, a, b, b, b];
        let plan = Plan::from_area_grid(3, 2, &cells, &areas());

        assert_eq!(plan.rects.len(), 3);
        assert_eq!(covered_tiles(&plan), 5);
        assert!(!overlaps(&plan));
        assert_eq!(plan.rects[2], PlanRect::from_tiles("/area/b", [0, 1], [2, 1]));
    }

    #[test]
    fn a_converted_plan_is_unsaved() {
        let plan = Plan::from_area_grid(2, 2, &[Some(0); 4], &areas());
        let document = PlanDocument::untitled(plan);

        assert!(document.is_dirty());
        assert_eq!(document.path, None);
        assert_eq!(
            document.plan().rects,
            vec![PlanRect::from_tiles("/area/a", [0, 0], [1, 1])]
        );
    }

    #[test]
    fn the_store_tracks_focus() {
        let mut store = PlanStore::default();
        let first = store.open(PlanDocument::new());
        let second = store.open(PlanDocument::new());

        assert_eq!(store.focused(), Some(second));
        store.set_focused(Some(first));
        assert_eq!(store.focused(), Some(first));
        assert!(store.close(first).is_some());
        assert_eq!(store.focused(), None);
        assert_eq!(store.ids().collect::<Vec<_>>(), vec![second]);
        store.set_focused(Some(first));
        assert_eq!(store.focused(), None);
    }
}
