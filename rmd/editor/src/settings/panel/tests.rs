use dear_imgui_rs::{Key, MouseButton};
use ui::{
    keybind::KeyBinding,
    testing::{CONTEXT_LOCK, assert_window_centered, finish_frame, popup_context, set_desktop_geometry},
    theme::DEFAULT_THEME,
    widgets::button_width,
};

use super::{
    appearance::{FontPicker, draw_ui_scale_setting},
    compiler::PROFILE_RELOAD_POPUP,
    keybinds::{SETTINGS_KEYBINDING_GROUPS, finish_keybind_capture},
    theme::ThemeEditor,
    *,
};

#[test]
fn settings_reopens_centered_after_resize_and_stays_draggable() {
    let _guard = CONTEXT_LOCK.lock().unwrap();
    for desktop in [false, true] {
        let mut context = popup_context(desktop);
        let mut window = SettingsWindow::new().unwrap();
        let mut session = Session::new();
        let mut settings = Settings {
            focus_windows_on_hover: false,
            ..Settings::default()
        };
        let mut themes = Themes::load(None, context.style(), DEFAULT_THEME);
        let mut draw = |context: &mut dear_imgui_rs::Context, window: &mut SettingsWindow| {
            window.draw(context.frame(), &mut session, &mut settings, &mut themes, false);
            finish_frame(context);
        };

        set_desktop_geometry(&mut context, [0.0, 0.0], [1920.0, 1080.0]);
        window.open();
        for _ in 0..4 {
            draw(&mut context, &mut window);
        }
        assert_window_centered(&mut context, c"###settings-v2");
        // SAFETY: the test owns the context and the rendered Settings window.
        let original = unsafe {
            let window = &*dear_imgui_rs::sys::igFindWindowByName(c"###settings-v2".as_ptr());
            [window.Pos.x, window.Pos.y]
        };
        let grab = [original[0] + 60.0, original[1] + 8.0];
        for down in [false, true] {
            context.io_mut().add_mouse_pos_event(grab);
            context.io_mut().add_mouse_button_event(MouseButton::Left, down);
            draw(&mut context, &mut window);
        }
        context.io_mut().add_mouse_pos_event([grab[0] - 150.0, grab[1] - 130.0]);
        draw(&mut context, &mut window);
        context.io_mut().add_mouse_button_event(MouseButton::Left, false);
        for _ in 0..3 {
            draw(&mut context, &mut window);
        }
        // SAFETY: the Settings window remains live in the test's current context.
        unsafe {
            let moved = &*dear_imgui_rs::sys::igFindWindowByName(c"###settings-v2".as_ptr());
            assert!((moved.Pos.x - (original[0] - 150.0)).abs() <= 1.0);
            assert!((moved.Pos.y - (original[1] - 130.0)).abs() <= 1.0);
        }

        window.open = false;
        draw(&mut context, &mut window);
        let position = if desktop { [180.0, 90.0] } else { [0.0, 0.0] };
        set_desktop_geometry(&mut context, position, [960.0, 640.0]);
        window.open();
        for _ in 0..4 {
            draw(&mut context, &mut window);
        }
        assert_eq!(context.main_viewport().pos(), position);
        assert_eq!(context.main_viewport().size(), [960.0, 640.0]);
        assert_window_centered(&mut context, c"###settings-v2");
    }
}

