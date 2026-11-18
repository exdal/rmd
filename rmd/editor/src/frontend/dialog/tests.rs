use std::{
    ffi::{CStr, CString},
    path::{Path, PathBuf},
};

use dear_imgui_rs::{ConfigFlags, Key, MouseButton, sys};
use dmm::{Map, Size};
use ui::{
    testing::{CONTEXT_LOCK, PopupContext, assert_window_centered, finish_frame, popup_context, set_desktop_geometry},
    theme::{DEFAULT_THEME, Themes},
};

use super::{KEYBIND_PRESET_POPUP, TRUST_POPUP, draw_keybind_preset_dialog};
use crate::{
    document::{DocumentId, MapDocument},
    frontend::{UiState, fixtures::button_centers},
    session::Session,
    settings::Settings,
    testing::TempDir,
};

struct CloseUi {
    context: PopupContext,
    state: UiState,
    session: Session,
    settings: Settings,
    themes: Themes,
    close_buttons: Vec<[f32; 2]>,
    save_buttons: Vec<[f32; 2]>,
    close_open: bool,
    save_open: bool,
}

impl CloseUi {
    fn new() -> Self {
        let mut context = popup_context(false);
        context.io_mut().set_display_size([1280.0, 720.0]);
        Self::with_context(context)
    }

    fn with_context(mut context: PopupContext) -> Self {
        let flags = context.io().config_flags() | ConfigFlags::DOCKING_ENABLE;
        context.io_mut().set_config_flags(flags);
        let themes = Themes::load(None, context.style(), DEFAULT_THEME);
        Self {
            context,
            state: UiState::new(false).unwrap(),
            session: Session::new(),
            settings: Settings {
                check_for_updates: false,
                focus_windows_on_hover: true,
                ..Settings::default()
            },
            themes,
            close_buttons: Vec::new(),
            save_buttons: Vec::new(),
            close_open: false,
            save_open: false,
        }
    }

    fn add_map(&mut self, path: impl AsRef<Path>, dirty: bool, needs_path: bool) -> DocumentId {
        let path = path.as_ref().to_path_buf();
        let map = Map::new(Size { x: 4, y: 4, z: 1 });
        let mut document = if needs_path {
            MapDocument::create(path, map, 1)
        } else {
            MapDocument::open(path, map, 1)
        };
        if dirty {
            document.mark_unsaved();
        }
        self.session.state.open_document(document)
    }

    fn step(&mut self) {
        let ui = self.context.frame();
        self.state
            .draw(ui, &mut self.session, &mut self.settings, &mut self.themes, None)
            .unwrap();
        self.close_open = ui.is_popup_open("Unsaved changes##close-map");
        self.save_open = ui.is_popup_open("Save map##save-map");
        self.close_buttons = button_centers(
            ui,
            c"Unsaved changes##close-map",
            &["Cancel", "Discard Changes", "Save Map"],
        );
        self.save_buttons = button_centers(ui, c"Save map##save-map", &["Cancel", "Save Map"]);
        finish_frame(&mut self.context);
    }

    fn settle(&mut self) {
        for _ in 0..4 {
            self.step();
        }
    }

    fn select(&mut self, id: DocumentId) {
        self.settle();
        self.session.set_active_document(id);
        self.state.map_views.get_mut(&id).unwrap().focus = true;
        self.settle();
        self.assert_selected_map(id);
    }

    fn click(&mut self, point: [f32; 2]) {
        for down in [false, true, false] {
            self.context.io_mut().add_mouse_pos_event(point);
            self.context.io_mut().add_mouse_button_event(MouseButton::Left, down);
            self.step();
        }
        self.settle();
    }

    fn close_tab(&mut self, id: DocumentId) {
        let keep_selected =
            self.session.state.active() == Some(id) && self.session.state.document(id).unwrap().is_dirty();
        let name = CString::new(format!("###viewport-{}", id.get())).unwrap();
        // SAFETY: this test owns the current context while holding CONTEXT_LOCK.
        let point = unsafe {
            let window = &*sys::igFindWindowByName(name.as_ptr());
            let rect = window.DC.DockTabItemRect;
            let font_size = (*self.context.as_raw()).FontSize;
            [
                rect.Max.x - self.context.style().frame_padding()[0] - font_size / 2.0,
                (rect.Min.y + rect.Max.y) / 2.0,
            ]
        };
        for down in [false, true, false] {
            self.context.io_mut().add_mouse_pos_event(point);
            self.context.io_mut().add_mouse_button_event(MouseButton::Left, down);
            self.step();
            if keep_selected && self.state.close_queue.contains(&id) {
                self.assert_selected_map(id);
            }
        }
        self.settle();
    }

