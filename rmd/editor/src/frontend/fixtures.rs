use base::path::TreePath;
use std::{ffi::CStr, fs, path::PathBuf};

use dear_imgui_rs::{Condition, Key, MouseButton, Ui, sys};
use dmm::{Prefab, Size};
use render::MapViewInteraction;
use ui::testing::rectangle_context;

use super::{UiState, menu::MenuActions};
use crate::{
    document::DocumentId,
    maploader::panel::controls_layout as module_controls_layout,
    palette::panel::history::recent_button_size,
    session::Session,
    settings::Settings,
    viewport::panel::{
        MapViewDraw, MapViewState,
        overlay::{OverlayRect, conflict_controls_layout, overlay_padding},
    },
};

pub(crate) struct RectangleUiHarness {
    pub(crate) context: dear_imgui_rs::Context,
    pub(crate) state: UiState,
    pub(crate) session: Session,
    pub(crate) settings: Settings,
    pub(crate) view: MapViewState,
    pub(crate) interaction: MapViewInteraction,
    pub(crate) id: DocumentId,
    pub(crate) module_buttons: Option<[[f32; 2]; 3]>,
    pub(crate) conflict_button: Option<[f32; 2]>,
    pub(crate) focus_other_window: bool,
    pub(crate) is_modal_open: bool,
}

