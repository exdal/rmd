use core::{
    path::TreePath,
    types::{Identifier, Value},
};
use std::collections::{HashMap, HashSet, VecDeque};

use dmi::metadata::Dir;
use dmm::{Coord, Prefab};
use objtree::{ObjectTree, TypeId};

use crate::{
    command::Edit,
    document::{MapDocument, PlacedPrefab, PlacedTile, PrefabInstanceId, Selection},
    visual,
};

pub const MAX_FILL_TILES: usize = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tool {
    Place,
    #[default]
    Select,
    BlockSelect,
    Delete,
    Fill,
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
pub enum BlockSelectionMode {
    #[default]
    Full,
    Hollow {
        line_width: u32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectionMask {
    pub bounds: Selection,
    pub mode: BlockSelectionMode,
}

impl SelectionMask {
    pub fn includes(self, coord: Coord) -> bool { self.mode.includes(self.bounds, coord) }
}

impl BlockSelectionMode {
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
    pub prefab: Option<&'a Prefab>,
    pub target: Option<PrefabInstanceId>,
    pub coord: Coord,
    pub anchor: Option<Coord>,
    pub fill_mode: FillMode,
    pub custom_fill_boundaries: &'a [TreePath],
}

pub struct ToolEdit {
    pub edit: Edit,
    pub selected: Option<PrefabInstanceId>,
    pub affected: Vec<PrefabInstanceId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillError {
    TooLarge { limit: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlacementKind {
    Atom,
    Turf,
    Area,
}

impl Tool {
    pub fn label(self) -> &'static str {
        match self {
            Tool::Place => "Place",
            Tool::Select => "Select",
            Tool::BlockSelect => "Block Select",
            Tool::Delete => "Delete",
            Tool::Fill => "Fill",
        }
    }

    pub fn build_edit(self, context: &mut ToolContext<'_>) -> Option<ToolEdit> {
        match self {
            Self::Place => place(context),
            Self::Delete => delete(context),
            Self::Select | Self::BlockSelect => None,
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

pub fn move_selection(
    document: &mut MapDocument, tree: &ObjectTree, selection: Selection, target_min: Coord,
) -> Option<(ToolEdit, Selection)> {
    place_selection(
        document,
        tree,
        selection,
        target_min,
        SelectionRotation::Original,
        SelectionPlacement::Move,
    )
}

pub fn copy_selection(
    document: &mut MapDocument, tree: &ObjectTree, selection: Selection, target_min: Coord,
) -> Option<(ToolEdit, Selection)> {
    place_selection(
        document,
        tree,
        selection,
        target_min,
        SelectionRotation::Original,
        SelectionPlacement::Copy,
    )
}

pub fn fill_selection(
    document: &mut MapDocument, tree: &ObjectTree, selection: Selection, prefab: &Prefab, mode: BlockSelectionMode,
) -> Option<ToolEdit> {
    if !selection.is_well_formed()
        || !coord_in_bounds(selection.min, document.map.size)
        || !coord_in_bounds(selection.max, document.map.size)
        || mode.tiles(selection).any(|coord| !document.allows_edit_at(coord))
    {
        return None;
    }

    let kind = placement_kind(tree, prefab)?;
    let mut edit = Edit::new(format!("fill block with {}", prefab.path));
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
        selected: None,
        affected,
    })
}

pub fn place_selection(
    document: &mut MapDocument, tree: &ObjectTree, selection: Selection, target_min: Coord,
    rotation: SelectionRotation, placement: SelectionPlacement,
) -> Option<(ToolEdit, Selection)> {
    place_selection_with_mode(
        document,
        tree,
        selection,
        target_min,
        rotation,
        placement,
        BlockSelectionMode::Full,
    )
}

pub fn place_selection_with_mode(
    document: &mut MapDocument, tree: &ObjectTree, selection: Selection, target_min: Coord,
    rotation: SelectionRotation, placement: SelectionPlacement, mode: BlockSelectionMode,
) -> Option<(ToolEdit, Selection)> {
    let target = rotated_selection_at(selection, target_min, rotation)?;
    if target == selection && rotation == SelectionRotation::Original {
        return None;
    }
    let label = match placement {
        SelectionPlacement::Move => "move block",
        SelectionPlacement::Copy => "copy block",
    };

    build_selection_edit(
        document,
        tree,
        SelectionEditRequest {
            source: selection,
            target,
            transforms: rotation.transforms(),
            placement,
            mode,
            label,
        },
    )
}

pub fn transform_selection(
    document: &mut MapDocument, tree: &ObjectTree, selection: Selection, transform: SelectionTransform,
) -> Option<(ToolEdit, Selection)> {
    transform_selection_with_mode(document, tree, selection, transform, BlockSelectionMode::Full)
}

pub fn transform_selection_with_mode(
    document: &mut MapDocument, tree: &ObjectTree, selection: Selection, transform: SelectionTransform,
    mode: BlockSelectionMode,
) -> Option<(ToolEdit, Selection)> {
    if !selection.is_well_formed() {
        return None;
    }
    let target = transformed_selection(selection, transform)?;
    let label = match transform {
        SelectionTransform::RotateClockwise => "rotate block clockwise",
        SelectionTransform::RotateCounterClockwise => "rotate block counterclockwise",
        SelectionTransform::MirrorHorizontal => "mirror block horizontally",
        SelectionTransform::MirrorVertical => "mirror block vertically",
    };

    build_selection_edit(
        document,
        tree,
        SelectionEditRequest {
            source: selection,
            target,
            transforms: std::slice::from_ref(&transform),
            placement: SelectionPlacement::Move,
            mode,
            label,
        },
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
    mode: BlockSelectionMode,
    label: &'a str,
}

impl SelectionEditRequest<'_> {
    fn source_tiles(&self) -> impl Iterator<Item = Coord> { self.mode.tiles(self.source) }

    fn touched_tiles(&self) -> impl Iterator<Item = Coord> { self.source_tiles().chain(self.mode.tiles(self.target)) }

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
    document: &mut MapDocument, tree: &ObjectTree, request: SelectionEditRequest<'_>,
) -> Option<(ToolEdit, Selection)> {
    let size = document.map.size;
    let touched = request.touched_tiles().collect::<HashSet<_>>();
    if touched
        .iter()
        .any(|coord| !coord_in_bounds(*coord, size) || !document.allows_edit_at(*coord))
    {
        return None;
    }

    let defaults = if request.placement == SelectionPlacement::Move {
        Some(default_tile_paths(tree)?)
    } else {
        None
    };
    let mut payload = Vec::with_capacity(touched.len());
    for coord in request.source_tiles() {
        let destination = request.destination_of(coord);
        let mut tile = document.placed_tile(coord)?;
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
    if let Some((default_turf, default_area)) = defaults {
        for coord in request.source_tiles() {
            staged.insert(
                coord,
                vec![
                    document.instantiate(Prefab::new(default_turf.clone())),
                    document.instantiate(Prefab::new(default_area.clone())),
                ],
            );
        }
    }
    for (coord, incoming) in payload {
        staged.insert(coord, incoming);
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
            selected: None,
            affected,
        },
        request.target,
    ))
}

pub fn default_tile_paths(tree: &ObjectTree) -> Option<(TreePath, TreePath)> {
    Some((
        default_path(tree, "turf", tree.roots().turf?)?,
        default_path(tree, "area", tree.roots().area?)?,
    ))
}

fn default_path(tree: &ObjectTree, variable: &str, root: TypeId) -> Option<TreePath> {
    let configured = tree
        .id_of(&TreePath::parse("/world"))
        .and_then(|world| tree.var_inherited(world, &Identifier::from(variable)))
        .and_then(|var| match &var.value {
            Value::Path(path) => Some(path),
            _ => None,
        })
        .and_then(|path| tree.id_of(path).map(|id| (path, id)))
        .filter(|(_, id)| tree.is_subtype_of(*id, root))
        .map(|(path, _)| path.clone());

    configured.or_else(|| tree.get(root).map(|decl| TreePath::parse(&decl.path.to_string())))
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
    let dir_name = Identifier::from("dir");
    let direction = path_direction.or_else(|| match visual::resolve_value(tree, id, prefab, &dir_name) {
        Some(resolved) => resolved.value.as_num().and_then(|value| Dir::from_bits(value as u32)),
        None => Dir::from_bits(appearance.dir),
    });
    let target_direction = direction.map(|direction| transform_direction(direction, transform));

    if let Some(direction) = target_direction {
        if let Some(path) = directional_type_target(tree, id, direction) {
            prefab.path = path;
            prefab.remove_var(&Identifier::from("dir"));
        } else {
            set_resolved_number(tree, prefab, "dir", direction.to_bits() as i32);
        }
    }

    for (x_name, y_name, values) in [
        ("pixel_x", "pixel_y", [appearance.pixel_x, appearance.pixel_y]),
        ("pixel_w", "pixel_z", [appearance.pixel_w, appearance.pixel_z]),
        ("step_x", "step_y", [appearance.step_x, appearance.step_y]),
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
            "dir" => appearance.dir as i32,
            "pixel_x" => appearance.pixel_x,
            "pixel_y" => appearance.pixel_y,
            "pixel_w" => appearance.pixel_w,
            "pixel_z" => appearance.pixel_z,
            "step_x" => appearance.step_x,
            "step_y" => appearance.step_y,
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

fn direction_from_name(name: &str) -> Option<Dir> {
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

fn directional_type_group(tree: &ObjectTree, selected: TypeId) -> Option<(TypeId, Option<Dir>)> {
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

fn directional_type_target(tree: &ObjectTree, selected: TypeId, direction: Dir) -> Option<TreePath> {
    let (group, _) = directional_type_group(tree, selected)?;

    tree.get(group)?.children.iter().find_map(|child| {
        let child = tree.get(*child)?;

        (child.path.name().and_then(|name| direction_from_name(name.as_str())) == Some(direction))
            .then(|| TreePath::parse(&child.path.to_string()))
    })
}

fn place(context: &mut ToolContext<'_>) -> Option<ToolEdit> {
    let prefab = context.prefab?.clone();
    let kind = placement_kind(context.tree, &prefab)?;
    let (after, selected, affected) =
        place_prefab_on_tile(context.document, context.tree, context.coord, &prefab, kind)?;

    let mut edit = Edit::new(format!("place {}", prefab.path));
    edit.change(context.document, context.coord, after);

    Some(ToolEdit {
        edit,
        selected: Some(selected),
        affected,
    })
}

fn place_prefab_on_tile(
    document: &mut MapDocument, tree: &ObjectTree, coord: Coord, prefab: &Prefab, kind: PlacementKind,
) -> Option<(PlacedTile, PrefabInstanceId, Vec<PrefabInstanceId>)> {
    let mut after = document.placed_tile(coord)?;
    let mut affected = Vec::new();

    let selected = match kind {
        PlacementKind::Turf | PlacementKind::Area => {
            let matching = after
                .iter()
                .enumerate()
                .filter_map(|(index, placed)| (placement_kind(tree, placed.prefab()) == Some(kind)).then_some(index))
                .collect::<Vec<_>>();

            if let Some(first) = matching.first().copied() {
                let selected = after[first].id();
                *after[first].prefab_mut() = prefab.clone();
                affected.push(selected);
                for index in matching.into_iter().skip(1).rev() {
                    affected.push(after.remove(index).id());
                }

                selected
            } else {
                let placed = document.instantiate(prefab.clone());
                let selected = placed.id();
                let index = insertion_index(tree, &after, kind);
                after.insert(index, placed);
                affected.push(selected);

                selected
            }
        },
        PlacementKind::Atom => {
            let placed = document.instantiate(prefab.clone());
            let selected = placed.id();
            let index = insertion_index(tree, &after, kind);
            after.insert(index, placed);
            affected.push(selected);

            selected
        },
    };

    (document.placed_tile(coord).as_ref() != Some(&after)).then_some((after, selected, affected))
}

fn delete(context: &mut ToolContext<'_>) -> Option<ToolEdit> {
    let target = context.target?;
    let location = context.document.instance_location(target)?;
    if !context.document.allows_edit_at(location.coord) {
        return None;
    }
    let mut after = context.document.placed_tile(location.coord)?;
    if after
        .get(location.prefab_index)
        .is_none_or(|placed| placed.id() != target)
    {
        return None;
    }
    let placed = after.remove(location.prefab_index);

    let mut edit = Edit::new(format!("delete {}", placed.prefab().path));
    edit.change(context.document, location.coord, after);

    Some(ToolEdit {
        edit,
        selected: None,
        affected: vec![target],
    })
}

fn fill(
    context: &mut ToolContext<'_>, max_tiles: Option<usize>, mask: Option<SelectionMask>,
) -> Result<Option<ToolEdit>, FillError> {
    if !coord_in_bounds(context.coord, context.document.map.size)
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
        selected: None,
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

    let mut region = Vec::new();
    let mut pending = VecDeque::from([context.coord]);
    let mut visited = HashSet::from([context.coord]);

    while let Some(coord) = pending.pop_front() {
        if !context.document.allows_edit_at(coord)
            || mask.is_some_and(|mask| !mask.includes(coord))
            || is_boundary(coord)
        {
            continue;
        }

        region.push(coord);
        for neighbor in cardinal_neighbors(coord, context.document.map.size) {
            if visited.insert(neighbor) {
                pending.push_back(neighbor);
            }
        }
    }

    region
}

fn area_region(context: &ToolContext<'_>, mask: Option<SelectionMask>) -> Vec<Coord> {
    let area = match context.tree.roots().area {
        Some(area) => area,
        None => return Vec::new(),
    };
    let Some(seed) = prefab_of_subtype(context.tree, &context.document.map, context.coord, area).cloned() else {
        return Vec::new();
    };

    let mut region = Vec::new();
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

        region.push(coord);
        for neighbor in cardinal_neighbors(coord, context.document.map.size) {
            if visited.insert(neighbor) {
                pending.push_back(neighbor);
            }
        }
    }

    region
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

fn prefab_of_subtype<'a>(tree: &ObjectTree, map: &'a dmm::Map, coord: Coord, ancestor: TypeId) -> Option<&'a Prefab> {
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

fn insertion_index(tree: &ObjectTree, tile: &[crate::document::PlacedPrefab], kind: PlacementKind) -> usize {
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

fn placement_kind(tree: &ObjectTree, prefab: &Prefab) -> Option<PlacementKind> {
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
mod tests {
    use core::{
        location::Location,
        path::TreePath,
        types::{Value, VarModifiers},
    };
    use std::collections::{HashMap, HashSet};

    use dmm::{Coord, Map, Prefab, Size};
    use objtree::VarDecl;

    use super::{
        BlockSelectionMode,
        Dir,
        FillError,
        FillMode,
        MAX_FILL_TILES,
        Selection,
        SelectionMask,
        SelectionPlacement,
        SelectionRotation,
        SelectionTransform,
        Tool,
        ToolContext,
        copy_selection,
        default_tile_paths,
        fill_selection,
        move_selection,
        place_selection,
        place_selection_with_mode,
        rotate_point,
        transform_direction,
        transform_point,
        transform_selection,
        transform_selection_with_mode,
        transformed_selection,
    };
    use crate::{document::MapDocument, focus::AreaFocus};

    fn tree() -> objtree::ObjectTree {
        let mut tree = objtree::ObjectTree::new();
        for path in [
            "/atom",
            "/atom/movable",
            "/obj",
            "/obj/table",
            "/obj/chair",
            "/obj/sign",
            "/obj/sign/directional",
            "/obj/sign/directional/north",
            "/obj/sign/directional/south",
            "/obj/sign/directional/east",
            "/obj/sign/directional/west",
            "/obj/window",
            "/obj/window/reinforced",
            "/obj/machinery/door/airlock",
            "/turf",
            "/turf/floor",
            "/turf/wall",
            "/turf/open/floor",
            "/turf/open/floor/blue",
            "/turf/open/space",
            "/turf/closed/wall",
            "/turf/closed/wall/reinforced",
            "/turf/closed/reinforced",
            "/area",
            "/area/station",
            "/area/space",
            "/datum",
        ] {
            tree.register(&TreePath::parse(path), Location::default());
        }
        for (path, parent) in [
            ("/atom/movable", "/atom"),
            ("/obj", "/atom/movable"),
            ("/turf", "/atom"),
            ("/area", "/atom"),
        ] {
            let id = tree.id_of(&TreePath::parse(path)).unwrap();
            tree.get_mut(id).unwrap().parent_type = Some(TreePath::parse(parent));
        }
        tree.resolve_parent_types();

        tree
    }

    fn map_document(paths: &[&str]) -> MapDocument {
        let mut map = Map::new(Size { x: 1, y: 1, z: 1 });
        let key = map.intern_tile(paths.iter().map(|path| Prefab::new(TreePath::parse(path))).collect());
        map.grid[0][0][0] = key;

        MapDocument::new(map, 1)
    }

    fn grid_document(width: u32, height: u32, mut tile_at: impl FnMut(Coord) -> Vec<Prefab>) -> MapDocument {
        let mut map = Map::new(Size {
            x: width,
            y: height,
            z: 1,
        });
        for y in 1..=height {
            for x in 1..=width {
                let coord = Coord::new(x, y, 1);
                let key = map.intern_tile(tile_at(coord));
                map.grid[0][(height - y) as usize][(x - 1) as usize] = key;
            }
        }

        MapDocument::new(map, 1)
    }

    fn prefabs(paths: &[&str]) -> Vec<Prefab> { paths.iter().map(|path| Prefab::new(TreePath::parse(path))).collect() }

    fn tile_paths(document: &MapDocument, coord: Coord) -> Vec<String> {
        document
            .map
            .tile_at(coord)
            .unwrap()
            .iter()
            .map(|prefab| prefab.path.to_string())
            .collect()
    }

    fn turf_at(document: &MapDocument, coord: Coord) -> &Prefab {
        document
            .map
            .tile_at(coord)
            .unwrap()
            .iter()
            .find(|prefab| prefab.path.to_string().starts_with("/turf"))
            .unwrap()
    }

    fn area_at(document: &MapDocument, coord: Coord) -> &Prefab {
        document
            .map
            .tile_at(coord)
            .unwrap()
            .iter()
            .find(|prefab| prefab.path.to_string().starts_with("/area"))
            .unwrap()
    }

    fn fill(
        document: &mut MapDocument, tree: &objtree::ObjectTree, prefab: &Prefab, coord: Coord, fill_mode: FillMode,
    ) -> Option<super::ToolEdit> {
        fill_with_boundaries(document, tree, prefab, coord, fill_mode, &[])
    }

    fn fill_with_boundaries(
        document: &mut MapDocument, tree: &objtree::ObjectTree, prefab: &Prefab, coord: Coord, fill_mode: FillMode,
        custom_fill_boundaries: &[TreePath],
    ) -> Option<super::ToolEdit> {
        Tool::Fill.build_edit(&mut ToolContext {
            document,
            tree,
            prefab: Some(prefab),
            target: None,
            coord,
            anchor: None,
            fill_mode,
            custom_fill_boundaries,
        })
    }

    fn fill_with_limit(
        document: &mut MapDocument, tree: &objtree::ObjectTree, prefab: &Prefab, coord: Coord, fill_mode: FillMode,
        max_tiles: Option<usize>,
    ) -> Result<Option<super::ToolEdit>, FillError> {
        Tool::Fill.build_fill_edit(
            &mut ToolContext {
                document,
                tree,
                prefab: Some(prefab),
                target: None,
                coord,
                anchor: None,
                fill_mode,
                custom_fill_boundaries: &[],
            },
            max_tiles,
        )
    }

    fn place(document: &mut MapDocument, tree: &objtree::ObjectTree, prefab: &Prefab) -> Option<super::ToolEdit> {
        Tool::Place.build_edit(&mut ToolContext {
            document,
            tree,
            prefab: Some(prefab),
            target: None,
            coord: dmm::Coord::new(1, 1, 1),
            anchor: None,
            fill_mode: FillMode::default(),
            custom_fill_boundaries: &[],
        })
    }

    fn scoped_fill(
        document: &mut MapDocument, coord: Coord, fill_mode: FillMode, mask: SelectionMask, limit: Option<usize>,
    ) -> Result<Option<super::ToolEdit>, FillError> {
        Tool::Fill.build_fill_edit_with_mask(
            &mut ToolContext {
                document,
                tree: &tree(),
                prefab: Some(&Prefab::new(TreePath::parse("/turf/open/floor/blue"))),
                target: None,
                coord,
                anchor: None,
                fill_mode,
                custom_fill_boundaries: &[TreePath::parse("/obj/window")],
            },
            limit,
            Some(mask),
        )
    }

    #[test]
    fn every_bucket_mode_obeys_full_and_hollow_selection_masks() {
        for fill_mode in [FillMode::Wall, FillMode::EntireArea, FillMode::Custom] {
            for mode in [
                BlockSelectionMode::Full,
                BlockSelectionMode::Hollow { line_width: 1 },
                BlockSelectionMode::Hollow { line_width: 2 },
            ] {
                let mut document = grid_document(7, 7, |_| prefabs(&["/turf/open/floor", "/area/station"]));
                let mask = SelectionMask {
                    bounds: Selection::from_drag(Coord::new(2, 2, 1), Coord::new(6, 6, 1)),
                    mode,
                };
                let action = scoped_fill(&mut document, mask.bounds.min, fill_mode, mask, None)
                    .unwrap()
                    .unwrap();
                let actual = action
                    .edit
                    .changes
                    .iter()
                    .map(|change| change.coord)
                    .collect::<HashSet<_>>();
                assert_eq!(actual, mode.tiles(mask.bounds).collect(), "{fill_mode:?}, {mode:?}");
                assert!(
                    scoped_fill(&mut document, Coord::new(1, 1, 1), fill_mode, mask, None)
                        .unwrap()
                        .is_none()
                );
                if mode != BlockSelectionMode::Full {
                    assert!(
                        scoped_fill(&mut document, Coord::new(4, 4, 1), fill_mode, mask, None)
                            .unwrap()
                            .is_none()
                    );
                }
            }
        }
    }

    #[test]
    fn bucket_traversal_cannot_leave_the_selection_to_reach_a_disconnected_part() {
        for fill_mode in [FillMode::Wall, FillMode::EntireArea, FillMode::Custom] {
            let mut document = grid_document(5, 5, |coord| {
                if coord.x == 3 && (2..=4).contains(&coord.y) {
                    match fill_mode {
                        FillMode::Wall => prefabs(&["/turf/closed/wall", "/area/station"]),
                        FillMode::EntireArea => prefabs(&["/turf/open/floor", "/area/space"]),
                        FillMode::Custom => prefabs(&["/turf/open/floor", "/area/station", "/obj/window"]),
                    }
                } else {
                    prefabs(&["/turf/open/floor", "/area/station"])
                }
            });
            let mask = SelectionMask {
                bounds: Selection::from_drag(Coord::new(2, 2, 1), Coord::new(4, 4, 1)),
                mode: BlockSelectionMode::Full,
            };
            let action = scoped_fill(&mut document, mask.bounds.min, fill_mode, mask, None)
                .unwrap()
                .unwrap();
            assert_eq!(
                action
                    .edit
                    .changes
                    .iter()
                    .map(|change| change.coord)
                    .collect::<HashSet<_>>(),
                (2..=4).map(|y| Coord::new(2, y, 1)).collect(),
                "{fill_mode:?}"
            );
        }
    }

    #[test]
    fn scoped_fill_limit_counts_only_selected_changes_and_never_partially_applies() {
        let mut document = grid_document(80, 80, |_| prefabs(&["/turf/open/floor", "/area/station"]));
        let mask = SelectionMask {
            bounds: Selection::from_drag(Coord::new(2, 2, 1), Coord::new(6, 6, 1)),
            mode: BlockSelectionMode::Full,
        };
        assert!(matches!(
            scoped_fill(&mut document, mask.bounds.min, FillMode::Wall, mask, Some(24)),
            Err(FillError::TooLarge { limit: 24 })
        ));
        assert_eq!(
            turf_at(&document, mask.bounds.min).path,
            TreePath::parse("/turf/open/floor")
        );
        let action = scoped_fill(
            &mut document,
            mask.bounds.min,
            FillMode::Wall,
            mask,
            Some(MAX_FILL_TILES),
        )
        .unwrap()
        .unwrap();
        assert_eq!(action.edit.changes.len(), 25);
        document.apply(action.edit);
        assert_eq!(
            turf_at(&document, Coord::new(1, 1, 1)).path,
            TreePath::parse("/turf/open/floor")
        );
        assert!(document.undo());
        assert_eq!(
            turf_at(&document, mask.bounds.min).path,
            TreePath::parse("/turf/open/floor")
        );
    }

    #[test]
    fn objects_append_before_turf_and_area_with_exact_overrides() {
        let tree = tree();
        let mut document = map_document(&["/obj/table", "/turf/floor", "/area/station"]);
        let mut chair = Prefab::new(TreePath::parse("/obj/chair"));
        chair.set_var("name".into(), Value::Text("custom".into()));
        let action = place(&mut document, &tree, &chair).unwrap();
        let selected = action.selected.unwrap();
        document.apply(action.edit);

        let paths = document
            .map
            .tile_at(dmm::Coord::new(1, 1, 1))
            .unwrap()
            .iter()
            .map(|prefab| prefab.path.to_string())
            .collect::<Vec<_>>();
        assert_eq!(paths, ["/obj/table", "/obj/chair", "/turf/floor", "/area/station"]);
        assert_eq!(
            document.prefab_instance(selected).unwrap().0.var(&"name".into()),
            Some(&Value::Text("custom".into()))
        );
    }

    #[test]
    fn turf_and_area_replace_their_category_and_keep_the_first_id() {
        let tree = tree();
        let mut document = map_document(&["/turf/floor", "/turf/wall", "/area/station"]);
        let turf_id = document.instance_ids_at(dmm::Coord::new(1, 1, 1))[0];
        let before = document.placed_tile(dmm::Coord::new(1, 1, 1)).unwrap();
        let action = place(&mut document, &tree, &Prefab::new(TreePath::parse("/turf/wall"))).unwrap();
        assert_eq!(action.selected, Some(turf_id));
        assert_eq!(action.affected.len(), 2);
        document.apply(action.edit);

        let tile = document.map.tile_at(dmm::Coord::new(1, 1, 1)).unwrap();
        assert_eq!(tile.len(), 2);
        assert_eq!(tile[0].path, TreePath::parse("/turf/wall"));
        assert_eq!(document.instance_ids_at(dmm::Coord::new(1, 1, 1))[0], turf_id);
        assert!(document.undo());
        assert_eq!(document.placed_tile(dmm::Coord::new(1, 1, 1)).unwrap(), before);

        let mut document = map_document(&["/obj/table", "/turf/floor", "/area/station"]);
        let area_id = document.instance_ids_at(dmm::Coord::new(1, 1, 1))[2];
        let action = place(&mut document, &tree, &Prefab::new(TreePath::parse("/area/space"))).unwrap();
        assert_eq!(action.selected, Some(area_id));
        document.apply(action.edit);
        let tile = document.map.tile_at(dmm::Coord::new(1, 1, 1)).unwrap();
        assert_eq!(tile[2].path, TreePath::parse("/area/space"));
        assert_eq!(document.instance_ids_at(dmm::Coord::new(1, 1, 1))[2], area_id);
    }

    #[test]
    fn identical_category_replacement_is_a_noop_and_datums_are_rejected() {
        let tree = tree();
        let mut document = map_document(&["/turf/floor", "/area/station"]);

        assert!(place(&mut document, &tree, &Prefab::new(TreePath::parse("/turf/floor"))).is_none());
        assert!(place(&mut document, &tree, &Prefab::new(TreePath::parse("/datum"))).is_none());
    }

    #[test]
    fn delete_removes_only_the_target_and_undo_restores_its_id() {
        let tree = tree();
        let coord = dmm::Coord::new(1, 1, 1);
        let mut document = map_document(&["/obj/table", "/turf/floor", "/area/station"]);
        let target = document.instance_ids_at(coord)[0];
        document.select_instance(Some(target));

        let action = Tool::Delete
            .build_edit(&mut ToolContext {
                document: &mut document,
                tree: &tree,
                prefab: None,
                target: Some(target),
                coord,
                anchor: None,
                fill_mode: FillMode::default(),
                custom_fill_boundaries: &[],
            })
            .unwrap();
        assert_eq!(action.selected, None);
        assert_eq!(action.affected, [target]);
        document.apply(action.edit);

        let paths = document
            .map
            .tile_at(coord)
            .unwrap()
            .iter()
            .map(|prefab| prefab.path.to_string())
            .collect::<Vec<_>>();
        assert_eq!(paths, ["/turf/floor", "/area/station"]);
        assert_eq!(document.selected_instance(), None);
        assert_eq!(document.instance_location(target), None);

        assert!(
            Tool::Delete
                .build_edit(&mut ToolContext {
                    document: &mut document,
                    tree: &tree,
                    prefab: None,
                    target: Some(target),
                    coord,
                    anchor: None,
                    fill_mode: FillMode::default(),
                    custom_fill_boundaries: &[],
                })
                .is_none()
        );

        assert!(document.undo());
        assert_eq!(document.instance_ids_at(coord)[0], target);
        assert!(document.redo());
        assert_eq!(document.instance_location(target), None);
    }

    #[test]
    fn wall_fill_replaces_the_interior_and_preserves_boundaries_objects_areas_and_ids() {
        let tree = tree();
        let mut document = grid_document(5, 5, |coord| {
            let boundary = coord.x == 1 || coord.x == 5 || coord.y == 1 || coord.y == 5;
            let mut paths = if coord == Coord::new(3, 3, 1) {
                vec!["/obj/table"]
            } else {
                Vec::new()
            };
            paths.push(if boundary {
                "/turf/closed/wall"
            } else {
                "/turf/open/floor"
            });
            if coord == Coord::new(3, 3, 1) {
                paths.push("/turf/open/space");
            }
            paths.push("/area/station");

            prefabs(&paths)
        });
        let interior = (2..=4)
            .flat_map(|y| (2..=4).map(move |x| Coord::new(x, y, 1)))
            .collect::<Vec<_>>();
        let before = interior
            .iter()
            .map(|coord| (*coord, document.placed_tile(*coord).unwrap()))
            .collect::<Vec<_>>();
        let turf_ids = interior
            .iter()
            .map(|coord| {
                let tile = document.map.tile_at(*coord).unwrap();
                tile.iter()
                    .zip(document.instance_ids_at(*coord))
                    .find_map(|(prefab, id)| prefab.path.to_string().starts_with("/turf").then_some(*id))
                    .unwrap()
            })
            .collect::<Vec<_>>();
        let mut blue = Prefab::new(TreePath::parse("/turf/open/floor/blue"));
        blue.set_var("name".into(), Value::Text("custom blue".into()));

        let action = fill(&mut document, &tree, &blue, Coord::new(3, 3, 1), FillMode::Wall).unwrap();
        assert_eq!(action.selected, None);
        assert_eq!(action.edit.changes.len(), 9);
        assert_eq!(action.affected.len(), 10);
        document.apply(action.edit);

        for (index, coord) in interior.iter().enumerate() {
            assert_eq!(turf_at(&document, *coord), &blue);
            assert!(document.instance_ids_at(*coord).contains(&turf_ids[index]));
        }
        assert_eq!(
            turf_at(&document, Coord::new(1, 3, 1)).path,
            TreePath::parse("/turf/closed/wall")
        );
        let center_paths = document
            .map
            .tile_at(Coord::new(3, 3, 1))
            .unwrap()
            .iter()
            .map(|prefab| prefab.path.to_string())
            .collect::<Vec<_>>();
        assert_eq!(center_paths, ["/obj/table", "/turf/open/floor/blue", "/area/station"]);

        assert!(document.undo());
        for (coord, tile) in before {
            assert_eq!(document.placed_tile(coord), Some(tile));
        }
        assert!(!document.undo());
        assert!(document.redo());
        assert_eq!(turf_at(&document, Coord::new(3, 3, 1)), &blue);
    }

    #[test]
    fn wall_fill_may_reach_the_map_edge() {
        let tree = tree();
        let mut document = grid_document(3, 2, |coord| {
            if coord == Coord::new(2, 1, 1) {
                prefabs(&["/area/space"])
            } else {
                prefabs(&["/turf/open/space", "/area/space"])
            }
        });
        let floor = Prefab::new(TreePath::parse("/turf/open/floor"));

        let action = fill(&mut document, &tree, &floor, Coord::new(1, 1, 1), FillMode::Wall).unwrap();
        assert_eq!(action.edit.changes.len(), 6);
        document.apply(action.edit);

        for y in 1..=2 {
            for x in 1..=3 {
                assert_eq!(turf_at(&document, Coord::new(x, y, 1)), &floor);
            }
        }
    }

    #[test]
    fn wall_fill_only_uses_closed_turfs_as_boundaries() {
        let tree = tree();
        let window = Coord::new(3, 1, 1);
        let airlock = Coord::new(5, 3, 1);
        let solid_override = Coord::new(3, 5, 1);
        let mut document = grid_document(5, 5, |coord| {
            let boundary = coord.x == 1 || coord.x == 5 || coord.y == 1 || coord.y == 5;
            let mut tile = Vec::new();
            if coord == window {
                tile.push(Prefab::new(TreePath::parse("/obj/window")));
            } else if coord == airlock {
                tile.push(Prefab::new(TreePath::parse("/obj/machinery/door/airlock")));
            } else if coord == solid_override {
                let mut table = Prefab::new(TreePath::parse("/obj/table"));
                table.set_var("density".into(), Value::Num(1.0));
                tile.push(table);
            }
            tile.push(Prefab::new(TreePath::parse(if boundary && tile.is_empty() {
                "/turf/closed/wall"
            } else {
                "/turf/open/floor"
            })));
            tile.push(Prefab::new(TreePath::parse("/area/station")));

            tile
        });
        let blue = Prefab::new(TreePath::parse("/turf/open/floor/blue"));

        let action = fill(&mut document, &tree, &blue, Coord::new(3, 3, 1), FillMode::Wall).unwrap();
        assert_eq!(action.edit.changes.len(), 12);
        document.apply(action.edit);

        for coord in [window, airlock, solid_override] {
            assert_eq!(turf_at(&document, coord), &blue);
        }
    }

    #[test]
    fn custom_fill_uses_configured_types_and_their_subtypes_as_boundaries() {
        let tree = tree();
        let window = Coord::new(3, 1, 1);
        let mut document = grid_document(5, 5, |coord| {
            let boundary = coord.x == 1 || coord.x == 5 || coord.y == 1 || coord.y == 5;
            if coord == window {
                prefabs(&["/obj/window/reinforced", "/turf/open/floor", "/area/station"])
            } else {
                prefabs(&[
                    if boundary {
                        "/turf/closed/wall/reinforced"
                    } else {
                        "/turf/open/floor"
                    },
                    "/area/station",
                ])
            }
        });
        let blue = Prefab::new(TreePath::parse("/turf/open/floor/blue"));
        let boundaries = [TreePath::parse("/turf/closed/wall"), TreePath::parse("/obj/window")];

        let action = fill_with_boundaries(
            &mut document,
            &tree,
            &blue,
            Coord::new(3, 3, 1),
            FillMode::Custom,
            &boundaries,
        )
        .unwrap();
        assert_eq!(action.edit.changes.len(), 9);
        document.apply(action.edit);

        assert_eq!(turf_at(&document, Coord::new(3, 3, 1)), &blue);
        assert_eq!(
            turf_at(&document, Coord::new(1, 3, 1)).path,
            TreePath::parse("/turf/closed/wall/reinforced")
        );
        assert_eq!(turf_at(&document, window).path, TreePath::parse("/turf/open/floor"));

        let unresolved = [TreePath::parse("/obj/missing")];
        assert!(
            fill_with_boundaries(
                &mut document,
                &tree,
                &blue,
                Coord::new(3, 3, 1),
                FillMode::Custom,
                &unresolved,
            )
            .is_none()
        );
    }

    #[test]
    fn wall_and_entire_area_modes_can_replace_areas_without_changing_turfs() {
        let tree = tree();
        let station = Prefab::new(TreePath::parse("/area/station"));
        let mut document = grid_document(5, 5, |coord| {
            let boundary = coord.x == 1 || coord.x == 5 || coord.y == 1 || coord.y == 5;
            prefabs(&[
                if boundary {
                    "/turf/closed/wall"
                } else {
                    "/turf/open/floor"
                },
                "/area/space",
            ])
        });
        let center = Coord::new(3, 3, 1);
        let center_turf = turf_at(&document, center).clone();

        let action = fill(&mut document, &tree, &station, center, FillMode::Wall).unwrap();
        assert_eq!(action.edit.changes.len(), 9);
        document.apply(action.edit);

        assert_eq!(turf_at(&document, center), &center_turf);
        for y in 2..=4 {
            for x in 2..=4 {
                assert_eq!(area_at(&document, Coord::new(x, y, 1)), &station);
            }
        }
        assert_eq!(
            area_at(&document, Coord::new(1, 3, 1)).path,
            TreePath::parse("/area/space")
        );

        let mut document = grid_document(4, 1, |coord| {
            let mut area = Prefab::new(TreePath::parse("/area/space"));
            if coord.x == 4 {
                area.set_var("name".into(), Value::Text("separate".into()));
            }

            vec![Prefab::new(TreePath::parse("/turf/open/floor")), area]
        });
        let turfs = (1..=4)
            .map(|x| turf_at(&document, Coord::new(x, 1, 1)).clone())
            .collect::<Vec<_>>();

        let action = fill(
            &mut document,
            &tree,
            &station,
            Coord::new(1, 1, 1),
            FillMode::EntireArea,
        )
        .unwrap();
        assert_eq!(action.edit.changes.len(), 3);
        document.apply(action.edit);

        for (index, turf) in turfs.iter().enumerate() {
            let coord = Coord::new(index as u32 + 1, 1, 1);
            assert_eq!(turf_at(&document, coord), turf);
            assert_eq!(
                area_at(&document, coord).path,
                TreePath::parse(if coord.x == 4 { "/area/space" } else { "/area/station" })
            );
        }
    }

    #[test]
    fn entire_area_uses_the_connected_exact_area_and_floor_fill_preserves_walls() {
        let tree = tree();
        let mut document = grid_document(4, 1, |coord| {
            let turf = match coord.x {
                1 | 4 => "/turf/open/floor",
                2 => "/turf/closed/wall",
                _ => "/turf/open/space",
            };
            let mut area = Prefab::new(TreePath::parse("/area/station"));
            if coord.x == 4 {
                area.set_var("name".into(), Value::Text("Engineering".into()));
            }

            vec![Prefab::new(TreePath::parse(turf)), area]
        });
        let mut blue = Prefab::new(TreePath::parse("/turf/open/floor/blue"));
        blue.set_var("name".into(), Value::Text("blue".into()));

        let action = fill(&mut document, &tree, &blue, Coord::new(1, 1, 1), FillMode::EntireArea).unwrap();
        assert_eq!(action.edit.changes.len(), 2);
        document.apply(action.edit);

        assert_eq!(turf_at(&document, Coord::new(1, 1, 1)), &blue);
        assert_eq!(
            turf_at(&document, Coord::new(2, 1, 1)).path,
            TreePath::parse("/turf/closed/wall")
        );
        assert_eq!(turf_at(&document, Coord::new(3, 1, 1)), &blue);
        assert_eq!(
            turf_at(&document, Coord::new(4, 1, 1)).path,
            TreePath::parse("/turf/open/floor")
        );

        assert!(document.undo());
        let space = Prefab::new(TreePath::parse("/turf/open/space"));
        let action = fill(&mut document, &tree, &space, Coord::new(1, 1, 1), FillMode::EntireArea).unwrap();
        assert_eq!(action.edit.changes.len(), 2);
        document.apply(action.edit);
        for x in 1..=3 {
            assert_eq!(turf_at(&document, Coord::new(x, 1, 1)), &space);
        }
        assert_eq!(
            turf_at(&document, Coord::new(4, 1, 1)).path,
            TreePath::parse("/turf/open/floor")
        );
    }

    #[test]
    fn fill_rejects_non_turfs_and_clips_to_the_active_focus() {
        let tree = tree();
        let mut document = grid_document(3, 1, |_| prefabs(&["/turf/open/space", "/area/space"]));
        let seed = Coord::new(1, 1, 1);
        assert!(
            fill(
                &mut document,
                &tree,
                &Prefab::new(TreePath::parse("/turf/open/floor")),
                Coord::new(0, 1, 1),
                FillMode::Wall,
            )
            .is_none()
        );
        assert!(
            fill(
                &mut document,
                &tree,
                &Prefab::new(TreePath::parse("/obj/table")),
                Coord::new(1, 1, 1),
                FillMode::Wall,
            )
            .is_none()
        );

        let mut no_area = map_document(&["/turf/open/space"]);
        assert!(
            fill(
                &mut no_area,
                &tree,
                &Prefab::new(TreePath::parse("/turf/open/floor")),
                Coord::new(1, 1, 1),
                FillMode::EntireArea,
            )
            .is_none()
        );

        let mut sparse_tree = objtree::ObjectTree::new();
        for path in ["/atom", "/turf/open/space"] {
            sparse_tree.register(&TreePath::parse(path), Location::default());
        }
        let turf = sparse_tree.id_of(&TreePath::parse("/turf")).unwrap();
        sparse_tree.get_mut(turf).unwrap().parent_type = Some(TreePath::parse("/atom"));
        sparse_tree.resolve_parent_types();
        let mut sparse_document = map_document(&["/turf/open/space"]);
        let space = Prefab::new(TreePath::parse("/turf/open/space"));
        assert!(fill(&mut sparse_document, &sparse_tree, &space, seed, FillMode::Wall).is_none());

        let area_id = document.instance_ids_at(seed)[1];
        document.set_focus(Some(AreaFocus::new(
            seed,
            Prefab::new(TreePath::parse("/area/space")),
            area_id,
            HashSet::from([seed, Coord::new(2, 1, 1)]),
        )));
        let floor = Prefab::new(TreePath::parse("/turf/open/floor"));
        let action = fill(&mut document, &tree, &floor, seed, FillMode::Wall).unwrap();
        assert_eq!(action.edit.changes.len(), 2);
        document.apply(action.edit);

        assert_eq!(turf_at(&document, seed), &floor);
        assert_eq!(turf_at(&document, Coord::new(2, 1, 1)), &floor);
        assert_eq!(
            turf_at(&document, Coord::new(3, 1, 1)).path,
            TreePath::parse("/turf/open/space")
        );
    }

    #[test]
    fn fill_limit_counts_only_tiles_that_would_change() {
        let tree = tree();
        let blue = Prefab::new(TreePath::parse("/turf/open/floor/blue"));
        let mut document = grid_document(4, 1, |coord| {
            let turf = match coord.x {
                2 => "/turf/closed/wall",
                3 => "/turf/open/floor/blue",
                _ => "/turf/open/space",
            };

            prefabs(&[turf, "/area/station"])
        });

        assert_eq!(
            fill_with_limit(
                &mut document,
                &tree,
                &blue,
                Coord::new(1, 1, 1),
                FillMode::EntireArea,
                Some(1),
            )
            .err(),
            Some(FillError::TooLarge { limit: 1 })
        );
        let action = fill_with_limit(
            &mut document,
            &tree,
            &blue,
            Coord::new(1, 1, 1),
            FillMode::EntireArea,
            Some(2),
        )
        .unwrap()
        .unwrap();
        assert_eq!(action.edit.changes.len(), 2);
    }

    #[test]
    fn oversized_fill_is_rejected_at_the_default_limit_and_can_be_built_without_it() {
        let tree = tree();
        let width = MAX_FILL_TILES as u32 + 1;
        let mut document = grid_document(width, 1, |_| prefabs(&["/turf/open/space", "/area/space"]));
        let floor = Prefab::new(TreePath::parse("/turf/open/floor"));

        assert_eq!(
            fill_with_limit(
                &mut document,
                &tree,
                &floor,
                Coord::new(1, 1, 1),
                FillMode::Wall,
                Some(MAX_FILL_TILES),
            )
            .err(),
            Some(FillError::TooLarge { limit: MAX_FILL_TILES })
        );
        assert_eq!(
            turf_at(&document, Coord::new(width, 1, 1)).path,
            TreePath::parse("/turf/open/space")
        );
        assert!(!document.undo());

        let action = fill_with_limit(&mut document, &tree, &floor, Coord::new(1, 1, 1), FillMode::Wall, None)
            .unwrap()
            .unwrap();
        assert_eq!(action.edit.changes.len(), MAX_FILL_TILES + 1);
        document.apply(action.edit);
        assert_eq!(turf_at(&document, Coord::new(width, 1, 1)), &floor);
        assert!(document.undo());
        assert_eq!(
            turf_at(&document, Coord::new(width, 1, 1)).path,
            TreePath::parse("/turf/open/space")
        );
        assert!(!document.undo());
    }

    #[test]
    fn block_fill_replaces_turfs_across_exactly_the_target_and_preserves_ids() {
        let tree = tree();
        let mut document = grid_document(3, 1, |_| prefabs(&["/obj/table", "/turf/floor", "/area/station"]));
        let target = Selection::from_drag(Coord::new(1, 1, 1), Coord::new(2, 1, 1));
        let before = (1..=3)
            .map(|x| {
                let coord = Coord::new(x, 1, 1);

                (coord, document.placed_tile(coord).unwrap())
            })
            .collect::<Vec<_>>();
        let turf_ids = (1..=2)
            .map(|x| document.instance_ids_at(Coord::new(x, 1, 1))[1])
            .collect::<Vec<_>>();
        let wall = Prefab::new(TreePath::parse("/turf/wall"));

        let action = fill_selection(&mut document, &tree, target, &wall, BlockSelectionMode::Full).unwrap();
        assert_eq!(action.edit.label, "fill block with /turf/wall");
        assert_eq!(action.edit.changes.len(), 2);
        assert_eq!(action.selected, None);
        document.apply(action.edit);

        for (x, turf_id) in (1..=2).zip(turf_ids) {
            let coord = Coord::new(x, 1, 1);
            assert_eq!(
                document
                    .map
                    .tile_at(coord)
                    .unwrap()
                    .iter()
                    .map(|prefab| prefab.path.to_string())
                    .collect::<Vec<_>>(),
                ["/obj/table", "/turf/wall", "/area/station"]
            );
            assert_eq!(document.instance_ids_at(coord)[1], turf_id);
        }
        assert_eq!(document.placed_tile(Coord::new(3, 1, 1)).unwrap(), before[2].1);

        assert!(document.undo());
        for (coord, tile) in before {
            assert_eq!(document.placed_tile(coord).unwrap(), tile);
        }
        assert!(!document.undo());
    }

    #[test]
    fn hollow_block_fill_mode_uses_the_configured_distance_from_each_edge() {
        let selection = Selection::from_drag(Coord::new(1, 1, 1), Coord::new(6, 5, 1));
        let included = |line_width| {
            selection
                .iter()
                .filter(|coord| BlockSelectionMode::Hollow { line_width }.includes(selection, *coord))
                .collect::<HashSet<_>>()
        };

        let one_tile = included(1);
        assert_eq!(one_tile.len(), 18);
        assert!(one_tile.contains(&Coord::new(1, 1, 1)));
        assert!(one_tile.contains(&Coord::new(6, 5, 1)));
        assert!(!one_tile.contains(&Coord::new(2, 2, 1)));

        let two_tiles = included(2);
        assert_eq!(two_tiles.len(), 28);
        assert!(!two_tiles.contains(&Coord::new(3, 3, 1)));
        assert!(!two_tiles.contains(&Coord::new(4, 3, 1)));

        assert_eq!(included(3).len(), 30);
        assert_eq!(included(0), one_tile);

        let one_row = Selection::from_drag(Coord::new(2, 3, 1), Coord::new(5, 3, 1));
        assert!(
            one_row
                .iter()
                .all(|coord| BlockSelectionMode::Hollow { line_width: 1 }.includes(one_row, coord))
        );
    }

    #[test]
    fn hollow_block_fill_changes_only_the_border_and_undoes_as_one_edit() {
        let tree = tree();
        let mut document = grid_document(5, 5, |_| prefabs(&["/turf/floor", "/area/station"]));
        let selection = Selection::from_drag(Coord::new(1, 1, 1), Coord::new(5, 5, 1));
        let before = selection
            .iter()
            .map(|coord| (coord, document.placed_tile(coord).unwrap()))
            .collect::<Vec<_>>();
        let turf_ids = selection
            .iter()
            .map(|coord| (coord, document.instance_ids_at(coord)[0]))
            .collect::<HashMap<_, _>>();
        let wall = Prefab::new(TreePath::parse("/turf/wall"));

        let action = fill_selection(
            &mut document,
            &tree,
            selection,
            &wall,
            BlockSelectionMode::Hollow { line_width: 1 },
        )
        .unwrap();
        assert_eq!(action.edit.changes.len(), 16);
        document.apply(action.edit);

        for coord in selection.iter() {
            let expected = if coord.x == 1 || coord.x == 5 || coord.y == 1 || coord.y == 5 {
                "/turf/wall"
            } else {
                "/turf/floor"
            };
            assert_eq!(turf_at(&document, coord).path, TreePath::parse(expected));
            assert_eq!(document.instance_ids_at(coord)[0], turf_ids[&coord]);
        }

        assert!(document.undo());
        for (coord, tile) in before {
            assert_eq!(document.placed_tile(coord).unwrap(), tile);
        }
        assert!(!document.undo());
    }

    #[test]
    fn hollow_block_mode_saturates_instead_of_overflowing_on_thin_or_wide_lines() {
        let single = Selection::from_drag(Coord::new(4, 4, 1), Coord::new(4, 4, 1));
        assert!(BlockSelectionMode::Hollow { line_width: 1 }.includes(single, Coord::new(4, 4, 1)));
        assert!(BlockSelectionMode::Hollow { line_width: u32::MAX }.includes(single, Coord::new(4, 4, 1)));

        let block = Selection::from_drag(Coord::new(1, 1, 1), Coord::new(4, 4, 1));
        for line_width in [2, 3, u32::MAX] {
            assert!(
                block
                    .iter()
                    .all(|coord| BlockSelectionMode::Hollow { line_width }.includes(block, coord)),
                "a {line_width}-tile line should leave no hole in a 4x4 block"
            );
        }

        for mode in [BlockSelectionMode::Full, BlockSelectionMode::Hollow { line_width: 1 }] {
            assert!(!mode.includes(block, Coord::new(5, 1, 1)));
            assert!(!mode.includes(block, Coord::new(1, 5, 1)));
            assert!(!mode.includes(block, Coord::new(1, 1, 2)));
        }
    }

    #[test]
    fn hollow_block_edits_ignore_tiles_blocked_inside_the_hole() {
        let tree = tree();
        let mut document = grid_document(5, 5, |_| prefabs(&["/turf/floor", "/area/station"]));
        let selection = Selection::from_drag(Coord::new(1, 1, 1), Coord::new(5, 5, 1));
        let border = BlockSelectionMode::Hollow { line_width: 1 };
        let seed = Coord::new(1, 1, 1);
        let area = document.instance_ids_at(seed)[1];
        document.set_focus(Some(AreaFocus::new(
            seed,
            Prefab::new(TreePath::parse("/area/station")),
            area,
            selection
                .iter()
                .filter(|coord| border.includes(selection, *coord))
                .collect(),
        )));
        let wall = Prefab::new(TreePath::parse("/turf/wall"));

        // the focus only covers the ring, so a full-rectangle fill has to be rejected outright
        assert!(fill_selection(&mut document, &tree, selection, &wall, BlockSelectionMode::Full).is_none());
        // ...and so does a thicker ring, which reaches one tile further into the blocked interior
        assert!(
            fill_selection(
                &mut document,
                &tree,
                selection,
                &wall,
                BlockSelectionMode::Hollow { line_width: 2 }
            )
            .is_none()
        );

        let action = fill_selection(&mut document, &tree, selection, &wall, border).unwrap();
        assert_eq!(action.edit.changes.len(), 16);
        assert!(document.apply(action.edit));
        assert_eq!(
            turf_at(&document, Coord::new(3, 1, 1)).path,
            TreePath::parse("/turf/wall")
        );
        assert_eq!(
            turf_at(&document, Coord::new(3, 3, 1)).path,
            TreePath::parse("/turf/floor")
        );
    }

    #[test]
    fn hollow_block_moves_relocate_the_ring_and_leave_both_interiors_alone() {
        let tree = tree();
        let mut document = grid_document(10, 4, |coord| {
            prefabs(if coord.x <= 5 {
                &["/obj/table", "/turf/floor", "/area/station"]
            } else {
                &["/obj/chair", "/turf/wall", "/area/space"]
            })
        });
        let source = Selection::from_drag(Coord::new(1, 1, 1), Coord::new(5, 4, 1));
        let target = Selection::from_drag(Coord::new(6, 1, 1), Coord::new(10, 4, 1));
        let border = BlockSelectionMode::Hollow { line_width: 1 };
        let before = source
            .iter()
            .chain(target.iter())
            .map(|coord| (coord, document.placed_tile(coord).unwrap()))
            .collect::<Vec<_>>();
        let moved_table = document.instance_ids_at(Coord::new(1, 1, 1))[0];
        let interior_table = document.instance_ids_at(Coord::new(2, 2, 1))[0];

        let (action, selected) = place_selection_with_mode(
            &mut document,
            &tree,
            source,
            target.min,
            SelectionRotation::Original,
            SelectionPlacement::Move,
            border,
        )
        .unwrap();
        assert_eq!(selected, target);
        // fourteen ring tiles cleared at the source, fourteen overwritten at the destination
        assert_eq!(action.edit.changes.len(), 28);
        document.apply(action.edit);

        for coord in source.iter() {
            let expected: &[&str] = if border.includes(source, coord) {
                &["/turf", "/area"]
            } else {
                &["/obj/table", "/turf/floor", "/area/station"]
            };
            assert_eq!(tile_paths(&document, coord), expected, "source tile {coord:?}");
        }
        for coord in target.iter() {
            let expected: &[&str] = if border.includes(target, coord) {
                &["/obj/table", "/turf/floor", "/area/station"]
            } else {
                &["/obj/chair", "/turf/wall", "/area/space"]
            };
            assert_eq!(tile_paths(&document, coord), expected, "target tile {coord:?}");
        }
        assert_eq!(
            document.instance_location(moved_table).unwrap().coord,
            Coord::new(6, 1, 1)
        );
        assert_eq!(
            document.instance_location(interior_table).unwrap().coord,
            Coord::new(2, 2, 1)
        );

        assert!(document.undo());
        for (coord, tile) in before {
            assert_eq!(document.placed_tile(coord).unwrap(), tile);
        }
        assert!(!document.undo());
    }

    #[test]
    fn overlapping_hollow_block_moves_clear_ring_tiles_the_new_ring_does_not_cover() {
        let tree = tree();
        let mut document = grid_document(6, 4, |_| prefabs(&["/obj/table", "/turf/floor", "/area/station"]));
        let source = Selection::from_drag(Coord::new(1, 1, 1), Coord::new(5, 4, 1));
        let target = Selection::from_drag(Coord::new(2, 1, 1), Coord::new(6, 4, 1));
        let border = BlockSelectionMode::Hollow { line_width: 1 };
        let tables = Selection::from_drag(Coord::new(1, 1, 1), Coord::new(6, 4, 1))
            .iter()
            .map(|coord| (coord, document.instance_ids_at(coord)[0]))
            .collect::<HashMap<_, _>>();

        let (action, selected) = place_selection_with_mode(
            &mut document,
            &tree,
            source,
            target.min,
            SelectionRotation::Original,
            SelectionPlacement::Move,
            border,
        )
        .unwrap();
        assert_eq!(selected, target);
        // the two rings overlap on eight tiles, so twenty tiles change in total
        assert_eq!(action.edit.changes.len(), 20);
        document.apply(action.edit);

        // tiles inside both holes are never staged, so their original instances survive untouched
        for coord in [
            Coord::new(3, 2, 1),
            Coord::new(4, 2, 1),
            Coord::new(3, 3, 1),
            Coord::new(4, 3, 1),
        ] {
            assert_eq!(
                tile_paths(&document, coord),
                ["/obj/table", "/turf/floor", "/area/station"]
            );
            assert_eq!(document.instance_ids_at(coord)[0], tables[&coord]);
        }

        // ring tiles the shifted ring no longer covers fall back to the world defaults
        for coord in [
            Coord::new(1, 1, 1),
            Coord::new(1, 2, 1),
            Coord::new(1, 3, 1),
            Coord::new(1, 4, 1),
            Coord::new(5, 2, 1),
            Coord::new(5, 3, 1),
        ] {
            assert_eq!(
                tile_paths(&document, coord),
                ["/turf", "/area"],
                "cleared tile {coord:?}"
            );
        }

        // everything on the new ring came from the tile one step to its left
        for coord in target.iter().filter(|coord| border.includes(target, *coord)) {
            let origin = Coord::new(coord.x - 1, coord.y, coord.z);
            assert_eq!(
                document.instance_ids_at(coord)[0],
                tables[&origin],
                "ring tile {coord:?}"
            );
        }
    }

    #[test]
    fn rotated_hollow_block_moves_land_the_ring_on_the_rotated_ring() {
        let tree = tree();
        let mut document = grid_document(9, 5, |coord| {
            prefabs(if coord.x <= 5 {
                &["/obj/table", "/turf/floor", "/area/station"]
            } else {
                &["/obj/chair", "/turf/wall", "/area/space"]
            })
        });
        let source = Selection::from_drag(Coord::new(1, 1, 1), Coord::new(5, 4, 1));
        let border = BlockSelectionMode::Hollow { line_width: 1 };

        let (action, target) = place_selection_with_mode(
            &mut document,
            &tree,
            source,
            Coord::new(6, 1, 1),
            SelectionRotation::Clockwise,
            SelectionPlacement::Move,
            border,
        )
        .unwrap();
        // the 5x4 source rotates into a 4x5 destination
        assert_eq!(target, Selection::from_drag(Coord::new(6, 1, 1), Coord::new(9, 5, 1)));
        assert_eq!(action.edit.changes.len(), 28);
        document.apply(action.edit);

        // the rotated ring covers exactly the destination ring, so only its hole keeps the old tiles
        for coord in target.iter() {
            let expected: &[&str] = if border.includes(target, coord) {
                &["/obj/table", "/turf/floor", "/area/station"]
            } else {
                &["/obj/chair", "/turf/wall", "/area/space"]
            };
            assert_eq!(tile_paths(&document, coord), expected, "target tile {coord:?}");
        }
        for coord in source.iter().filter(|coord| !border.includes(source, *coord)) {
            assert_eq!(
                tile_paths(&document, coord),
                ["/obj/table", "/turf/floor", "/area/station"]
            );
        }
    }

    #[test]
    fn mirrored_hollow_block_transforms_only_swap_the_ring() {
        let tree = tree();
        let mut document = grid_document(5, 4, |coord| {
            prefabs(match coord.x {
                1 => &["/turf/wall", "/area/station"],
                5 => &["/turf/open/space", "/area/station"],
                _ => &["/turf/floor", "/area/station"],
            })
        });
        let selection = Selection::from_drag(Coord::new(1, 1, 1), Coord::new(5, 4, 1));
        let border = BlockSelectionMode::Hollow { line_width: 1 };
        let before = selection
            .iter()
            .map(|coord| (coord, document.placed_tile(coord).unwrap()))
            .collect::<Vec<_>>();

        let (action, mirrored) = transform_selection_with_mode(
            &mut document,
            &tree,
            selection,
            SelectionTransform::MirrorHorizontal,
            border,
        )
        .unwrap();
        assert_eq!(mirrored, selection);
        // the centre column mirrors onto itself, leaving twelve of the fourteen ring tiles changed
        assert_eq!(action.edit.changes.len(), 12);
        document.apply(action.edit);

        for y in [1, 2, 3, 4] {
            assert_eq!(
                turf_at(&document, Coord::new(1, y, 1)).path,
                TreePath::parse("/turf/open/space")
            );
            assert_eq!(
                turf_at(&document, Coord::new(5, y, 1)).path,
                TreePath::parse("/turf/wall")
            );
        }
        // the hole never took part, so no tile there was reset to the world default either
        for coord in selection.iter().filter(|coord| !border.includes(selection, *coord)) {
            assert_eq!(turf_at(&document, coord).path, TreePath::parse("/turf/floor"));
            assert_eq!(area_at(&document, coord).path, TreePath::parse("/area/station"));
        }

        assert!(document.undo());
        for (coord, tile) in before {
            assert_eq!(document.placed_tile(coord).unwrap(), tile);
        }
        assert!(!document.undo());
    }

    #[test]
    fn block_fill_appends_distinct_objects_and_undoes_as_one_edit() {
        let tree = tree();
        let mut document = grid_document(2, 1, |_| prefabs(&["/turf/floor", "/area/station"]));
        let target = Selection::from_drag(Coord::new(1, 1, 1), Coord::new(2, 1, 1));
        let before = (1..=2)
            .map(|x| document.placed_tile(Coord::new(x, 1, 1)).unwrap())
            .collect::<Vec<_>>();
        let chair = Prefab::new(TreePath::parse("/obj/chair"));

        let action = fill_selection(&mut document, &tree, target, &chair, BlockSelectionMode::Full).unwrap();
        assert_eq!(action.edit.changes.len(), 2);
        assert_eq!(action.affected.len(), 2);
        assert_ne!(action.affected[0], action.affected[1]);
        document.apply(action.edit);

        for x in 1..=2 {
            assert_eq!(
                document
                    .map
                    .tile_at(Coord::new(x, 1, 1))
                    .unwrap()
                    .iter()
                    .map(|prefab| prefab.path.to_string())
                    .collect::<Vec<_>>(),
                ["/obj/chair", "/turf/floor", "/area/station"]
            );
        }

        assert!(document.undo());
        for (index, tile) in before.into_iter().enumerate() {
            assert_eq!(document.placed_tile(Coord::new(index as u32 + 1, 1, 1)).unwrap(), tile);
        }
        assert!(!document.undo());
    }

    #[test]
    fn block_fill_rejects_invalid_targets_atomically() {
        let tree = tree();
        let mut document = grid_document(2, 1, |_| prefabs(&["/turf/floor", "/area/station"]));
        let source = Coord::new(1, 1, 1);
        let area = document.instance_ids_at(source)[1];
        let before = (1..=2)
            .map(|x| document.placed_tile(Coord::new(x, 1, 1)).unwrap())
            .collect::<Vec<_>>();
        let chair = Prefab::new(TreePath::parse("/obj/chair"));

        assert!(
            fill_selection(
                &mut document,
                &tree,
                Selection::from_drag(source, Coord::new(3, 1, 1)),
                &chair,
                BlockSelectionMode::Full,
            )
            .is_none()
        );

        document.set_focus(Some(AreaFocus::new(
            source,
            Prefab::new(TreePath::parse("/area/station")),
            area,
            HashSet::from([source]),
        )));
        assert!(
            fill_selection(
                &mut document,
                &tree,
                Selection::from_drag(source, Coord::new(2, 1, 1)),
                &chair,
                BlockSelectionMode::Full,
            )
            .is_none()
        );

        assert_eq!(
            (1..=2)
                .map(|x| document.placed_tile(Coord::new(x, 1, 1)).unwrap())
                .collect::<Vec<_>>(),
            before
        );
        assert!(!document.undo());
    }

    #[test]
    fn block_moves_are_overlap_safe_and_replace_every_destination_tile() {
        let tree = tree();
        let mut document = grid_document(4, 1, |coord| {
            prefabs(match coord.x {
                1 => &["/obj/table", "/turf/floor", "/area/station"],
                2 => &["/obj/chair", "/turf/wall", "/area/space"],
                3 => &["/obj/window", "/turf/open/space", "/area/station"],
                _ => &["/turf/floor", "/area/station"],
            })
        });
        let source = Selection::from_drag(Coord::new(1, 1, 1), Coord::new(2, 1, 1));
        let before = (1..=3)
            .map(|x| {
                let coord = Coord::new(x, 1, 1);

                (coord, document.placed_tile(coord).unwrap())
            })
            .collect::<Vec<_>>();
        let table = document.instance_ids_at(Coord::new(1, 1, 1))[0];
        let chair = document.instance_ids_at(Coord::new(2, 1, 1))[0];
        let window = document.instance_ids_at(Coord::new(3, 1, 1))[0];

        let (action, moved) = move_selection(&mut document, &tree, source, Coord::new(2, 1, 1)).unwrap();
        assert_eq!(moved, Selection::from_drag(Coord::new(2, 1, 1), Coord::new(3, 1, 1)));
        assert_eq!(action.edit.changes.len(), 3);
        document.apply(action.edit);

        assert_eq!(
            document
                .map
                .tile_at(Coord::new(1, 1, 1))
                .unwrap()
                .iter()
                .map(|prefab| prefab.path.to_string())
                .collect::<Vec<_>>(),
            ["/turf", "/area"]
        );
        assert_eq!(document.instance_location(table).unwrap().coord, Coord::new(2, 1, 1));
        assert_eq!(document.instance_location(chair).unwrap().coord, Coord::new(3, 1, 1));
        assert_eq!(document.instance_location(window), None);
        assert_eq!(
            document
                .map
                .tile_at(Coord::new(3, 1, 1))
                .unwrap()
                .iter()
                .map(|prefab| prefab.path.to_string())
                .collect::<Vec<_>>(),
            ["/obj/chair", "/turf/wall", "/area/space"]
        );

        assert!(document.undo());
        for (coord, tile) in before {
            assert_eq!(document.placed_tile(coord).unwrap(), tile);
        }
        assert!(!document.undo());
        assert!(document.redo());
        assert_eq!(document.instance_location(table).unwrap().coord, Coord::new(2, 1, 1));
    }

    #[test]
    fn block_moves_restore_configured_world_turf_and_area() {
        let mut tree = tree();
        let world = tree.register(&TreePath::parse("/world"), Location::default());
        for (name, path) in [("turf", "/turf/open/space"), ("area", "/area/space")] {
            tree.get_mut(world).unwrap().vars.insert(
                name.into(),
                VarDecl {
                    name: name.into(),
                    declared_type: None,
                    modifiers: VarModifiers::default(),
                    value: Value::Path(TreePath::parse(path)),
                    initializer: None,
                    location: Location::default(),
                },
            );
        }
        assert_eq!(
            default_tile_paths(&tree),
            Some((TreePath::parse("/turf/open/space"), TreePath::parse("/area/space")))
        );
        let mut document = grid_document(2, 1, |coord| {
            if coord.x == 1 {
                prefabs(&["/obj/table", "/turf/floor", "/area/station"])
            } else {
                prefabs(&["/turf/floor", "/area/station"])
            }
        });
        let source = Selection::from_drag(Coord::new(1, 1, 1), Coord::new(1, 1, 1));

        let (action, _) = move_selection(&mut document, &tree, source, Coord::new(2, 1, 1)).unwrap();
        document.apply(action.edit);

        assert_eq!(
            document
                .map
                .tile_at(Coord::new(1, 1, 1))
                .unwrap()
                .iter()
                .map(|prefab| prefab.path.to_string())
                .collect::<Vec<_>>(),
            ["/turf/open/space", "/area/space"]
        );
    }

    #[test]
    fn block_copies_keep_the_source_and_replace_the_destination_with_new_ids() {
        let tree = tree();
        let mut document = grid_document(2, 1, |coord| {
            prefabs(if coord.x == 1 {
                &["/obj/table", "/turf/floor", "/area/station"]
            } else {
                &["/obj/chair", "/turf/wall", "/area/space"]
            })
        });
        let source_coord = Coord::new(1, 1, 1);
        let destination_coord = Coord::new(2, 1, 1);
        let selection = Selection::from_drag(source_coord, source_coord);
        let source_before = document.placed_tile(source_coord).unwrap();
        let destination_before = document.placed_tile(destination_coord).unwrap();
        let source_table = source_before[0].id();
        let destination_chair = destination_before[0].id();

        let (action, copied) = copy_selection(&mut document, &tree, selection, destination_coord).unwrap();
        assert_eq!(copied, Selection::from_drag(destination_coord, destination_coord));
        document.apply(action.edit);

        assert_eq!(document.placed_tile(source_coord).unwrap(), source_before);
        assert_eq!(document.instance_location(source_table).unwrap().coord, source_coord);
        assert_eq!(document.instance_location(destination_chair), None);
        let destination = document.placed_tile(destination_coord).unwrap();
        assert_eq!(
            destination
                .iter()
                .map(|placed| placed.prefab().path.to_string())
                .collect::<Vec<_>>(),
            ["/obj/table", "/turf/floor", "/area/station"]
        );
        let copied_table = destination
            .iter()
            .find(|placed| placed.prefab().path == TreePath::parse("/obj/table"))
            .unwrap();
        assert_ne!(copied_table.id(), source_table);

        assert!(document.undo());
        assert_eq!(document.placed_tile(source_coord).unwrap(), source_before);
        assert_eq!(document.placed_tile(destination_coord).unwrap(), destination_before);
        assert!(!document.undo());
    }

    #[test]
    fn rotated_block_moves_relocate_and_rotate_every_object_in_one_edit() {
        let tree = tree();
        let mut document = grid_document(6, 4, |coord| {
            let mut tile = Vec::new();
            if coord.x <= 2 && coord.y <= 3 {
                let mut object = Prefab::new(TreePath::parse("/obj/table"));
                object.set_var("dir".into(), Value::Num(Dir::North.to_bits() as f32));
                tile.push(object);
            }
            tile.extend(prefabs(&["/turf/floor", "/area/station"]));

            tile
        });
        let source = Selection::from_drag(Coord::new(1, 1, 1), Coord::new(2, 3, 1));
        let target_min = Coord::new(4, 1, 1);
        let before = (1..=4)
            .flat_map(|y| (1..=6).map(move |x| Coord::new(x, y, 1)))
            .map(|coord| (coord, document.placed_tile(coord).unwrap()))
            .collect::<Vec<_>>();
        let objects = source
            .iter()
            .map(|coord| (coord, document.instance_ids_at(coord)[0]))
            .collect::<Vec<_>>();

        let (action, target) = place_selection(
            &mut document,
            &tree,
            source,
            target_min,
            SelectionRotation::Clockwise,
            SelectionPlacement::Move,
        )
        .unwrap();
        assert_eq!(target, Selection::from_drag(Coord::new(4, 1, 1), Coord::new(6, 2, 1)));
        document.apply(action.edit);

        for (coord, object) in objects {
            let relative = (coord.x - source.min.x, coord.y - source.min.y);
            let (x, y) = rotate_point(relative, source.width(), source.height(), SelectionRotation::Clockwise);
            let (prefab, location) = document.prefab_instance(object).unwrap();
            assert_eq!(location.coord, Coord::new(target.min.x + x, target.min.y + y, 1));
            assert_eq!(prefab.var(&"dir".into()), Some(&Value::Num(Dir::East.to_bits() as f32)));
        }
        assert!(source.iter().all(|coord| {
            document
                .map
                .tile_at(coord)
                .is_some_and(|tile| tile.iter().all(|prefab| !prefab.path.to_string().starts_with("/obj")))
        }));

        assert!(document.undo());
        for (coord, tile) in before {
            assert_eq!(document.placed_tile(coord).unwrap(), tile);
        }
        assert!(!document.undo());
    }

    #[test]
    fn block_moves_are_rejected_atomically_outside_the_active_focus() {
        let tree = tree();
        let mut document = grid_document(2, 1, |_| prefabs(&["/turf/floor", "/area/station"]));
        let source = Coord::new(1, 1, 1);
        let area = document.instance_ids_at(source)[1];
        document.set_focus(Some(AreaFocus::new(
            source,
            Prefab::new(TreePath::parse("/area/station")),
            area,
            HashSet::from([source]),
        )));
        let before = (1..=2)
            .map(|x| document.placed_tile(Coord::new(x, 1, 1)).unwrap())
            .collect::<Vec<_>>();
        let selection = Selection::from_drag(source, source);

        assert!(move_selection(&mut document, &tree, selection, Coord::new(2, 1, 1)).is_none());
        assert_eq!(
            (1..=2)
                .map(|x| document.placed_tile(Coord::new(x, 1, 1)).unwrap())
                .collect::<Vec<_>>(),
            before
        );
        assert!(!document.undo());
    }

    #[test]
    fn block_rotation_swaps_dimensions_positions_and_atom_appearance() {
        let tree = tree();
        let mut document = grid_document(4, 4, |coord| {
            let mut object = Prefab::new(TreePath::parse(if coord == Coord::new(1, 1, 1) {
                "/obj/table"
            } else {
                "/obj/chair"
            }));
            if coord == Coord::new(1, 1, 1) {
                object.set_var("dir".into(), Value::Num(Dir::North.to_bits() as f32));
                object.set_var("pixel_x".into(), Value::Num(2.0));
                object.set_var("pixel_y".into(), Value::Num(3.0));
                object.set_var("pixel_w".into(), Value::Num(4.0));
                object.set_var("pixel_z".into(), Value::Num(5.0));
                object.set_var("step_x".into(), Value::Num(6.0));
                object.set_var("step_y".into(), Value::Num(7.0));
            } else if coord == Coord::new(2, 1, 1) {
                object.set_var("dir".into(), Value::Num(3.0));
            } else if coord == Coord::new(1, 2, 1) {
                object.set_var("dir".into(), Value::Text(String::from("sideways")));
            }

            vec![
                object,
                Prefab::new(TreePath::parse("/turf/floor")),
                Prefab::new(TreePath::parse("/area/station")),
            ]
        });
        let source = Selection::from_drag(Coord::new(1, 1, 1), Coord::new(2, 3, 1));
        let objects = source
            .iter()
            .map(|coord| (coord, document.instance_ids_at(coord)[0]))
            .collect::<Vec<_>>();
        let invalid_numeric = document.instance_ids_at(Coord::new(2, 1, 1))[0];
        let invalid_text = document.instance_ids_at(Coord::new(1, 2, 1))[0];

        let (action, rotated) =
            transform_selection(&mut document, &tree, source, SelectionTransform::RotateClockwise).unwrap();
        assert_eq!(rotated, Selection::from_drag(Coord::new(1, 1, 1), Coord::new(3, 2, 1)));
        document.apply(action.edit);

        for (coord, object) in objects {
            let relative = (coord.x - source.min.x, coord.y - source.min.y);
            let (x, y) = transform_point(
                relative,
                source.width(),
                source.height(),
                SelectionTransform::RotateClockwise,
            );
            assert_eq!(
                document.instance_location(object).unwrap().coord,
                Coord::new(rotated.min.x + x, rotated.min.y + y, 1)
            );
        }
        let table = document
            .map
            .tile_at(Coord::new(1, 2, 1))
            .unwrap()
            .iter()
            .find(|prefab| prefab.path == TreePath::parse("/obj/table"))
            .unwrap();
        for (name, value) in [
            ("dir", Dir::East.to_bits() as f32),
            ("pixel_x", 3.0),
            ("pixel_y", -2.0),
            ("pixel_w", 5.0),
            ("pixel_z", -4.0),
            ("step_x", 7.0),
            ("step_y", -6.0),
        ] {
            assert_eq!(table.var(&name.into()), Some(&Value::Num(value)), "{name}");
        }
        assert_eq!(
            document.prefab_instance(invalid_numeric).unwrap().0.var(&"dir".into()),
            Some(&Value::Num(3.0))
        );
        assert_eq!(
            document.prefab_instance(invalid_text).unwrap().0.var(&"dir".into()),
            Some(&Value::Text(String::from("sideways")))
        );
    }

    #[test]
    fn block_rotation_preserves_spacing_between_offset_directional_objects() {
        let mut tree = tree();
        for (path, variables) in [
            ("/obj/sign/directional/north", [("dir", 1.0), ("pixel_y", 32.0)]),
            ("/obj/sign/directional/south", [("dir", 2.0), ("pixel_y", -32.0)]),
            ("/obj/sign/directional/east", [("dir", 4.0), ("pixel_x", 32.0)]),
            ("/obj/sign/directional/west", [("dir", 8.0), ("pixel_x", -32.0)]),
        ] {
            let id = tree.id_of(&TreePath::parse(path)).unwrap();
            for (name, value) in variables {
                tree.get_mut(id).unwrap().vars.insert(
                    name.into(),
                    VarDecl {
                        name: name.into(),
                        declared_type: None,
                        modifiers: VarModifiers::default(),
                        value: Value::Num(value),
                        initializer: None,
                        location: Location::default(),
                    },
                );
            }
        }
        let mut document = grid_document(1, 1, |_| {
            let mut upper = Prefab::new(TreePath::parse("/obj/sign"));
            upper.set_var("pixel_y".into(), Value::Num(5.0));
            upper.set_var("step_y".into(), Value::Num(3.0));
            let center = Prefab::new(TreePath::parse("/obj/sign"));
            let mut lower = Prefab::new(TreePath::parse("/obj/sign"));
            lower.set_var("pixel_y".into(), Value::Num(-5.0));
            lower.set_var("step_y".into(), Value::Num(-3.0));

            vec![
                upper,
                center,
                lower,
                Prefab::new(TreePath::parse("/turf")),
                Prefab::new(TreePath::parse("/area")),
            ]
        });
        let coord = Coord::new(1, 1, 1);
        let selection = Selection::from_drag(coord, coord);

        let (action, _) = place_selection(
            &mut document,
            &tree,
            selection,
            coord,
            SelectionRotation::Clockwise,
            SelectionPlacement::Move,
        )
        .unwrap();
        document.apply(action.edit);

        let appearances = document
            .map
            .tile_at(coord)
            .unwrap()
            .iter()
            .take(3)
            .map(|prefab| {
                assert_eq!(prefab.path, TreePath::parse("/obj/sign/directional/west"));
                let id = tree.id_of(&prefab.path).unwrap();

                crate::visual::resolve_id(&tree, id, prefab)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            appearances
                .iter()
                .map(|appearance| (
                    appearance.pixel_x + appearance.step_x,
                    appearance.pixel_y + appearance.step_y
                ))
                .collect::<Vec<_>>(),
            [(8, 0), (0, 0), (-8, 0)]
        );
        assert!(
            appearances
                .iter()
                .all(|appearance| appearance.dir == Dir::West.to_bits())
        );
    }

    #[test]
    fn block_transforms_use_the_lower_left_anchor_and_planar_directions() {
        let selection = Selection::from_drag(Coord::new(4, 7, 2), Coord::new(5, 9, 2));
        assert_eq!(
            transformed_selection(selection, SelectionTransform::RotateCounterClockwise),
            Some(Selection::from_drag(Coord::new(4, 7, 2), Coord::new(6, 8, 2)))
        );
        assert_eq!(
            transform_point((0, 0), 2, 3, SelectionTransform::RotateClockwise),
            (0, 1)
        );
        assert_eq!(
            transform_point((0, 2), 2, 3, SelectionTransform::RotateCounterClockwise),
            (0, 0)
        );
        assert_eq!(
            transform_point((0, 1), 2, 3, SelectionTransform::MirrorHorizontal),
            (1, 1)
        );
        assert_eq!(
            transform_point((0, 0), 2, 3, SelectionTransform::MirrorVertical),
            (0, 2)
        );
        assert_eq!(
            transform_direction(Dir::Northwest, SelectionTransform::RotateClockwise),
            Dir::Northeast
        );
        assert_eq!(
            transform_direction(Dir::Southeast, SelectionTransform::MirrorHorizontal),
            Dir::Southwest
        );
        assert_eq!(
            transform_direction(Dir::Northeast, SelectionTransform::MirrorVertical),
            Dir::Southeast
        );
    }

    #[test]
    fn block_rotation_prefers_directional_sibling_paths_and_preserves_ids() {
        let mut tree = tree();
        tree.register(&TreePath::parse("/obj/alarm/directional/north"), Location::default());
        tree.register(&TreePath::parse("/obj/alarm/directional/east"), Location::default());
        tree.resolve_parent_types();
        let mut document = map_document(&["/obj/alarm/directional/north", "/turf/floor", "/area/station"]);
        let coord = Coord::new(1, 1, 1);
        let object = document.instance_ids_at(coord)[0];
        let selection = Selection::from_drag(coord, coord);

        let (action, _) =
            transform_selection(&mut document, &tree, selection, SelectionTransform::RotateClockwise).unwrap();
        document.apply(action.edit);

        let (prefab, location) = document.prefab_instance(object).unwrap();
        assert_eq!(location.coord, coord);
        assert_eq!(prefab.path, TreePath::parse("/obj/alarm/directional/east"));
        assert_eq!(prefab.var(&"dir".into()), None);
    }
}