    fn escape(&mut self) {
        self.context.io_mut().add_key_event(Key::Escape, true);
        self.step();
        self.context.io_mut().add_key_event(Key::Escape, false);
        self.settle();
    }

    fn assert_selected_map(&self, id: DocumentId) {
        assert_eq!(self.session.state.active(), Some(id));
        let name = CString::new(format!("###viewport-{}", id.get())).unwrap();
        self.assert_selected(&name);
    }

    fn assert_selected(&self, name: &CStr) {
        // SAFETY: the test owns the current context and its rendered docked windows.
        unsafe {
            let window = sys::igFindWindowByName(name.as_ptr());
            assert!(!window.is_null());
            let dock = (*window).DockNode;
            assert!(!dock.is_null(), "the tested window must be docked");
            assert!(
                (*window).DockTabIsVisible(),
                "the requested tab should be selected and visible"
            );
        }
    }
}

#[test]
fn dirty_close_popup_recenters_after_leaving_fullscreen() {
    let _guard = CONTEXT_LOCK.lock().unwrap();
    for desktop in [false, true] {
        let context = popup_context(desktop);
        let mut app = CloseUi::with_context(context);
        set_desktop_geometry(&mut app.context, [0.0, 0.0], [1920.0, 1080.0]);
        let id = app.add_map("dirty.dmm", true, true);
        app.select(id);
        app.state.request_close(&mut app.session, [id]);
        app.settle();
        assert!(app.close_open);
        assert_window_centered(&mut app.context, c"Unsaved changes##close-map");
        app.escape();

        let position = if desktop { [180.0, 90.0] } else { [0.0, 0.0] };
        set_desktop_geometry(&mut app.context, position, [960.0, 640.0]);
        app.settle();
        app.state.request_close(&mut app.session, [id]);
        app.settle();
        assert!(app.close_open);
        assert_eq!(app.context.main_viewport().size(), [960.0, 640.0]);
        assert_eq!(app.context.main_viewport().pos(), position);
        assert_window_centered(&mut app.context, c"Unsaved changes##close-map");

        set_desktop_geometry(&mut app.context, position, [900.0, 600.0]);
        app.settle();
        assert_window_centered(&mut app.context, c"Unsaved changes##close-map");
        app.click(app.close_buttons[2]);
        assert!(app.save_open && !app.close_open);
        assert_window_centered(&mut app.context, c"Save map##save-map");
    }
}

#[test]
fn dirty_tab_close_keeps_the_map_selected_until_cancel_or_escape() {
    let _guard = CONTEXT_LOCK.lock().unwrap();
    let mut app = CloseUi::new();
    let id = app.add_map("dirty.dmm", true, false);
    app.select(id);
    for escape in [false, true] {
        app.close_tab(id);
        assert!(app.close_open);
        app.assert_selected_map(id);
        // SAFETY: this test owns the context and its active confirmation modal.
        unsafe {
            let focused = (*app.context.as_raw()).NavWindow;
            assert!(!focused.is_null());
            assert_ne!((*focused).Flags & sys::ImGuiWindowFlags_Modal, 0);
        }
        if escape {
            app.escape();
        } else {
            app.click(app.close_buttons[0]);
        }
        assert!(!app.close_open);
        assert!(app.state.close_queue.is_empty());
        assert!(app.session.state.document(id).unwrap().is_dirty());
        app.assert_selected_map(id);
    }
}

#[test]
fn closing_an_inactive_dirty_tab_selects_that_map_for_confirmation() {
    let _guard = CONTEXT_LOCK.lock().unwrap();
    let mut app = CloseUi::new();
    let dirty = app.add_map("dirty.dmm", true, false);
    let active = app.add_map("active.dmm", false, false);
    app.select(active);
    app.close_tab(dirty);
    assert!(app.close_open);
    app.assert_selected_map(dirty);
    app.escape();
    app.assert_selected_map(dirty);
    assert!(app.session.state.document(active).is_some());
}

#[test]
fn clean_active_tab_closes_select_the_next_map_then_previous_then_welcome() {
    let _guard = CONTEXT_LOCK.lock().unwrap();
    let mut app = CloseUi::new();
    let first = app.add_map("first.dmm", false, false);
    let second = app.add_map("second.dmm", false, false);
    let third = app.add_map("third.dmm", false, false);
    app.select(second);
    for (closing, remaining) in [(second, Some(third)), (third, Some(first)), (first, None)] {
        app.close_tab(closing);
        assert!(!app.close_open);
        assert!(app.session.state.document(closing).is_none());
        if let Some(id) = remaining {
            app.assert_selected_map(id);
        } else {
            app.assert_selected(c"###welcome");
        }
    }
}

