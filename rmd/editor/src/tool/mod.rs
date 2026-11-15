pub(crate) mod panel;
pub(crate) mod session;

use base::{
    path::TreePath,
    types::{Identifier, Value},
    vars,
};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use dmi::metadata::Dir;
use dmm::{Coord, Prefab};
use objtree::{ObjectTree, TypeId};
use ui::icons::materialdesignicons::{
    ICON_ARROW_ALL, ICON_COMMENT_TEXT_OUTLINE, ICON_ERASER, ICON_EYEDROPPER, ICON_FIND_REPLACE, ICON_FORMAT_COLOR_FILL,
    ICON_PENCIL, ICON_SELECT_DRAG, ICON_TEXTURE, ICON_VECTOR_POLYLINE,
};

use crate::{
    clipboard::{has_kind, merge_pasted},
    document::{MapDocument, PlacedPrefab, PlacedTile, PrefabInstanceId, Selection, command::Edit, grid::Grid},
    environment::world::{BaseTile, World},
    frame::{HiddenTypes, visual},
};

pub const MAX_FILL_TILES: usize = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tool {
    Place,
    #[default]
    Pick,
    Node,
    Select,
    Delete,
    Replace,
    Fill,
    RectFill,
    Comment,
    Move,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionTransform {
    RotateClockwise,
    RotateCounterClockwise,
    MirrorHorizontal,
    MirrorVertical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SelectionRotation {
    #[default]
    Original,
    Clockwise,
    Half,
    CounterClockwise,
}

impl SelectionRotation {
    pub const fn clockwise(self) -> Self {
        match self {
            Self::Original => Self::Clockwise,
            Self::Clockwise => Self::Half,
            Self::Half => Self::CounterClockwise,
            Self::CounterClockwise => Self::Original,
        }
    }

    pub const fn counter_clockwise(self) -> Self {
        match self {
            Self::Original => Self::CounterClockwise,
            Self::Clockwise => Self::Original,
            Self::Half => Self::Clockwise,
            Self::CounterClockwise => Self::Half,
        }
    }

    const fn transforms(self) -> &'static [SelectionTransform] {
        match self {
            Self::Original => &[],
            Self::Clockwise => &[SelectionTransform::RotateClockwise],
            Self::Half => &[SelectionTransform::RotateClockwise, SelectionTransform::RotateClockwise],
            Self::CounterClockwise => &[SelectionTransform::RotateCounterClockwise],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionPlacement {
    Move,
    Copy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FillMode {
    #[default]
    Wall,
    EntireArea,
    Custom,
}

impl FillMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Wall => "Wall",
            Self::EntireArea => "Entire Area",
            Self::Custom => "Custom",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RegionSelectionMode {
    #[default]
    Full,
    Hollow {
        line_width: u32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectionMask {
    pub bounds: Selection,
    pub mode: RegionSelectionMode,
}

impl SelectionMask {
    pub fn includes(self, coord: Coord) -> bool {
        self.mode.includes(self.bounds, coord)
    }
}

impl RegionSelectionMode {
    /// all the tiles of `selection` this mode actually edits, hollow mode will
    /// only edit tiles that are inside the ring
    pub fn tiles(self, selection: Selection) -> impl Iterator<Item = Coord> {
        selection.iter().filter(move |coord| self.includes(selection, *coord))
    }

    pub fn includes(self, selection: Selection, coord: Coord) -> bool {
        if !selection.contains(coord) {
            return false;
        }

        match self {
            Self::Full => true,
            Self::Hollow { line_width } => {
                let line_width = line_width.max(1);

                coord.x - selection.min.x < line_width
                    || selection.max.x - coord.x < line_width
                    || coord.y - selection.min.y < line_width
                    || selection.max.y - coord.y < line_width
            },
        }
    }
}

pub struct ToolContext<'a> {
    pub document: &'a mut MapDocument,
    pub tree: &'a ObjectTree,
    pub world: &'a World,
    pub prefab: Option<&'a Prefab>,
    pub target: Option<PrefabInstanceId>,
    pub coord: Coord,
    pub anchor: Option<Coord>,
    pub fill_mode: FillMode,
    pub custom_fill_boundaries: &'a [TreePath],
}

pub struct ToolEdit {
    pub edit: Edit,
    pub picked: Option<PrefabInstanceId>,
    pub affected: Vec<PrefabInstanceId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillError {
    TooLarge { limit: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlacementKind {
    Atom,
    Turf,
    Area,
}

impl Tool {
    pub fn label(self) -> &'static str {
        match self {
            Tool::Place => "Place",
            Tool::Pick => "Pick",
            Tool::Node => "Node",
            Tool::Select => "Select",
            Tool::Delete => "Delete",
            Tool::Replace => "Replace",
            Tool::Fill => "Fill",
            Tool::RectFill => "Rectangle Fill",
            Tool::Comment => "Comment",
            Tool::Move => "Move",
        }
    }

    pub fn icon(self) -> char {
        match self {
            Tool::Place => ICON_PENCIL,
            Tool::Pick => ICON_EYEDROPPER,
            Tool::Node => ICON_VECTOR_POLYLINE,
            Tool::Select => ICON_SELECT_DRAG,
            Tool::Delete => ICON_ERASER,
            Tool::Replace => ICON_FIND_REPLACE,
            Tool::Fill => ICON_FORMAT_COLOR_FILL,
            Tool::RectFill => ICON_TEXTURE,
            Tool::Comment => ICON_COMMENT_TEXT_OUTLINE,
            Tool::Move => ICON_ARROW_ALL,
        }
    }

    pub fn build_edit(self, context: &mut ToolContext<'_>) -> Option<ToolEdit> {
        match self {
            Self::Place => place(context),
            Self::Delete => delete(context),
            Self::Pick | Self::Node | Self::Select | Self::Replace | Self::RectFill | Self::Comment | Self::Move => {
                None
            },
            Self::Fill => fill(context, Some(MAX_FILL_TILES), None).ok().flatten(),
        }
    }

    pub fn build_fill_edit(
        self, context: &mut ToolContext<'_>, max_tiles: Option<usize>,
    ) -> Result<Option<ToolEdit>, FillError> {
        self.build_fill_edit_with_mask(context, max_tiles, None)
    }

    pub fn build_fill_edit_with_mask(
        self, context: &mut ToolContext<'_>, max_tiles: Option<usize>, mask: Option<SelectionMask>,
    ) -> Result<Option<ToolEdit>, FillError> {
        if self == Self::Fill {
            fill(context, max_tiles, mask)
        } else {
            Ok(self.build_edit(context))
        }
    }
}

impl From<Tool> for net::Tool {
    fn from(tool: Tool) -> Self {
        match tool {
            Tool::Place => Self::Place,
            Tool::Pick => Self::Pick,
            Tool::Node => Self::Node,
            Tool::Select => Self::Select,
            Tool::Delete => Self::Delete,
            Tool::Replace => Self::Replace,
            Tool::Fill | Tool::RectFill => Self::Fill,
            Tool::Comment => Self::Comment,
            Tool::Move => Self::Move,
        }
    }
}

impl From<net::Tool> for Tool {
    fn from(tool: net::Tool) -> Self {
        match tool {
            net::Tool::Place => Self::Place,
            net::Tool::Pick => Self::Pick,
            net::Tool::Node => Self::Node,
            net::Tool::Select => Self::Select,
            net::Tool::Delete => Self::Delete,
            net::Tool::Replace => Self::Replace,
            net::Tool::Fill => Self::Fill,
            net::Tool::Comment => Self::Comment,
            net::Tool::Move => Self::Move,
        }
    }
}

impl From<RegionSelectionMode> for net::SelectionMode {
    fn from(mode: RegionSelectionMode) -> Self {
        match mode {
            RegionSelectionMode::Full => Self::Full,
            RegionSelectionMode::Hollow { line_width } => Self::Hollow { line_width },
        }
    }
}

impl From<net::SelectionMode> for RegionSelectionMode {
    fn from(mode: net::SelectionMode) -> Self {
        match mode {
            net::SelectionMode::Full => Self::Full,
            net::SelectionMode::Hollow { line_width } => Self::Hollow { line_width },
        }
    }
}

#[cfg(test)]
pub fn move_selection(
    document: &mut MapDocument, tree: &ObjectTree, world: &World, selection: Selection, target_min: Coord,
) -> Option<(ToolEdit, Selection)> {
    place_selection(
        document,
        tree,
        world,
        selection,
        target_min,
        SelectionRotation::Original,
        SelectionPlacement::Move,
    )
}

#[cfg(test)]
pub fn copy_selection(
    document: &mut MapDocument, tree: &ObjectTree, world: &World, selection: Selection, target_min: Coord,
) -> Option<(ToolEdit, Selection)> {
    place_selection(
        document,
        tree,
        world,
        selection,
        target_min,
        SelectionRotation::Original,
        SelectionPlacement::Copy,
    )
}

pub fn fill_selection(
    document: &mut MapDocument, tree: &ObjectTree, selection: Selection, prefab: &Prefab, mode: RegionSelectionMode,
) -> Option<ToolEdit> {
    if !selection.is_well_formed()
        || !coord_in_bounds(selection.min, document.map.size())
        || !coord_in_bounds(selection.max, document.map.size())
        || mode.tiles(selection).any(|coord| !document.allows_edit_at(coord))
    {
        return None;
    }

    let kind = placement_kind(tree, prefab)?;
    let mut edit = Edit::new(format!("fill region with {}", prefab.path));
    let mut affected = Vec::new();
    for coord in mode.tiles(selection) {
        let Some((after, _, tile_affected)) = place_prefab_on_tile(document, tree, coord, prefab, kind) else {
            continue;
        };

        edit.change(document, coord, after);
        affected.extend(tile_affected);
    }

    if edit.is_empty() {
        return None;
    }

    affected.sort_unstable_by_key(|id| id.get());

    Some(ToolEdit {
        edit,
        picked: None,
        affected,
    })
}

#[cfg(test)]
pub fn place_selection(
    document: &mut MapDocument, tree: &ObjectTree, world: &World, selection: Selection, target_min: Coord,
    rotation: SelectionRotation, placement: SelectionPlacement,
) -> Option<(ToolEdit, Selection)> {
    place_selection_with_mode(
        document,
        tree,
        world,
        selection,
        target_min,
        rotation,
        placement,
        RegionSelectionMode::Full,
        &HiddenTypes::default(),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn place_selection_with_mode(
    document: &mut MapDocument, tree: &ObjectTree, world: &World, selection: Selection, target_min: Coord,
    rotation: SelectionRotation, placement: SelectionPlacement, mode: RegionSelectionMode, hidden: &HiddenTypes,
) -> Option<(ToolEdit, Selection)> {
    let target = rotated_selection_at(selection, target_min, rotation)?;
    if target == selection && rotation == SelectionRotation::Original {
        return None;
    }
    let label = match placement {
        SelectionPlacement::Move => "move region",
        SelectionPlacement::Copy => "copy region",
    };

    build_selection_edit(
        document,
        tree,
        world,
        SelectionEditRequest {
            source: selection,
            target,
            transforms: rotation.transforms(),
            placement,
            mode,
            label,
        },
        hidden,
    )
}

#[cfg(test)]
pub fn transform_selection(
    document: &mut MapDocument, tree: &ObjectTree, world: &World, selection: Selection, transform: SelectionTransform,
) -> Option<(ToolEdit, Selection)> {
    transform_selection_with_mode(
        document,
        tree,
        world,
        selection,
        transform,
        RegionSelectionMode::Full,
        &HiddenTypes::default(),
    )
}

pub fn transform_selection_with_mode(
    document: &mut MapDocument, tree: &ObjectTree, world: &World, selection: Selection, transform: SelectionTransform,
    mode: RegionSelectionMode, hidden: &HiddenTypes,
) -> Option<(ToolEdit, Selection)> {
    if !selection.is_well_formed() {
        return None;
    }
    let target = transformed_selection(selection, transform)?;
    let label = match transform {
        SelectionTransform::RotateClockwise => "rotate region clockwise",
        SelectionTransform::RotateCounterClockwise => "rotate region counterclockwise",
        SelectionTransform::MirrorHorizontal => "mirror region horizontally",
        SelectionTransform::MirrorVertical => "mirror region vertically",
    };

    build_selection_edit(
        document,
        tree,
        world,
        SelectionEditRequest {
            source: selection,
            target,
            transforms: std::slice::from_ref(&transform),
            placement: SelectionPlacement::Move,
            mode,
            label,
        },
        hidden,
    )
}

pub fn rotated_selection_at(selection: Selection, target_min: Coord, rotation: SelectionRotation) -> Option<Selection> {
    let mut target = selection.with_min(target_min)?;
    for transform in rotation.transforms() {
        target = transformed_selection(target, *transform)?;
    }

    Some(target)
}

pub fn rotate_point(point: (u32, u32), width: u32, height: u32, rotation: SelectionRotation) -> (u32, u32) {
    match rotation {
        SelectionRotation::Original => point,
        SelectionRotation::Clockwise => transform_point(point, width, height, SelectionTransform::RotateClockwise),
        SelectionRotation::Half => (width - 1 - point.0, height - 1 - point.1),
        SelectionRotation::CounterClockwise => {
            transform_point(point, width, height, SelectionTransform::RotateCounterClockwise)
        },
    }
}

pub fn rotate_prefab(tree: &ObjectTree, prefab: &mut Prefab, rotation: SelectionRotation) {
    for transform in rotation.transforms() {
        transform_prefab(tree, prefab, *transform);
    }
}

pub fn transformed_selection(selection: Selection, transform: SelectionTransform) -> Option<Selection> {
    if !selection.is_well_formed() {
        return None;
    }
    let (width, height) = match transform {
        SelectionTransform::RotateClockwise | SelectionTransform::RotateCounterClockwise => {
            (selection.height(), selection.width())
        },
        SelectionTransform::MirrorHorizontal | SelectionTransform::MirrorVertical => {
            (selection.width(), selection.height())
        },
    };
    let target = Selection {
        min: selection.min,
        max: Coord::new(
            selection.min.x.checked_add(width.checked_sub(1)?)?,
            selection.min.y.checked_add(height.checked_sub(1)?)?,
            selection.min.z,
        ),
    };

    Some(target)
}

struct SelectionEditRequest<'a> {
    source: Selection,
    target: Selection,
    transforms: &'a [SelectionTransform],
    placement: SelectionPlacement,
    mode: RegionSelectionMode,
    label: &'a str,
}

impl SelectionEditRequest<'_> {
    fn source_tiles(&self) -> impl Iterator<Item = Coord> {
        self.mode.tiles(self.source)
    }

    fn touched_tiles(&self) -> impl Iterator<Item = Coord> {
        self.source_tiles().chain(self.mode.tiles(self.target))
    }

    fn destination_of(&self, coord: Coord) -> Coord {
        let mut point = (coord.x - self.source.min.x, coord.y - self.source.min.y);
        let (mut width, mut height) = (self.source.width(), self.source.height());
        for transform in self.transforms {
            point = transform_point(point, width, height, *transform);
            if matches!(
                transform,
                SelectionTransform::RotateClockwise | SelectionTransform::RotateCounterClockwise
            ) {
                (width, height) = (height, width);
            }
        }

        Coord::new(
            self.target.min.x + point.0,
            self.target.min.y + point.1,
            self.target.min.z,
        )
    }
}

fn build_selection_edit(
    document: &mut MapDocument, tree: &ObjectTree, world: &World, request: SelectionEditRequest<'_>,
    hidden: &HiddenTypes,
) -> Option<(ToolEdit, Selection)> {
    let size = document.map.size();
    let touched = request.touched_tiles().collect::<HashSet<_>>();
    if touched
        .iter()
        .any(|coord| !coord_in_bounds(*coord, size) || !document.allows_edit_at(*coord))
    {
        return None;
    }

    let base_tile = if request.placement == SelectionPlacement::Move {
        Some(world.base_tile.as_ref()?)
    } else {
        None
    };
    let mut payload = Vec::with_capacity(touched.len());
    for coord in request.source_tiles() {
        let destination = request.destination_of(coord);
        let mut tile = document
            .placed_tile(coord)?
            .into_iter()
            .filter(|placed| !hidden.hides(placed.prefab()))
            .collect::<PlacedTile>();
        if request.placement == SelectionPlacement::Copy {
            tile = tile
                .into_iter()
                .map(|placed| document.instantiate(placed.prefab().clone()))
                .collect();
        }
        for transform in request.transforms {
            for placed in &mut tile {
                transform_prefab(tree, placed.prefab_mut(), *transform);
            }
        }
        payload.push((destination, tile));
    }

    let mut staged = HashMap::<Coord, PlacedTile>::new();
    if let Some(base_tile) = base_tile {
        for coord in request.source_tiles() {
            let mut left = document
                .placed_tile(coord)?
                .into_iter()
                .filter(|placed| hidden.hides(placed.prefab()))
                .collect::<PlacedTile>();
            for (kind, path) in [
                (PlacementKind::Turf, &base_tile.turf),
                (PlacementKind::Area, &base_tile.area),
            ] {
                if !has_kind(tree, &left, kind) {
                    let placed = document.instantiate(Prefab::new(path.clone()));
                    left.insert(insertion_index(tree, &left, kind), placed);
                }
            }

            staged.insert(coord, left);
        }
    }
    for (coord, incoming) in payload {
        let before = match staged.remove(&coord) {
            Some(before) => before,
            None => document.placed_tile(coord)?,
        };
        staged.insert(coord, merge_pasted(tree, &before, incoming, hidden));
    }

    let mut coords = staged.keys().copied().collect::<Vec<_>>();
    coords.sort_unstable_by_key(|coord| (coord.z, coord.y, coord.x));
    let mut edit = Edit::new(request.label);
    let mut affected = HashSet::new();
    for coord in coords {
        let after = staged.remove(&coord)?;
        let before = document.placed_tile(coord)?;
        if before == after {
            continue;
        }
        affected.extend(before.iter().map(PlacedPrefab::id));
        affected.extend(after.iter().map(PlacedPrefab::id));
        edit.change(document, coord, after);
    }
    if edit.is_empty() {
        return None;
    }

    let mut affected = affected.into_iter().collect::<Vec<_>>();
    affected.sort_unstable_by_key(|id| id.get());

    Some((
        ToolEdit {
            edit,
            picked: None,
            affected,
        },
        request.target,
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BaseKind {
    Turf,
    Area,
}

impl BaseKind {
    fn placement(self) -> PlacementKind {
        match self {
            Self::Turf => PlacementKind::Turf,
            Self::Area => PlacementKind::Area,
        }
    }

    fn root(self) -> &'static str {
        match self {
            Self::Turf => "turf",
            Self::Area => "area",
        }
    }

    fn is_prefab(self, prefab: &Prefab) -> bool {
        prefab
            .path
            .segments
            .first()
            .is_some_and(|segment| segment.as_str() == self.root())
    }

    fn is_in(self, tile: &[PlacedPrefab]) -> bool {
        tile.iter().any(|placed| self.is_prefab(placed.prefab()))
    }

    fn refill(
        self, document: &mut MapDocument, tree: &ObjectTree, path: &TreePath, tile: &mut PlacedTile,
    ) -> Option<PrefabInstanceId> {
        if self.is_in(tile) {
            return None;
        }

        let placed = document.instantiate(Prefab::new(path.clone()));
        let id = placed.id();
        tile.insert(insertion_index(tree, tile, self.placement()), placed);

        Some(id)
    }
}

pub fn is_missing_base(tile: &[Prefab]) -> bool {
    [BaseKind::Turf, BaseKind::Area]
        .into_iter()
        .any(|kind| !tile.iter().any(|prefab| kind.is_prefab(prefab)))
}

pub fn same_prefabs(left: &[PlacedPrefab], right: &[PlacedPrefab]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| left.prefab() == right.prefab())
}

pub fn refill_missing_bases(
    document: &mut MapDocument, tree: &ObjectTree, base_tile: &BaseTile, tile: &mut PlacedTile,
) -> Vec<PrefabInstanceId> {
    [(BaseKind::Turf, &base_tile.turf), (BaseKind::Area, &base_tile.area)]
        .into_iter()
        .filter_map(|(kind, path)| kind.refill(document, tree, path, tile))
        .collect()
}

fn transform_point((x, y): (u32, u32), width: u32, height: u32, transform: SelectionTransform) -> (u32, u32) {
    match transform {
        SelectionTransform::RotateClockwise => (y, width - 1 - x),
        SelectionTransform::RotateCounterClockwise => (height - 1 - y, x),
        SelectionTransform::MirrorHorizontal => (width - 1 - x, y),
        SelectionTransform::MirrorVertical => (x, height - 1 - y),
    }
}

fn transform_prefab(tree: &ObjectTree, prefab: &mut Prefab, transform: SelectionTransform) {
    let Some(id) = tree.id_of(&prefab.path) else {
        return;
    };
    let appearance = visual::resolve_id(tree, id, prefab);
    let path_direction = directional_type_group(tree, id).and_then(|(_, direction)| direction);
    let dir_name = Identifier::from(vars::DIR);
    let direction = path_direction.or_else(|| match visual::resolve_value(tree, id, prefab, &dir_name) {
        Some(resolved) => resolved.value.as_num().and_then(|value| Dir::from_bits(value as u32)),
        None => Dir::from_bits(appearance.dir),
    });
    let target_direction = direction.map(|direction| transform_direction(direction, transform));

    if let Some(direction) = target_direction {
        if let Some(path) = directional_type_target(tree, id, direction) {
            prefab.path = path;
            prefab.remove_var(&Identifier::from(vars::DIR));
        } else {
            set_resolved_number(tree, prefab, vars::DIR, direction.to_bits() as i32);
        }
    }

    for (x_name, y_name, values) in [
        (vars::PIXEL_X, vars::PIXEL_Y, [appearance.pixel_x, appearance.pixel_y]),
        (vars::PIXEL_W, vars::PIXEL_Z, [appearance.pixel_w, appearance.pixel_z]),
        (vars::STEP_X, vars::STEP_Y, [appearance.step_x, appearance.step_y]),
    ] {
        let [x, y] = transform_vector(values, transform);
        set_resolved_number(tree, prefab, x_name, x);
        set_resolved_number(tree, prefab, y_name, y);
    }
}

fn set_resolved_number(tree: &ObjectTree, prefab: &mut Prefab, name: &str, value: i32) {
    let inherited = tree
        .id_of(&prefab.path)
        .map(|id| visual::resolve_id(tree, id, &Prefab::new(prefab.path.clone())))
        .map(|appearance| match name {
            vars::DIR => appearance.dir as i32,
            vars::PIXEL_X => appearance.pixel_x,
            vars::PIXEL_Y => appearance.pixel_y,
            vars::PIXEL_W => appearance.pixel_w,
            vars::PIXEL_Z => appearance.pixel_z,
            vars::STEP_X => appearance.step_x,
            vars::STEP_Y => appearance.step_y,
            _ => value,
        });
    let name = Identifier::from(name);
    if inherited == Some(value) {
        prefab.remove_var(&name);
    } else {
        prefab.set_var(name, Value::Num(value as f32));
    }
}

fn transform_vector([x, y]: [i32; 2], transform: SelectionTransform) -> [i32; 2] {
    match transform {
        SelectionTransform::RotateClockwise => [y, x.saturating_neg()],
        SelectionTransform::RotateCounterClockwise => [y.saturating_neg(), x],
        SelectionTransform::MirrorHorizontal => [x.saturating_neg(), y],
        SelectionTransform::MirrorVertical => [x, y.saturating_neg()],
    }
}

fn transform_direction(direction: Dir, transform: SelectionTransform) -> Dir {
    use Dir::{East, North, Northeast, Northwest, South, Southeast, Southwest, West};

    match transform {
        SelectionTransform::RotateClockwise => match direction {
            North => East,
            East => South,
            South => West,
            West => North,
            Northeast => Southeast,
            Southeast => Southwest,
            Southwest => Northwest,
            Northwest => Northeast,
        },
        SelectionTransform::RotateCounterClockwise => match direction {
            North => West,
            West => South,
            South => East,
            East => North,
            Northeast => Northwest,
            Northwest => Southwest,
            Southwest => Southeast,
            Southeast => Northeast,
        },
        SelectionTransform::MirrorHorizontal => match direction {
            North => North,
            South => South,
            East => West,
            West => East,
            Northeast => Northwest,
            Northwest => Northeast,
            Southeast => Southwest,
            Southwest => Southeast,
        },
        SelectionTransform::MirrorVertical => match direction {
            North => South,
            South => North,
            East => East,
            West => West,
            Northeast => Southeast,
            Southeast => Northeast,
            Northwest => Southwest,
            Southwest => Northwest,
        },
    }
}

pub(crate) fn direction_from_name(name: &str) -> Option<Dir> {
    match name {
        "south" => Some(Dir::South),
        "north" => Some(Dir::North),
        "east" => Some(Dir::East),
        "west" => Some(Dir::West),
        "southeast" => Some(Dir::Southeast),
        "southwest" => Some(Dir::Southwest),
        "northeast" => Some(Dir::Northeast),
        "northwest" => Some(Dir::Northwest),
        _ => None,
    }
}

pub(crate) fn directional_type_group(tree: &ObjectTree, selected: TypeId) -> Option<(TypeId, Option<Dir>)> {
    let declaration = tree.get(selected)?;
    let name = declaration.path.name()?.as_str();
    if name == "directional" {
        return Some((selected, None));
    }

    if let Some(direction) = direction_from_name(name)
        && let Some(parent) = declaration.parent
        && tree
            .get(parent)
            .and_then(|parent| parent.path.name())
            .is_some_and(|name| name.as_str() == "directional")
    {
        return Some((parent, Some(direction)));
    }

    declaration.children.iter().copied().find_map(|child| {
        tree.get(child)
            .and_then(|child| child.path.name())
            .is_some_and(|name| name.as_str() == "directional")
            .then_some((child, None))
    })
}

pub(crate) fn directional_type_target(tree: &ObjectTree, selected: TypeId, direction: Dir) -> Option<TreePath> {
    let (group, _) = directional_type_group(tree, selected)?;

    tree.get(group)?.children.iter().find_map(|child| {
        let child = tree.get(*child)?;

        (child.path.name().and_then(|name| direction_from_name(name.as_str())) == Some(direction))
            .then(|| TreePath::parse(&child.path.to_string()))
    })
}

fn place(context: &mut ToolContext<'_>) -> Option<ToolEdit> {
    let prefab = context.prefab?.clone();

    place_on_tile(context.document, context.tree, context.coord, &prefab, None)
}

pub fn place_replacing_objs(
    document: &mut MapDocument, tree: &ObjectTree, coord: Coord, prefab: &Prefab, hidden: &HiddenTypes,
) -> Option<ToolEdit> {
    place_on_tile(document, tree, coord, prefab, Some(hidden))
}

fn place_on_tile(
    document: &mut MapDocument, tree: &ObjectTree, coord: Coord, prefab: &Prefab, replaced_objs: Option<&HiddenTypes>,
) -> Option<ToolEdit> {
    let kind = placement_kind(tree, prefab)?;
    let mut tile = document.placed_tile(coord)?;
    let mut affected = Vec::new();
    if let Some(hidden) = replaced_objs
        && is_obj(tree, &prefab.path)
    {
        tile.retain(|placed| {
            let replaced = is_obj(tree, &placed.prefab().path) && !hidden.hides(placed.prefab());
            if replaced {
                affected.push(placed.id());
            }
            !replaced
        });
    }
    let (after, picked, placed) = place_prefab_into(document, tree, coord, tile, prefab, kind)?;
    affected.extend(placed);

    let mut edit = Edit::new(format!("place {}", prefab.path));
    edit.change(document, coord, after);

    Some(ToolEdit {
        edit,
        picked: Some(picked),
        affected,
    })
}

fn is_obj(tree: &ObjectTree, path: &TreePath) -> bool {
    tree.roots()
        .obj
        .zip(tree.id_of(path))
        .is_some_and(|(obj, id)| tree.is_subtype_of(id, obj))
}

fn place_prefab_on_tile(
    document: &mut MapDocument, tree: &ObjectTree, coord: Coord, prefab: &Prefab, kind: PlacementKind,
) -> Option<(PlacedTile, PrefabInstanceId, Vec<PrefabInstanceId>)> {
    let tile = document.placed_tile(coord)?;

    place_prefab_into(document, tree, coord, tile, prefab, kind)
}

fn place_prefab_into(
    document: &mut MapDocument, tree: &ObjectTree, coord: Coord, mut after: PlacedTile, prefab: &Prefab,
    kind: PlacementKind,
) -> Option<(PlacedTile, PrefabInstanceId, Vec<PrefabInstanceId>)> {
    let mut affected = Vec::new();

    let picked = match kind {
        PlacementKind::Turf | PlacementKind::Area => {
            let matching = after
                .iter()
                .enumerate()
                .filter_map(|(index, placed)| (placement_kind(tree, placed.prefab()) == Some(kind)).then_some(index))
                .collect::<Vec<_>>();

            if let Some(first) = matching.first().copied() {
                let picked = after[first].id();
                *after[first].prefab_mut() = prefab.clone();
                affected.push(picked);
                for index in matching.into_iter().skip(1).rev() {
                    affected.push(after.remove(index).id());
                }

                picked
            } else {
                let placed = document.instantiate(prefab.clone());
                let picked = placed.id();
                let index = insertion_index(tree, &after, kind);
                after.insert(index, placed);
                affected.push(picked);

                picked
            }
        },
        PlacementKind::Atom => {
            let placed = document.instantiate(prefab.clone());
            let picked = placed.id();
            let index = insertion_index(tree, &after, kind);
            after.insert(index, placed);
            affected.push(picked);

            picked
        },
    };

    (document.placed_tile(coord).as_ref() != Some(&after)).then_some((after, picked, affected))
}

fn delete(context: &mut ToolContext<'_>) -> Option<ToolEdit> {
    let target = context.target?;

    delete_instances(context.document, context.tree, context.world, &[target], "delete")
}

pub fn delete_instances(
    document: &mut MapDocument, tree: &ObjectTree, world: &World, targets: &[PrefabInstanceId], verb: &str,
) -> Option<ToolEdit> {
    let mut targets_by_tile = BTreeMap::<Coord, Vec<PrefabInstanceId>>::new();
    for target in targets {
        let Some(location) = document.instance_location(*target) else {
            continue;
        };

        if !document.allows_edit_instance(*target) {
            continue;
        }

        targets_by_tile.entry(location.coord).or_default().push(*target);
    }

    let mut changes = Vec::new();
    let mut affected = Vec::new();
    let mut removed = Vec::new();
    for (coord, ids) in targets_by_tile {
        let Some(before) = document.placed_tile(coord) else {
            continue;
        };

        let mut after = before.clone();
        let mut removed_on_tile = Vec::new();
        for id in ids {
            if let Some(index) = after.iter().position(|placed| placed.id() == id) {
                removed_on_tile.push(after.remove(index));
            }
        }

        if removed_on_tile.is_empty() {
            continue;
        }

        let mut affected_on_tile = removed_on_tile.iter().map(PlacedPrefab::id).collect::<Vec<_>>();
        if let Some(base_tile) = &world.base_tile {
            affected_on_tile.extend(refill_missing_bases(document, tree, base_tile, &mut after));
        }

        if same_prefabs(&before, &after) {
            continue;
        }

        affected.extend(affected_on_tile);
        removed.extend(removed_on_tile);
        changes.push((coord, after));
    }

    let label = match removed.as_slice() {
        [] => return None,
        [placed] => format!("{verb} {}", placed.prefab().path),
        _ => format!("{verb} {} atoms", removed.len()),
    };

    let mut edit = Edit::new(label);
    for (coord, after) in changes {
        edit.change(document, coord, after);
    }

    Some(ToolEdit {
        edit,
        picked: None,
        affected,
    })
}

fn fill(
    context: &mut ToolContext<'_>, max_tiles: Option<usize>, mask: Option<SelectionMask>,
) -> Result<Option<ToolEdit>, FillError> {
    if !coord_in_bounds(context.coord, context.document.map.size())
        || !context.document.allows_edit_at(context.coord)
        || mask.is_some_and(|mask| !mask.includes(context.coord))
    {
        return Ok(None);
    }

    let Some(prefab) = context.prefab.cloned() else {
        return Ok(None);
    };
    let Some(prefab_id) = context.tree.id_of(&prefab.path) else {
        return Ok(None);
    };
    let Some(kind) = placement_kind(context.tree, &prefab) else {
        return Ok(None);
    };
    if kind == PlacementKind::Atom {
        return Ok(None);
    }

    let floor = context.tree.id_of(&TreePath::parse("/turf/open/floor"));
    let wall = context.tree.id_of(&TreePath::parse("/turf/closed"));
    let coords = match context.fill_mode {
        FillMode::Wall => match wall {
            Some(wall) => boundary_region(context, &[wall], mask),
            None => return Ok(None),
        },
        FillMode::EntireArea => area_region(context, mask),
        FillMode::Custom => custom_region(context, mask),
    };

    if coords.is_empty() {
        return Ok(None);
    }

    let preserve_walls = kind == PlacementKind::Turf
        && context.fill_mode == FillMode::EntireArea
        && floor.is_some_and(|floor| context.tree.is_subtype_of(prefab_id, floor));
    let mut coords = coords
        .into_iter()
        .filter(|coord| {
            !preserve_walls || !wall.is_some_and(|wall| tile_has_subtype(context.tree, context.document, *coord, wall))
        })
        .filter(|coord| would_replace_kind(context.document, context.tree, *coord, &prefab, kind))
        .collect::<Vec<_>>();

    if let Some(limit) = max_tiles
        && coords.len() > limit
    {
        return Err(FillError::TooLarge { limit });
    }

    if coords.is_empty() {
        return Ok(None);
    }

    let mut edit = Edit::new(format!("fill {}", prefab.path));
    let mut affected = Vec::new();

    for coord in coords.drain(..) {
        let Some((after, tile_affected)) = replace_kind(context.document, context.tree, coord, &prefab, kind) else {
            continue;
        };

        edit.change(context.document, coord, after);
        affected.extend(tile_affected);
    }

    if edit.is_empty() {
        return Ok(None);
    }

    Ok(Some(ToolEdit {
        edit,
        picked: None,
        affected,
    }))
}

pub(crate) fn coord_in_bounds(coord: Coord, size: dmm::Size) -> bool {
    (1..=size.x).contains(&coord.x) && (1..=size.y).contains(&coord.y) && (1..=size.z).contains(&coord.z)
}

fn custom_region(context: &ToolContext<'_>, mask: Option<SelectionMask>) -> Vec<Coord> {
    let boundaries = context
        .custom_fill_boundaries
        .iter()
        .filter_map(|path| context.tree.id_of(path))
        .collect::<Vec<_>>();

    if boundaries.is_empty() {
        return Vec::new();
    }

    boundary_region(context, &boundaries, mask)
}

fn boundary_region(context: &ToolContext<'_>, boundaries: &[TypeId], mask: Option<SelectionMask>) -> Vec<Coord> {
    let is_boundary = |coord| tile_has_any_subtype(context.tree, context.document, coord, boundaries);

    if is_boundary(context.coord) {
        return Vec::new();
    }

    let mut tiles = Vec::new();
    let mut pending = VecDeque::from([context.coord]);
    let mut visited = HashSet::from([context.coord]);

    while let Some(coord) = pending.pop_front() {
        if !context.document.allows_edit_at(coord)
            || mask.is_some_and(|mask| !mask.includes(coord))
            || is_boundary(coord)
        {
            continue;
        }

        tiles.push(coord);
        for neighbor in cardinal_neighbors(coord, context.document.map.size()) {
            if visited.insert(neighbor) {
                pending.push_back(neighbor);
            }
        }
    }

    tiles
}

fn area_region(context: &ToolContext<'_>, mask: Option<SelectionMask>) -> Vec<Coord> {
    let area = match context.tree.roots().area {
        Some(area) => area,
        None => return Vec::new(),
    };
    let Some(seed) = prefab_of_subtype(context.tree, &context.document.map, context.coord, area).cloned() else {
        return Vec::new();
    };

    let mut tiles = Vec::new();
    let mut pending = VecDeque::from([context.coord]);
    let mut visited = HashSet::from([context.coord]);

    while let Some(coord) = pending.pop_front() {
        if !context.document.allows_edit_at(coord)
            || mask.is_some_and(|mask| !mask.includes(coord))
            || !prefab_of_subtype(context.tree, &context.document.map, coord, area)
                .is_some_and(|candidate| crate::frame::same_area(&seed, candidate))
        {
            continue;
        }

        tiles.push(coord);
        for neighbor in cardinal_neighbors(coord, context.document.map.size()) {
            if visited.insert(neighbor) {
                pending.push_back(neighbor);
            }
        }
    }

    tiles
}

fn cardinal_neighbors(coord: Coord, size: dmm::Size) -> impl Iterator<Item = Coord> {
    [
        (coord.x > 1).then(|| Coord::new(coord.x - 1, coord.y, coord.z)),
        (coord.x < size.x).then(|| Coord::new(coord.x + 1, coord.y, coord.z)),
        (coord.y > 1).then(|| Coord::new(coord.x, coord.y - 1, coord.z)),
        (coord.y < size.y).then(|| Coord::new(coord.x, coord.y + 1, coord.z)),
    ]
    .into_iter()
    .flatten()
}

fn tile_has_subtype(tree: &ObjectTree, document: &MapDocument, coord: Coord, ancestor: TypeId) -> bool {
    prefab_of_subtype(tree, &document.map, coord, ancestor).is_some()
}

fn tile_has_any_subtype(tree: &ObjectTree, document: &MapDocument, coord: Coord, ancestors: &[TypeId]) -> bool {
    document.map.tile_at(coord).is_some_and(|tile| {
        tile.iter().any(|prefab| {
            tree.id_of(&prefab.path)
                .is_some_and(|id| ancestors.iter().any(|ancestor| tree.is_subtype_of(id, *ancestor)))
        })
    })
}

fn prefab_of_subtype<'a>(tree: &ObjectTree, map: &'a Grid, coord: Coord, ancestor: TypeId) -> Option<&'a Prefab> {
    map.tile_at(coord)?.iter().find(|prefab| {
        tree.id_of(&prefab.path)
            .is_some_and(|id| tree.is_subtype_of(id, ancestor))
    })
}

fn would_replace_kind(
    document: &MapDocument, tree: &ObjectTree, coord: Coord, prefab: &Prefab, kind: PlacementKind,
) -> bool {
    let Some(tile) = document.map.tile_at(coord) else {
        return false;
    };
    let mut matching = tile.iter().filter(|placed| placement_kind(tree, placed) == Some(kind));
    let Some(first) = matching.next() else {
        return true;
    };

    first != prefab || matching.next().is_some()
}

fn replace_kind(
    document: &mut MapDocument, tree: &ObjectTree, coord: Coord, prefab: &Prefab, kind: PlacementKind,
) -> Option<(crate::document::PlacedTile, Vec<PrefabInstanceId>)> {
    let mut after = document.placed_tile(coord)?;
    let matching = after
        .iter()
        .enumerate()
        .filter_map(|(index, placed)| (placement_kind(tree, placed.prefab()) == Some(kind)).then_some(index))
        .collect::<Vec<_>>();
    let mut affected = Vec::new();

    if let Some(first) = matching.first().copied() {
        let selected = after[first].id();
        *after[first].prefab_mut() = prefab.clone();
        affected.push(selected);
        for index in matching.into_iter().skip(1).rev() {
            affected.push(after.remove(index).id());
        }
    } else {
        let placed = document.instantiate(prefab.clone());
        let selected = placed.id();
        let index = insertion_index(tree, &after, kind);
        after.insert(index, placed);
        affected.push(selected);
    }

    (document.placed_tile(coord).as_ref() != Some(&after)).then_some((after, affected))
}

pub(crate) fn insertion_index(tree: &ObjectTree, tile: &[crate::document::PlacedPrefab], kind: PlacementKind) -> usize {
    let rank = placement_rank(kind);

    tile.iter()
        .position(|placed| {
            placement_kind(tree, placed.prefab())
                .map(placement_rank)
                .is_some_and(|candidate| candidate > rank)
        })
        .unwrap_or(tile.len())
}

fn placement_rank(kind: PlacementKind) -> u8 {
    match kind {
        PlacementKind::Atom => 0,
        PlacementKind::Turf => 1,
        PlacementKind::Area => 2,
    }
}

pub fn placement_kind(tree: &ObjectTree, prefab: &Prefab) -> Option<PlacementKind> {
    let id = tree.id_of(&prefab.path)?;
    let roots = tree.roots();
    let atom = roots.atom?;
    if !tree.is_subtype_of(id, atom) {
        return None;
    }
    if roots.turf.is_some_and(|turf| tree.is_subtype_of(id, turf)) {
        return Some(PlacementKind::Turf);
    }
    if roots.area.is_some_and(|area| tree.is_subtype_of(id, area)) {
        return Some(PlacementKind::Area);
    }

    Some(PlacementKind::Atom)
}

pub fn is_placeable(tree: &ObjectTree, id: TypeId) -> bool {
    tree.roots().atom.is_some_and(|atom| tree.is_subtype_of(id, atom))
}

#[cfg(test)]
mod tests;
