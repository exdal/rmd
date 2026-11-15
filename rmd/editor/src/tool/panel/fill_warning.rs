use base::path::TreePath;

use dear_imgui_rs::Ui;
use dmm::{Coord, Prefab};
use ui::widgets::{DialogButton, begin_centered_modal, dialog_buttons};

use crate::{
    document::DocumentId,
    frontend::dialog::MODAL_FLAGS,
    session::Session,
    tool::{FillMode, SelectionMask, Tool},
};

pub(crate) const FILL_LIMIT_WARNING_POPUP: &str = "Large fill##fill-limit-warning";

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PendingFillWarning {
    pub(crate) context: FillWarningContext,
    pub(crate) coord: Coord,
    pub(crate) fill_mode: FillMode,
    pub(crate) custom_fill_boundaries: Vec<TreePath>,
    pub(crate) limit: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FillWarningContext {
    document: Option<DocumentId>,
    revision: Option<u64>,
    z: u32,
    mask: Option<SelectionMask>,
    prefab: Option<Prefab>,
    focus: Option<dmm::PrefabInstanceId>,
}

impl FillWarningContext {
    pub(crate) fn capture(session: &Session) -> Self {
        Self {
            document: session.state.active(),
            revision: session.edit_revision(),
            z: session.z(),
            mask: session.selection_mask(),
            prefab: session.palette().cloned(),
            focus: session.focused_area(),
        }
    }
}

pub(crate) fn draw_fill_limit_warning(
    ui: &Ui, session: &mut Session, pending: &mut Option<PendingFillWarning>, fill_mode: FillMode,
    boundaries: &[TreePath],
) {
    let Some(warning) = pending.as_ref() else {
        return;
    };

    let Some(_modal) = begin_centered_modal(ui, FILL_LIMIT_WARNING_POPUP, MODAL_FLAGS) else {
        return;
    };

    let mut fill_anyway = false;
    let mut dismiss = false;

    if !warning.matches(session, fill_mode, boundaries) {
        ui.close_current_popup();
        *pending = None;
        return;
    }

    ui.text(format!("This fill would change more than {} tiles.", warning.limit));
    ui.text("The operation may make the editor unresponsive.");
    ui.text("Do you want to fill it anyway?");

    let pressed = dialog_buttons(
        ui,
        &[DialogButton::cancel("Cancel"), DialogButton::primary("Fill Anyway")],
        true,
    );

    if pressed == Some(0) {
        dismiss = true;
        ui.close_current_popup();
    }

    if pressed == Some(1) {
        fill_anyway = true;
        dismiss = true;
        ui.close_current_popup();
    }

    if dismiss
        && let Some(warning) = pending.take()
        && fill_anyway
        && warning.matches(session, fill_mode, boundaries)
    {
        session.fill_at_unlimited(warning.coord, warning.fill_mode, &warning.custom_fill_boundaries);
    }
}

impl PendingFillWarning {
    fn matches(&self, session: &Session, fill_mode: FillMode, boundaries: &[TreePath]) -> bool {
        session.tool() == Tool::Fill
            && self.context == FillWarningContext::capture(session)
            && self.fill_mode == fill_mode
            && self.custom_fill_boundaries == boundaries
    }
}

#[cfg(test)]
mod tests {
    use base::path::TreePath;

    use dmm::{Coord, Prefab, Size};

    use super::{FillWarningContext, PendingFillWarning};
    use crate::{
        document::{MapDocument, selection::Selection},
        session::Session,
        tool::{FillMode, RegionSelectionMode, SelectionMask, Tool},
    };

    #[test]
    fn pending_fill_is_invalidated_when_the_mask_document_or_palette_changes() {
        let mut session = Session::new();
        let size = Size { x: 10, y: 10, z: 2 };
        let id = session.state.open_document(MapDocument::new(dmm::Map::new(size), 1));
        session.set_tool(Tool::Fill);
        let mask = SelectionMask {
            bounds: Selection::from_drag(Coord::new(2, 2, 1), Coord::new(4, 4, 1)),
            mode: RegionSelectionMode::Full,
        };
        session.try_select_region(mask);
        let warning = PendingFillWarning {
            context: FillWarningContext::capture(&session),
            coord: mask.bounds.min,
            fill_mode: FillMode::Wall,
            custom_fill_boundaries: vec![],
            limit: 5000,
        };
        assert!(warning.matches(&session, FillMode::Wall, &[]));
        assert!(!warning.matches(&session, FillMode::EntireArea, &[]));
        assert!(!warning.matches(&session, FillMode::Wall, &[TreePath::parse("/obj/window")]));
        session.try_select_region(SelectionMask {
            mode: RegionSelectionMode::Hollow { line_width: 1 },
            ..mask
        });
        assert!(!warning.matches(&session, FillMode::Wall, &[]));
        session.try_select_region(mask);
        session.state.open_document(MapDocument::new(dmm::Map::new(size), 1));
        assert!(!warning.matches(&session, FillMode::Wall, &[]));
        session.state.set_active(id);
        session
            .state
            .choose_prefab(Prefab::new(TreePath::parse("/turf/open/floor")));
        assert!(!warning.matches(&session, FillMode::Wall, &[]));
    }
}
