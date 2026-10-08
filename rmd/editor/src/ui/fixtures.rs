use core::path::TreePath;
use std::{
    ffi::CStr,
    ops::{Deref, DerefMut},
    path::PathBuf,
};

use dear_imgui_rs::{BackendFlags, Condition, Context, Key, MouseButton, sys};
use dmm::{Prefab, Size};
use editor::document::DocumentId;
use render::MapViewInteraction;

use super::{
    MapViewState,
    OverlayRect,
    PlacementControls,
    UiState,
    block_placement_controls_layout,
    conflict_controls_layout,
    menu::MenuActions,
    overlay_padding,
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
    context.io_mut().set_config_macosx_behaviors(false);
    context
}

pub(super) struct PopupContext(Context);

impl Deref for PopupContext {
    type Target = Context;

    fn deref(&self) -> &Context { &self.0 }
}

impl DerefMut for PopupContext {
    fn deref_mut(&mut self) -> &mut Context { &mut self.0 }
}

impl Drop for PopupContext {
    fn drop(&mut self) { self.0.destroy_platform_windows().unwrap(); }
}

/// Optionally installs a headless backend for ImGui's desktop-coordinate viewport behavior.
pub(super) fn popup_context(desktop: bool) -> PopupContext {
    let mut context = rectangle_context();
    if !desktop {
        return PopupContext(context);
    }

    unsafe extern "C" fn create(viewport: *mut sys::ImGuiViewport) {
        unsafe { (*viewport).PlatformHandle = std::ptr::dangling_mut::<u8>().cast() };
    }
    unsafe extern "C" fn destroy(viewport: *mut sys::ImGuiViewport) {
        unsafe { (*viewport).PlatformHandle = std::ptr::null_mut() };
    }
    unsafe extern "C" fn noop(_: *mut sys::ImGuiViewport) {}
    unsafe extern "C" fn set_vec2(_: *mut sys::ImGuiViewport, _: *const sys::ImVec2) {}
    unsafe extern "C" fn set_title(_: *mut sys::ImGuiViewport, _: *const std::ffi::c_char) {}
    unsafe extern "C" fn get_pos(viewport: *mut sys::ImGuiViewport, output: *mut sys::ImVec2) {
        unsafe { *output = (*viewport).Pos };
    }
    unsafe extern "C" fn get_size(viewport: *mut sys::ImGuiViewport, output: *mut sys::ImVec2) {
        unsafe { *output = (*viewport).Size };
    }

    let flags =
        context.io().backend_flags() | BackendFlags::PLATFORM_HAS_VIEWPORTS | BackendFlags::RENDERER_HAS_VIEWPORTS;
    context.io_mut().set_backend_flags(flags);
    // SAFETY: the callbacks only access live ImGui viewports. The test owns the context and
    // uses inert handles without allocating native windows or renderer resources.
    unsafe {
        let platform = context.platform_io_mut();
        platform.set_platform_create_window_raw(Some(create));
        platform.set_platform_destroy_window_raw(Some(destroy));
        platform.set_platform_show_window_raw(Some(noop));
        platform.set_platform_set_window_pos_raw(Some(set_vec2));
        platform.set_platform_get_window_pos_raw(Some(get_pos));
        platform.set_platform_set_window_size_raw(Some(set_vec2));
        platform.set_platform_get_window_size_raw(Some(get_size));
        platform.set_platform_set_window_title_raw(Some(set_title));
        platform.set_monitors(&[sys::ImGuiPlatformMonitor {
            MainPos: sys::ImVec2 { x: 0.0, y: 0.0 },
            MainSize: sys::ImVec2 { x: 3840.0, y: 2160.0 },
            WorkPos: sys::ImVec2 { x: 0.0, y: 0.0 },
            WorkSize: sys::ImVec2 { x: 3840.0, y: 2160.0 },
            DpiScale: 1.0,
            PlatformHandle: std::ptr::null_mut(),
        }]);
        create(context.main_viewport().as_raw().cast_mut());
    }
    context.enable_multi_viewport();
    PopupContext(context)
}

pub(super) fn set_desktop_geometry(context: &mut Context, position: [f32; 2], size: [f32; 2]) {
    context.io_mut().set_display_size(size);
    // SAFETY: the test owns the context; its headless backend reads this simulated native position.
    unsafe {
        (*context.main_viewport().as_raw().cast_mut()).Pos = sys::ImVec2 {
            x: position[0],
            y: position[1],
        };
    }
}

pub(super) fn finish_frame(context: &mut Context) {
    assert!(context.render_legacy().valid());
    context.update_platform_windows();
}

pub(super) fn assert_window_centered(context: &mut Context, name: &CStr) {
    let center = context.main_viewport().work_center();
    // SAFETY: the test owns the context and inspects its live, rendered window.
    unsafe {
        let window = sys::igFindWindowByName(name.as_ptr());
        assert!(!window.is_null());
        let window = &*window;
        assert!(window.Active && !window.Hidden, "the popup should be visible");
        assert_eq!(
            window.Viewport.cast_const().cast::<sys::ImGuiViewport>(),
            context.main_viewport().as_raw()
        );
        for (axis, actual) in [window.Pos.x + window.Size.x / 2.0, window.Pos.y + window.Size.y / 2.0]
            .into_iter()
            .enumerate()
        {
            assert!(
                (actual - center[axis]).abs() <= 1.0,
                "popup center {actual}, viewport center {}",
                center[axis]
            );
        }
    }
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
    pub(super) is_modal_open: bool,
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
            is_modal_open: false,
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
                coop: &mut Default::default(),
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
                min: [min[0], min[1] + ui.frame_height() + 2.0 * overlay_padding(ui)],
                max: [max[0], max[1] - recent_button_size(ui) - 2.0 * overlay_padding(ui)],
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
                min: [min[0], min[1] + ui.frame_height() + 2.0 * overlay_padding(ui)],
                max: [max[0], max[1] - recent_button_size(ui) - 2.0 * overlay_padding(ui)],
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