#[test]
fn discard_selects_the_next_map_then_the_previous_map_then_welcome() {
    let _guard = CONTEXT_LOCK.lock().unwrap();
    let mut app = CloseUi::new();
    let first = app.add_map("first.dmm", true, false);
    let second = app.add_map("second.dmm", true, false);
    let third = app.add_map("third.dmm", true, false);
    app.select(second);
    for (closing, remaining) in [(second, Some(third)), (third, Some(first)), (first, None)] {
        app.close_tab(closing);
        assert!(app.close_open);
        app.assert_selected_map(closing);
        app.click(app.close_buttons[1]);
        assert!(app.session.state.document(closing).is_none());
        if let Some(id) = remaining {
            app.assert_selected_map(id);
        } else {
            app.assert_selected(c"###welcome");
        }
    }
}

#[test]
fn closing_the_last_map_reopens_welcome_and_clean_background_closes_keep_focus() {
    let _guard = CONTEXT_LOCK.lock().unwrap();
    let mut app = CloseUi::new();
    let background = app.add_map("background.dmm", false, false);
    let id = app.add_map("last.dmm", true, false);
    app.select(id);
    app.close_tab(background);
    assert!(app.session.state.document(background).is_none());
    app.assert_selected_map(id);
    app.state.show_welcome = false;
    app.settle();
    app.close_tab(id);
    app.assert_selected_map(id);
    app.click(app.close_buttons[1]);
    assert!(app.state.show_welcome);
    app.assert_selected(c"###welcome");
}

#[test]
fn successful_save_closes_the_map_and_focuses_its_neighbor() {
    let _guard = CONTEXT_LOCK.lock().unwrap();
    let dir = TempDir::new("save");
    let file = dir.path().join("saved.dmm");
    let mut app = CloseUi::new();
    let id = app.add_map(&file, true, false);
    let next = app.add_map("next.dmm", false, false);
    app.select(id);
    app.close_tab(id);
    app.assert_selected_map(id);
    app.click(app.close_buttons[2]);
    assert!(file.is_file());
    assert!(app.session.state.document(id).is_none());
    assert!(!app.close_open);
    app.assert_selected_map(next);
}

#[test]
fn a_failed_save_keeps_the_confirmation_and_map_open() {
    let _guard = CONTEXT_LOCK.lock().unwrap();
    let dir = TempDir::new("save-failure");
    let mut app = CloseUi::new();
    let id = app.add_map(dir.path(), true, false);
    app.select(id);
    app.close_tab(id);
    app.click(app.close_buttons[2]);
    assert!(app.close_open);
    assert!(app.state.open_error.is_some());
    assert!(app.session.state.document(id).unwrap().is_dirty());
    app.assert_selected_map(id);
    app.escape();
}

#[test]
fn save_as_keeps_the_pending_close_until_it_succeeds() {
    let _guard = CONTEXT_LOCK.lock().unwrap();
    let dir = TempDir::new("save-as");
    let file = dir.path().join("saved.dmm");
    let mut app = CloseUi::new();
    let id = app.add_map(&file, true, true);
    let next = app.add_map("next.dmm", false, false);
    app.select(id);
    app.close_tab(id);
    app.click(app.close_buttons[2]);
    assert!(app.save_open && !app.close_open);
    assert_eq!(app.state.close_queue.front(), Some(&id));
    assert!(app.state.save_dialog.as_ref().unwrap().close_after_save);
    app.assert_selected_map(id);

    app.state.save_dialog.as_mut().unwrap().path = dir.path().display().to_string();
    app.settle();
    app.click(app.save_buttons[1]);
    assert!(app.save_open);
    assert!(app.state.save_dialog.as_ref().unwrap().error.is_some());
    app.assert_selected_map(id);

    app.state.save_dialog.as_mut().unwrap().path = file.display().to_string();
    app.settle();
    // Save As must save its requesting document even if session activation changes.
    app.session.set_active_document(next);
    app.click(app.save_buttons[1]);
    assert!(file.is_file());
    assert!(app.session.state.document(id).is_none());
    assert!(app.state.close_queue.is_empty());
    assert!(app.state.save_dialog.is_none());
    app.assert_selected_map(next);
}