#[test]
fn ui_scale_changes_only_when_apply_is_clicked() {
    let _context = CONTEXT_LOCK.lock().unwrap();
    let mut context = ui::testing::rectangle_context();
    context.io_mut().set_display_framebuffer_scale([1.5, 1.5]);
    let mut settings = Settings::default();
    let mut draft = None;
    let draw = |context: &mut dear_imgui_rs::Context, settings: &mut Settings, draft: &mut Option<u32>| {
        let ui = context.frame();
        let mut centers = [[0.0; 2]; 2];
        ui.window("ui-scale-settings-test")
            .position([20.0, 20.0], Condition::Always)
            .size([650.0, 500.0], Condition::Always)
            .build(|| {
                draw_ui_scale_setting(ui, settings, draft);
                let min = ui.item_rect_min();
                let max = ui.item_rect_max();
                let y = (min[1] + max[1]) * 0.5;
                let apply_max = min[0] - ui.clone_style().item_spacing()[0];
                centers = [
                    [apply_max - button_width(ui, "Apply") * 0.5, y],
                    [(min[0] + max[0]) * 0.5, y],
                ];
            });
        assert!(context.render_legacy().valid());
        centers
    };
    let click =
        |context: &mut dear_imgui_rs::Context, settings: &mut Settings, draft: &mut Option<u32>, position: [f32; 2]| {
            context.io_mut().add_mouse_pos_event(position);
            draw(context, settings, draft);
            context.io_mut().add_mouse_button_event(MouseButton::Left, true);
            draw(context, settings, draft);
            context.io_mut().add_mouse_button_event(MouseButton::Left, false);
            draw(context, settings, draft);
        };

    draw(&mut context, &mut settings, &mut draft);
    assert_eq!(draft, Some(150));
    draft = Some(100);
    let [apply, _] = draw(&mut context, &mut settings, &mut draft);
    assert_eq!(settings.ui_scale_override_percent, None);

    click(&mut context, &mut settings, &mut draft, apply);
    assert_eq!(settings.ui_scale_override_percent, Some(100));

    draft = Some(150);
    let [apply, _] = draw(&mut context, &mut settings, &mut draft);
    click(&mut context, &mut settings, &mut draft, apply);
    assert_eq!(settings.ui_scale_override_percent, None);

    draft = Some(120);
    let [apply, _] = draw(&mut context, &mut settings, &mut draft);
    click(&mut context, &mut settings, &mut draft, apply);
    assert_eq!(settings.ui_scale_override_percent, Some(120));

    let [_, reset] = draw(&mut context, &mut settings, &mut draft);
    click(&mut context, &mut settings, &mut draft, reset);
    assert_eq!(settings.ui_scale_override_percent, None);
    assert_eq!(draft, Some(150));
}

#[test]
fn only_held_actions_capture_a_lone_modifier() {
    let _context = CONTEXT_LOCK.lock().unwrap();
    let mut context = dear_imgui_rs::Context::create();
    context.font_atlas().try_claim_legacy_renderer().unwrap().build();
    context.io_mut().set_display_size([800.0, 600.0]);
    context.io_mut().set_delta_time(1.0 / 60.0);
    let mut keybindings = KeyBindings::default();

    for (action, captured) in [(KeybindAction::Save, false), (KeybindAction::ToolAlternate, true)] {
        let mut capturing = Some(action);
        let before = keybindings.get(action);
        context.io_mut().add_key_event(Key::ModShift, true);
        let ui = context.frame();
        finish_keybind_capture(ui, &mut capturing, &mut keybindings);
        let label = keybindings.get(action).label(ui);
        assert!(context.render_legacy().valid());
        context.io_mut().add_key_event(Key::ModShift, false);
        context.frame();
        assert!(context.render_legacy().valid());

        assert_eq!(capturing.is_none(), captured, "{action:?}");
        if captured {
            assert_eq!(keybindings.get(action), KeyBinding::new(Key::ModShift));
            assert_eq!(label, "Shift");
        } else {
            assert_eq!(keybindings.get(action), before);
        }
    }
}

#[test]
fn settings_categories_have_a_stable_order_and_default() {
    assert_eq!(SettingsCategory::default(), SettingsCategory::General);
    assert_eq!(
        SettingsCategory::ALL.map(SettingsCategory::label),
        [
            "General",
            "Appearance",
            "Viewport",
            "Compiler",
            "Git",
            "Object Tree",
            "Keybindings"
        ]
    );
}

#[test]
fn settings_keybinding_groups_include_every_action_once() {
    let grouped = SETTINGS_KEYBINDING_GROUPS
        .iter()
        .flat_map(|(_, actions)| actions.iter().copied())
        .collect::<Vec<_>>();

    assert_eq!(grouped.len(), KeybindAction::ALL.len());
    for action in KeybindAction::ALL {
        assert_eq!(grouped.iter().filter(|candidate| **candidate == action).count(), 1);
    }
}

