use base::{
    path::TreePath,
    types::{Identifier, Value},
    vars,
};

use dmi::metadata::Dir;
use dmm::{Coord, Prefab};
use objtree::{ObjectTree, TypeId};

use crate::{document::Selection, frame::visual};

use super::{SelectionRotation, SelectionTransform};

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

pub(super) fn transform_point(
    (x, y): (u32, u32), width: u32, height: u32, transform: SelectionTransform,
) -> (u32, u32) {
    match transform {
        SelectionTransform::RotateClockwise => (y, width - 1 - x),
        SelectionTransform::RotateCounterClockwise => (height - 1 - y, x),
        SelectionTransform::MirrorHorizontal => (width - 1 - x, y),
        SelectionTransform::MirrorVertical => (x, height - 1 - y),
    }
}

pub(super) fn transform_prefab(tree: &ObjectTree, prefab: &mut Prefab, transform: SelectionTransform) {
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

pub(super) fn transform_direction(direction: Dir, transform: SelectionTransform) -> Dir {
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
