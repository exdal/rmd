use base::vars;
use prefab::PrefabInstanceId;

use dear_imgui_rs::{Ui, WindowKey, WindowKeyError};
use ui::widgets::focus_window_on_hover;

use crate::{
    app::external_editor::SourceLocation, document::DocumentId, inspector::session::EditScope, session::Session,
    settings::Settings,
};

use self::state::{InspectorOutput, InspectorState};

mod direction;
mod property;
mod snapshot;
mod state;
mod variable;

const DISPLAY_ROWS: &[&str] = &[
    vars::NAME,
    vars::ICON,
    vars::ICON_STATE,
    vars::DIR,
    vars::PLANE,
    vars::LAYER,
    vars::COLOR,
    vars::ALPHA,
    vars::INVISIBILITY,
];
const ADVANCED_OFFSETS: &[&str] = &[vars::PIXEL_W, vars::PIXEL_Z];

/// Marks everything an identical scope edit reaches, in the inspector and on the map
pub(crate) const IDENTICAL_EDIT_COLOR: [f32; 4] = [1.0, 0.7, 0.15, 1.0];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum TransformMode {
    #[default]
    Pixel,
    Step,
}

impl TransformMode {
    const ALL: [Self; 2] = [Self::Pixel, Self::Step];

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Pixel => "Pixel",
            Self::Step => "Step",
        }
    }

    pub(crate) const fn variables(self) -> (&'static str, &'static str) {
        match self {
            Self::Pixel => (vars::PIXEL_X, vars::PIXEL_Y),
            Self::Step => (vars::STEP_X, vars::STEP_Y),
        }
    }
}

pub(crate) struct InspectorPanel {
    window: WindowKey,
    state: InspectorState,
}

#[derive(Default)]
pub(crate) struct InspectorPanelOutput {
    pub(crate) open_source: Option<SourceLocation>,
    pub(crate) find_similar: Option<(DocumentId, PrefabInstanceId)>,
    pub(crate) copy: Option<String>,
    pub(crate) docked: bool,
}

impl InspectorPanel {
    pub(crate) fn new() -> Result<Self, WindowKeyError> {
        Ok(Self {
            window: WindowKey::new("inspector", "Inspector")?,
            state: InspectorState::default(),
        })
    }

    pub(crate) const fn window(&self) -> &WindowKey {
        &self.window
    }

    pub(crate) const fn transform_mode(&self) -> TransformMode {
        self.state.transform_mode()
    }

    pub(crate) fn edits_identical(&self) -> bool {
        self.state.scope == EditScope::Identical
    }

    pub(crate) fn draw(
        &mut self, ui: &Ui, session: &mut Session, settings: &mut Settings, focus: bool, open: &mut bool,
    ) -> InspectorPanelOutput {
        if !*open {
            return InspectorPanelOutput::default();
        }

        let mut output = InspectorOutput::default();
        let mut docked = false;
        ui.window(&self.window).opened(open).focused(focus).build(|| {
            docked = ui.is_window_docked();
            if settings.focus_windows_on_hover {
                focus_window_on_hover(ui);
            }
            output = self
                .state
                .draw(ui, session, &mut settings.pinned_vars, &mut settings.pinned_types);
        });
        let find_similar = output
            .find_similar
            .then(|| session.state.active_document())
            .flatten()
            .and_then(|document| Some((document.id(), document.picked_instance()?)));

        InspectorPanelOutput {
            open_source: output.open_source,
            find_similar,
            copy: output.copy,
            docked,
        }
    }
}

#[cfg(test)]
mod tests;