#[test]
fn every_settings_category_renders_in_the_two_pane_window() {
    let _context = CONTEXT_LOCK.lock().unwrap();
    for mut category in SettingsCategory::ALL {
        let mut context = dear_imgui_rs::Context::create();
        context
            .font_atlas()
            .try_claim_legacy_renderer()
            .expect("legacy renderer font atlas should be available")
            .build();
        context.io_mut().set_display_size([1280.0, 720.0]);
        context.io_mut().set_delta_time(1.0 / 60.0);
        let ui = context.frame();
        let window = WindowKey::new(format!("settings-test-{}", category.label()), "Settings")
            .expect("valid settings window key");
        let mut open = true;
        let mut capturing = None;
        let mut resetting_keybinds = false;
        let mut pending_profile = None;
        let mut ui_scale_draft = None;
        let mut session = Session::new();
        let mut settings = Settings::default();
        let mut themes = Themes::load(None, &ui.clone_style(), DEFAULT_THEME);

        draw_settings_window(
            ui,
            &window,
            SettingsWindowState {
                open: &mut open,
                category: &mut category,
                capturing: &mut capturing,
                resetting_keybinds: &mut resetting_keybinds,
                pending_profile: &mut pending_profile,
                ui_scale_draft: &mut ui_scale_draft,
                theme_editor: &mut ThemeEditor::default(),
                font_picker: &mut FontPicker::default(),
                profiles: &[],
            },
            &mut session,
            &mut settings,
            &mut themes,
            false,
        );

        assert!(context.render_legacy().valid());
    }
}

#[test]
fn reloading_the_current_profile_skips_the_confirmation() {
    let _context = CONTEXT_LOCK.lock().unwrap();
    let mut context = ui::testing::rectangle_context();
    let mut pending_profile = Some(ProfileReload::Current);

    let ui = context.frame();
    let request = draw_profile_reload_dialog(ui, &mut pending_profile);
    let is_open = ui.is_popup_open(PROFILE_RELOAD_POPUP);
    assert!(context.render_legacy().valid());

    assert_eq!(request, Some(ProfileReload::Current));
    assert_eq!(pending_profile, None);
    assert!(!is_open);
}

#[test]
fn compiler_settings_render_native_and_forced_profile_states() {
    let _context = CONTEXT_LOCK.lock().unwrap();
    for forced_profile in [None, Some(String::from("tgstation"))] {
        let mut context = dear_imgui_rs::Context::create();
        context
            .font_atlas()
            .try_claim_legacy_renderer()
            .expect("legacy renderer font atlas should be available")
            .build();
        context.io_mut().set_display_size([1280.0, 720.0]);
        context.io_mut().set_delta_time(1.0 / 60.0);
        let ui = context.frame();
        let window = WindowKey::new(format!("settings-profile-test-{forced_profile:?}"), "Settings")
            .expect("valid settings window key");
        let mut open = true;
        let mut category = SettingsCategory::Compiler;
        let mut capturing = None;
        let mut resetting_keybinds = false;
        let mut pending_profile = None;
        let mut ui_scale_draft = None;
        let mut session = Session::new();
        let mut environment = crate::environment::Environment::new("station.dme", objtree::ObjectTree::new());
        environment.bake_options.forced_profile = forced_profile;
        environment.profiles = Some(crate::environment::Profiles {
            available: vec![
                String::from("/datum/demir/tgstation"),
                String::from("/datum/demir/tgstation/debug"),
            ],
            default: String::from("/datum/demir/tgstation"),
            active: String::from("/datum/demir/tgstation/debug"),
        });
        session.state.environment = Some(std::sync::Arc::new(environment));
        let mut settings = Settings::default();
        let mut themes = Themes::load(None, &ui.clone_style(), DEFAULT_THEME);

        let output = draw_settings_window(
            ui,
            &window,
            SettingsWindowState {
                open: &mut open,
                category: &mut category,
                capturing: &mut capturing,
                resetting_keybinds: &mut resetting_keybinds,
                pending_profile: &mut pending_profile,
                ui_scale_draft: &mut ui_scale_draft,
                theme_editor: &mut ThemeEditor::default(),
                font_picker: &mut FontPicker::default(),
                profiles: &[String::from("tgstation"), String::from("mine")],
            },
            &mut session,
            &mut settings,
            &mut themes,
            false,
        );

        assert_eq!(output, SettingsWindowOutput::default());
        assert!(context.render_legacy().valid());
    }
}
