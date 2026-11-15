use dmi::metadata::Dir;
use ui::icons::materialdesignicons::{
    ICON_ARROW_BOTTOM_LEFT, ICON_ARROW_BOTTOM_RIGHT, ICON_ARROW_DOWN, ICON_ARROW_LEFT, ICON_ARROW_RIGHT,
    ICON_ARROW_TOP_LEFT, ICON_ARROW_TOP_RIGHT, ICON_ARROW_UP,
};

use crate::gizmo::direction::DirectionalTypes;

pub(super) fn direction_label(direction: Dir) -> String {
    format!("{} {}", direction_icon(direction), direction_name(direction))
}

fn direction_icon(direction: Dir) -> char {
    match direction {
        Dir::South => ICON_ARROW_DOWN,
        Dir::North => ICON_ARROW_UP,
        Dir::East => ICON_ARROW_RIGHT,
        Dir::West => ICON_ARROW_LEFT,
        Dir::Southeast => ICON_ARROW_BOTTOM_RIGHT,
        Dir::Southwest => ICON_ARROW_BOTTOM_LEFT,
        Dir::Northeast => ICON_ARROW_TOP_RIGHT,
        Dir::Northwest => ICON_ARROW_TOP_LEFT,
    }
}

fn direction_name(direction: Dir) -> &'static str {
    match direction {
        Dir::South => "South",
        Dir::North => "North",
        Dir::East => "East",
        Dir::West => "West",
        Dir::Southeast => "Southeast",
        Dir::Southwest => "Southwest",
        Dir::Northeast => "Northeast",
        Dir::Northwest => "Northwest",
    }
}

pub(super) fn direction_choices(
    count: usize, declared: Option<[bool; 8]>, directional_types: Option<DirectionalTypes>,
) -> Vec<Dir> {
    if let Some(supported) = directional_types.map(|types| types.supported).or(declared) {
        return Dir::ORDER
            .into_iter()
            .zip(supported)
            .filter_map(|(direction, supported)| supported.then_some(direction))
            .collect();
    }

    Dir::ORDER.into_iter().take(count).collect()
}

pub(super) fn selected_direction(bits: u32, directional_types: Option<DirectionalTypes>) -> Option<Dir> {
    directional_types
        .and_then(|types| types.current)
        .or_else(|| Dir::from_bits(bits))
}
