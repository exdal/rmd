use dear_imgui_rs::{Key, Ui};
use dmm::{Coord, Size};
use editor::{document::Selection, tool::SelectionRotation};

use super::{OverlayRect, PlacementControls, block_placement_controls_layout, draw_overlay_underlay};
use crate::camera::Controller;

pub(super) const PASTE_LABELS: [&str; 2] = ["Paste", "Cancel"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PendingPaste {
    /// lower left corner of the footprint, in destination-map tiles
    pub(super) min: Coord,
    pub(super) rotation: SelectionRotation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PasteAction {
    Paste,
    Cancel,
}

pub(super) fn paste_controls(rotation_open: bool, target: Option<Selection>) -> Option<Selection> {
    (!rotation_open).then_some(target).flatten()
}

pub(super) fn draw_paste_controls(
    ui: &Ui, camera: &Controller, target: Selection, tile_size: u32, viewport_min: [f32; 2],
    controls_bounds: OverlayRect, can_paste: bool,
) -> Option<PasteAction> {
    let (position, bounds) = block_placement_controls_layout(
        ui,
        camera,
        PlacementControls::Paste,
        target,
        tile_size,
        viewport_min,
        controls_bounds,
    );
    draw_overlay_underlay(ui, bounds);
    ui.set_cursor_screen_pos(position);

    let mut action = None;
    if ui.with_disabled_if(!can_paste, || ui.button(PASTE_LABELS[0])) {
        action = Some(PasteAction::Paste);
    }
    ui.same_line();
    if ui.button(PASTE_LABELS[1]) {
        action = Some(PasteAction::Cancel);
    }
    if action.is_none()
        && ui.is_window_focused()
        && !ui.io().want_text_input()
        && !ui.is_popup_open_with_flags("", dear_imgui_rs::PopupQueryFlags::ANY_POPUP)
        && ui.is_key_pressed(Key::Enter)
        && can_paste
    {
        action = Some(PasteAction::Paste);
    }

    action
}

pub(super) fn centered_paste_min(width: u32, height: u32, anchor: Coord, map_size: Size) -> Coord {
    let place = |anchor: u32, extent: u32, limit: u32| {
        let extent = extent.max(1);
        let last = limit.saturating_sub(extent - 1).max(1);

        anchor.saturating_sub((extent - 1) / 2).max(1).min(last)
    };

    Coord::new(
        place(anchor.x, width, map_size.x),
        place(anchor.y, height, map_size.y),
        anchor.z,
    )
}

#[cfg(test)]
mod tests {
    use dmm::{Coord, Size};

    use super::centered_paste_min;

    #[test]
    fn a_pasted_block_lands_centered_on_the_cursor() {
        let size = Size { x: 20, y: 20, z: 1 };

        // An odd extent sits dead centre; an even one leans one tile left/down.
        assert_eq!(
            centered_paste_min(3, 3, Coord::new(10, 10, 1), size),
            Coord::new(9, 9, 1)
        );
        assert_eq!(
            centered_paste_min(4, 4, Coord::new(10, 10, 1), size),
            Coord::new(9, 9, 1)
        );
        assert_eq!(
            centered_paste_min(1, 1, Coord::new(10, 10, 1), size),
            Coord::new(10, 10, 1)
        );
    }

    #[test]
    fn a_pasted_block_is_nudged_back_inside_the_map() {
        let size = Size { x: 10, y: 10, z: 1 };

        // Centring near an edge would hang the block off the map, so it slides
        // back until the whole footprint fits.
        assert_eq!(centered_paste_min(5, 5, Coord::new(1, 1, 1), size), Coord::new(1, 1, 1));
        assert_eq!(
            centered_paste_min(5, 5, Coord::new(10, 10, 1), size),
            Coord::new(6, 6, 1)
        );
        // A block larger than the map still starts at the origin rather than
        // clamping to nothing.
        assert_eq!(
            centered_paste_min(40, 40, Coord::new(5, 5, 1), size),
            Coord::new(1, 1, 1)
        );
    }
}
