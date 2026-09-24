use core::path::TreePath;
use std::path::PathBuf;

use dear_imgui_rs::{Condition, Key, MouseButton};
use dmm::{Prefab, Size};
use editor::document::DocumentId;
use render::MapViewInteraction;

use super::{
    MapViewState,
    OVERLAY_PADDING,
    OverlayRect,
    PlacementControls,
    UiState,
    block_placement_controls_layout,
    conflict_controls_layout,
    recent_button_size,
    viewport::MapViewDraw,
};
use crate::{session::Session, settings::Settings};

pub(super) fn rectangle_context() -> dear_imgui_rs::Context {
    let mut context = dear_imgui_rs::Context::create();
    context.set_ini_filename(None::<PathBuf>).unwrap();
    context.font_atlas().try_claim_legacy_renderer().unwrap().build();
    context.io_mut().set_display_size([800.0, 600.0]);
    context.io_mut().set_delta_time(1.0 / 60.0);
    context.io_mut().set_config_input_trickle_event_queue(false);
    context
}

pub(super) struct RectangleUiHarness {
    pub(super) context: dear_imgui_rs::Context,
    pub(super) state: UiState,
    pub(super) session: Session,
    pub(super) settings: Settings,
    pub(super) view: MapViewState,
    pub(super) id: DocumentId,
    pub(super) fill_button: Option<[f32; 2]>,
    pub(super) mode_buttons: Option<([f32; 2], [f32; 2])>,
    pub(super) conflict_button: Option<[f32; 2]>,
    pub(super) focus_other_window: bool,
}

impl RectangleUiHarness {
    pub(super) fn new() -> Self {
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
        session.apply_map(crate::loader::LoadedMap {
            path: PathBuf::from("rectangle-ui-test.dmm"),
            map,
            z: 1,
            errors: vec![],
            repo: None,
            conflict: None,
        });

        Self::with_session(session)
    }

    pub(super) fn with_session(session: Session) -> Self {
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
            id,
            fill_button: None,
            mode_buttons: None,
            conflict_button: None,
            focus_other_window: false,
        };
        for _ in 0..3 {
            harness.step();
        }
        harness
    }

    pub(super) fn step(&mut self) {
        let ui = self.context.frame();
        if self.focus_other_window {
            ui.window("other-window")
                .position([680.0, 0.0], Condition::Always)
                .size([100.0, 100.0], Condition::Always)
                .focused(true)
                .build(|| ui.text("Other window"));
        }
        let name = format!("###viewport-{}", self.id.get());
        ui.set_window_pos_by_name(&name, [0.0; 2]);
        ui.set_window_size_by_name(&name, [800.0, 600.0]);
        self.state.draw_map_view(
            ui,
            &mut self.session,
            &mut self.settings,
            MapViewDraw {
                id: self.id,
                index: 0,
                view: &mut self.view,
                interaction: &mut MapViewInteraction::default(),
                guide_badges: &[],

                refit_requested: false,
                keep_open: &mut true,
            },
        );

        self.fill_button = None;
        self.conflict_button = None;
        if let Some(region) = self.session.conflict_regions(self.id, Some(self.session.z())).first() {
            let min = [self.view.rect.x as f32, self.view.rect.y as f32];
            let max = [
                min[0] + self.view.rect.width as f32,
                min[1] + self.view.rect.height as f32,
            ];
            let controls = OverlayRect {
                min: [min[0], min[1] + ui.frame_height() + 2.0 * OVERLAY_PADDING],
                max: [max[0], max[1] - recent_button_size(ui) - 2.0 * OVERLAY_PADDING],
            };
            if let Some((position, _)) = conflict_controls_layout(
                ui,
                &self.view.camera,
                region,
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
        if let Some(selection) = self.session.selection() {
            let min = [self.view.rect.x as f32, self.view.rect.y as f32];
            let max = [
                min[0] + self.view.rect.width as f32,
                min[1] + self.view.rect.height as f32,
            ];
            let controls = OverlayRect {
                min: [min[0], min[1] + ui.frame_height() + 2.0 * OVERLAY_PADDING],
                max: [max[0], max[1] - recent_button_size(ui) - 2.0 * OVERLAY_PADDING],
            };
            let (position, _) = block_placement_controls_layout(
                ui,
                &self.view.camera,
                PlacementControls::Block,
                selection,
                self.session.options.tile_size,
                min,
                controls,
            );
            let style = ui.clone_style();
            let row_height = ui.frame_height() + style.item_spacing()[1];
            let button_width = |label: &str| ui.calc_text_size(label)[0] + style.frame_padding()[0] * 2.0;
            self.fill_button = Some([
                position[0]
                    + button_width("Move")
                    + button_width("Copy")
                    + 2.0 * style.item_spacing()[0]
                    + button_width("Fill selection") * 0.5,
                position[1] + 2.0 * row_height + ui.frame_height() * 0.5,
            ]);
            let radio_width =
                |label: &str| ui.frame_height() + style.item_inner_spacing()[0] + ui.calc_text_size(label)[0];
            self.mode_buttons = Some((
                [position[0] + 5.0, position[1] + row_height + 5.0],
                [
                    position[0] + radio_width("Border") + style.item_spacing()[0] + 5.0,
                    position[1] + row_height + 5.0,
                ],
            ));
        }

        assert!(self.context.render_legacy().valid());
    }

    pub(super) fn tile(&self, x: u32, y: u32) -> [f32; 2] {
        let point = self
            .view
            .camera
            .map_to_screen([(x as f32 - 0.5) * 32.0, (y as f32 - 0.5) * 32.0]);
        [point[0] + self.view.rect.x as f32, point[1] + self.view.rect.y as f32]
    }

    pub(super) fn pointer(&mut self, point: [f32; 2], down: bool) {
        self.context.io_mut().add_mouse_pos_event(point);
        self.context.io_mut().add_mouse_button_event(MouseButton::Left, down);
        self.step();
    }

    pub(super) fn click(&mut self, point: [f32; 2]) {
        self.pointer(point, false);
        self.pointer(point, true);
        self.pointer(point, false);
    }

    pub(super) fn key(&mut self, key: Key, down: bool) {
        self.context.io_mut().add_key_event(key, down);
        self.step();
    }
}