impl RectangleUiHarness {
    pub(crate) fn new() -> Self {
        let mut session = Session::new();
        session
            .load_environment(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/env/test.dme"))
            .unwrap();
        let mut map = dmm::Map::new(Size { x: 20, y: 20, z: 1 });
        let base = map.intern_tile(vec![
            Prefab::new(TreePath::parse("/turf/open/floor")),
            Prefab::new(TreePath::parse("/area/station")),
        ]);
        for row in &mut map.grid[0] {
            row.fill(base);
        }
        session.apply_map(crate::environment::worker::LoadedMap {
            path: PathBuf::from("rectangle-ui-test.dmm"),
            map,
            z: 1,
            errors: vec![],
            repo: None,
            conflict: None,
        });

        Self::with_session(session)
    }

    pub(crate) fn with_session(session: Session) -> Self {
        let context = rectangle_context();
        let id = session.state.active().unwrap();
        let mut view = MapViewState::new(id).unwrap();
        view.refit = false;
        view.focus = true;
        view.camera.camera.x = 320.0;
        view.camera.camera.y = 320.0;
        let mut harness = Self {
            context,
            state: UiState::new(false).unwrap(),
            session,
            settings: Settings::default(),
            view,
            interaction: MapViewInteraction::default(),
            id,
            module_buttons: None,
            conflict_button: None,
            focus_other_window: false,
            is_modal_open: false,
        };
        for _ in 0..3 {
            harness.step();
        }
        harness
    }

    pub(crate) fn step(&mut self) {
        let ui = self.context.frame();
        if self.focus_other_window {
            ui.window("other-window")
                .position([680.0, 0.0], Condition::Always)
                .size([100.0, 100.0], Condition::Always)
                .focused(true)
                .build(|| ui.text("Other window"));
        }
        let mut menu = MenuActions::default();
        self.state.read_edit_keys(ui, &self.session, &self.settings, &mut menu);
        self.state.edit_command = self.state.edit_command.or(menu.edit);
        self.state.apply_history_actions(&mut self.session, &menu);
        if self.is_modal_open {
            if !ui.is_popup_open("test-modal") {
                ui.open_popup("test-modal");
            }

            if let Some(_modal) = ui.begin_modal_popup("test-modal") {
                ui.text("Modal");
            }
        }

        let name = format!("###viewport-{}", self.id.get());
        ui.set_window_pos_by_name(&name, [0.0; 2]);
        ui.set_window_size_by_name(&name, [800.0, 600.0]);
        self.interaction = MapViewInteraction::default();
        self.state.draw_map_view(
            ui,
            &mut self.session,
            &mut self.settings,
            MapViewDraw {
                id: self.id,
                index: 0,
                view: &mut self.view,
                interaction: &mut self.interaction,
                guide_badges: &[],

                refit_requested: false,
                keep_open: &mut true,
                coop: &mut Default::default(),
            },
        );

        self.module_buttons = self.view.gestures.module_popup.as_ref().map(|(key, coord)| {
            let min = [self.view.rect.x as f32, self.view.rect.y as f32];
            let max = [
                min[0] + self.view.rect.width as f32,
                min[1] + self.view.rect.height as f32,
            ];
            let controls = OverlayRect {
                min: [min[0], min[1] + ui.frame_height() + 2.0 * overlay_padding(ui)],
                max: [max[0], max[1] - recent_button_size(ui) - 2.0 * overlay_padding(ui)],
            };
            let (position, width, _) = module_controls_layout(
                ui,
                &self.view.camera,
                *coord,
                self.session.options.tile_size,
                min,
                controls,
                !key.ancestry.is_empty(),
            );
            let button = ui.frame_height();
            let spacing = ui.clone_style().item_spacing()[0];

            [
                [position[0] + 30.0, position[1] + button / 2.0],
                [position[0] + width - 1.5 * button - spacing, position[1] + button / 2.0],
                [position[0] + width - button / 2.0, position[1] + button / 2.0],
            ]
        });
        self.conflict_button = None;
        if let Some(hunk) = self.session.conflict_hunks(self.id, Some(self.session.z())).first() {
            let min = [self.view.rect.x as f32, self.view.rect.y as f32];
            let max = [
                min[0] + self.view.rect.width as f32,
                min[1] + self.view.rect.height as f32,
            ];
            let controls = OverlayRect {
                min: [min[0], min[1] + ui.frame_height() + 2.0 * overlay_padding(ui)],
                max: [max[0], max[1] - recent_button_size(ui) - 2.0 * overlay_padding(ui)],
            };
            if let Some((position, _)) = conflict_controls_layout(
                ui,
                &self.view.camera,
                hunk,
                self.session.options.tile_size,
                min,
                controls,
                "theirs",
            ) {
                let style = ui.clone_style();
                let head_width = ui.calc_text_size("HEAD")[0] + style.frame_padding()[0] * 2.0;
                let their_width = ui.calc_text_size("theirs")[0] + style.frame_padding()[0] * 2.0;
                self.conflict_button = Some([
                    position[0] + head_width + style.item_spacing()[0] + their_width * 0.5,
                    position[1] + ui.frame_height() * 0.5,
                ]);
            }
        }
        assert!(self.context.render_legacy().valid());
    }

    pub(crate) fn tile(&self, x: u32, y: u32) -> [f32; 2] {
        let point = self
            .view
            .camera
            .map_to_screen([(x as f32 - 0.5) * 32.0, (y as f32 - 0.5) * 32.0]);
        [point[0] + self.view.rect.x as f32, point[1] + self.view.rect.y as f32]
    }

    pub(crate) fn pointer(&mut self, point: [f32; 2], down: bool) {
        self.context.io_mut().add_mouse_pos_event(point);
        self.context.io_mut().add_mouse_button_event(MouseButton::Left, down);
        self.step();
    }

    pub(crate) fn drag(&mut self, from: [f32; 2], to: [f32; 2]) {
        self.pointer(from, false);
        self.pointer(from, true);
        self.pointer(to, true);
        self.pointer(to, false);
    }

    pub(crate) fn click(&mut self, point: [f32; 2]) {
        self.pointer(point, false);
        self.pointer(point, true);
        self.pointer(point, false);
    }

    pub(crate) fn key(&mut self, key: Key, down: bool) {
        self.context.io_mut().add_key_event(key, down);
        self.step();
    }
}

pub(crate) fn button_centers(ui: &Ui, name: &CStr, labels: &[&str]) -> Vec<[f32; 2]> {
    // SAFETY: this function runs during the test's current frame with CONTEXT_LOCK held.
    let Some(window) = (unsafe { sys::igFindWindowByName(name.as_ptr()).as_ref() }) else {
        return Vec::new();
    };
    if !window.Active {
        return Vec::new();
    }
    let style = ui.clone_style();
    let total = labels
        .iter()
        .map(|label| ui.calc_text_size(label)[0] + 2.0 * style.frame_padding()[0])
        .sum::<f32>()
        + style.item_spacing()[0] * labels.len().saturating_sub(1) as f32;
    let mut x = window.Pos.x + window.Size.x - style.window_padding()[0] - total;
    let y = window.Pos.y + window.Size.y - style.window_padding()[1] - ui.frame_height() / 2.0;
    labels
        .iter()
        .map(|label| {
            let width = ui.calc_text_size(label)[0] + 2.0 * style.frame_padding()[0];
            let center = [x + width / 2.0, y];
            x += width + style.item_spacing()[0];
            center
        })
        .collect::<Vec<_>>()
}

pub(crate) fn click_dialog_button(
    context: &mut dear_imgui_rs::Context, name: &CStr, labels: &[&str], mut draw: impl FnMut(&Ui),
) {
    let mut point = [0.0, 0.0];
    for _ in 0..4 {
        let ui = context.frame();
        ui.open_popup(name.to_str().unwrap());
        draw(ui);
        point = *button_centers(ui, name, labels).last().unwrap();
        assert!(context.render_legacy().valid());
    }

    for down in [false, true, false] {
        context.io_mut().add_mouse_pos_event(point);
        context.io_mut().add_mouse_button_event(MouseButton::Left, down);
        let ui = context.frame();
        draw(ui);
        assert!(context.render_legacy().valid());
    }
}

pub(crate) struct TestDirectory(pub(crate) PathBuf);

impl TestDirectory {
    pub(crate) fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("rmd-close-{name}-{}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