#[test]
fn cancelling_save_as_cancels_the_close_batch_and_keeps_maps_open() {
    let _guard = CONTEXT_LOCK.lock().unwrap();
    let mut app = CloseUi::new();
    let first = app.add_map("first.dmm", true, true);
    let second = app.add_map("second.dmm", true, true);
    app.select(first);
    app.state.request_close(&mut app.session, [first, second]);
    app.settle();
    assert_eq!(
        app.state.close_queue.iter().copied().collect::<Vec<_>>(),
        [first, second]
    );
    app.click(app.close_buttons[2]);
    assert!(app.save_open && !app.close_open);
    app.click(app.save_buttons[0]);
    assert!(!app.save_open && !app.close_open);
    assert!(app.state.close_queue.is_empty());
    for id in [first, second] {
        assert!(app.session.state.document(id).unwrap().is_dirty());
    }
    app.assert_selected_map(first);
}

#[test]
fn save_as_success_advances_the_close_batch_to_the_next_dirty_map() {
    let _guard = CONTEXT_LOCK.lock().unwrap();
    let dir = TempDir::new("save-as-batch");
    let file = dir.path().join("first.dmm");
    let mut app = CloseUi::new();
    let first = app.add_map(&file, true, true);
    let second = app.add_map("second.dmm", true, true);
    app.select(first);
    app.state.request_close(&mut app.session, [first, second]);
    app.settle();
    app.click(app.close_buttons[2]);
    app.click(app.save_buttons[1]);
    assert!(file.is_file());
    assert!(app.session.state.document(first).is_none());
    assert!(app.close_open && !app.save_open);
    assert_eq!(app.state.close_queue.front(), Some(&second));
    app.assert_selected_map(second);
    app.escape();
    assert!(app.session.state.document(second).unwrap().is_dirty());
}

#[test]
fn closing_several_maps_asks_about_each_unsaved_one_and_cancel_keeps_the_rest_open() {
    let _guard = CONTEXT_LOCK.lock().unwrap();
    let mut context = ui::testing::rectangle_context();
    let mut session = Session::new();
    let map = || dmm::Map::new(Size { x: 1, y: 1, z: 1 });
    let clean = session.state.open_document(MapDocument::new(map(), 1));
    let first = session.state.open_document(MapDocument::create("first.dmm", map(), 1));
    let second = session.state.open_document(MapDocument::create("second.dmm", map(), 1));
    let mut state = UiState::new(false).unwrap();

    let ids = session.state.document_ids();
    state.request_close(&mut session, ids);
    assert!(
        session.state.document(clean).is_none(),
        "a map without changes closes at once"
    );
    assert_eq!(Vec::from(state.close_queue.clone()), [first, second]);

    session.state.close_document(first);
    let ui = context.frame();
    state.draw_close_confirmation(ui, &mut session);
    assert!(context.render_legacy().valid());
    assert_eq!(
        Vec::from(state.close_queue.clone()),
        [second],
        "a map that went away leaves the queue"
    );

    context.io_mut().add_key_event(dear_imgui_rs::Key::Escape, true);
    let ui = context.frame();
    state.draw_close_confirmation(ui, &mut session);
    assert!(context.render_legacy().valid());
    assert!(state.close_queue.is_empty());
    assert!(session.state.document(second).is_some());
}

#[test]
fn first_launch_opens_the_keybinding_preset_dialog() {
    let _context = CONTEXT_LOCK.lock().unwrap();
    let mut context = dear_imgui_rs::Context::create();
    context
        .font_atlas()
        .try_claim_legacy_renderer()
        .expect("legacy renderer font atlas should be available")
        .build();
    context.io_mut().set_display_size([1280.0, 720.0]);
    context.io_mut().set_delta_time(1.0 / 60.0);
    let ui = context.frame();
    let mut open = true;

    assert_eq!(draw_keybind_preset_dialog(ui, &mut open), None);
    assert!(open);
    assert!(ui.is_popup_open(KEYBIND_PRESET_POPUP));
    assert!(context.render_legacy().valid());
}

#[test]
fn trust_popup_stays_closed_without_a_prompt_and_waits_for_a_choice() {
    let _context = CONTEXT_LOCK.lock().unwrap();
    let mut context = dear_imgui_rs::Context::create();
    context
        .font_atlas()
        .try_claim_legacy_renderer()
        .expect("legacy renderer font atlas should be available")
        .build();
    context.io_mut().set_display_size([1280.0, 720.0]);
    context.io_mut().set_delta_time(1.0 / 60.0);
    let mut state = UiState::new(false).unwrap();

    let ui = context.frame();
    assert_eq!(state.draw_trust_popup(ui), None);
    assert!(!ui.is_popup_open(TRUST_POPUP));
    assert!(context.render_legacy().valid());

    state.set_trust_prompt(Some(PathBuf::from("station.dme")));
    let ui = context.frame();
    assert_eq!(state.draw_trust_popup(ui), None);
    assert!(ui.is_popup_open(TRUST_POPUP));
    assert!(state.trust_prompt.is_some());
    assert!(context.render_legacy().valid());
}
