use std::error::Error;

use dear_imgui_rs::sys;
use dmm::writer;

use super::*;
use crate::session::fixtures::module_viewport_session;

type TestResult = Result<(), Box<dyn Error>>;

fn harness() -> Result<(impl Sized, RectangleUiHarness), Box<dyn Error>> {
    let (fixture, session) = module_viewport_session()?;
    let mut app = RectangleUiHarness::with_session(session);
    app.view.camera.camera.x = 64.0;
    app.view.camera.camera.y = 16.0;
    app.step();

    Ok((fixture, app))
}

fn buttons(app: &RectangleUiHarness) -> Result<[[f32; 2]; 3], Box<dyn Error>> {
    app.module_buttons.ok_or_else(|| "module controls are missing".into())
}

fn source(app: &RectangleUiHarness) -> Result<String, Box<dyn Error>> {
    Ok(writer::write(
        &app.session
            .state
            .active_document()
            .ok_or("active document is missing")?
            .to_map(),
    ))
}

fn selected(app: &RectangleUiHarness) -> Option<usize> { app.session.module_roots()[0].selected }

#[test]
fn clicking_a_marker_opens_the_popup() -> TestResult {
    let _guard = IMGUI_CONTEXT.lock().map_err(|error| error.to_string())?;
    let (_fixture, mut app) = harness()?;
    assert!(app.view.gestures.module_popup.is_none());

    app.click(app.tile(2, 1));

    assert!(app.view.gestures.module_popup.is_some());
    assert!(app.module_buttons.is_some());

    Ok(())
}

fn source_root(app: &RectangleUiHarness) -> Result<dmm::PrefabInstanceId, Box<dyn Error>> {
    app.session
        .module_roots()
        .iter()
        .find(|root| root.key.ancestry.is_empty())
        .map(|root| root.key.instance)
        .ok_or_else(|| "source root is missing".into())
}

#[test]
fn clicking_a_source_roots_marker_selects_it() -> TestResult {
    let _guard = IMGUI_CONTEXT.lock().map_err(|error| error.to_string())?;
    let (_fixture, mut app) = harness()?;
    assert_eq!(app.session.selected_instance(), None);

    app.click(app.tile(2, 1));

    assert_eq!(app.session.selected_instance(), Some(source_root(&app)?));
    assert!(app.view.gestures.module_popup.is_some());

    Ok(())
}

#[test]
fn clicking_a_nested_roots_marker_keeps_the_selection() -> TestResult {
    let _guard = IMGUI_CONTEXT.lock().map_err(|error| error.to_string())?;
    let (_fixture, mut app) = harness()?;

    app.click(app.tile(3, 1));

    assert_eq!(app.session.selected_instance(), None);
    assert_eq!(
        app.view
            .gestures
            .module_popup
            .as_ref()
            .ok_or("module popup is missing")?
            .0
            .ancestry
            .len(),
        1
    );

    Ok(())
}

#[test]
fn the_popup_follows_a_moved_root() -> TestResult {
    let _guard = IMGUI_CONTEXT.lock().map_err(|error| error.to_string())?;
    let (_fixture, mut app) = harness()?;
    app.click(app.tile(2, 1));
    let target = Coord::new(5, 1, 1);

    assert_eq!(
        app.session.move_selected_instance(target, "move", &[], None),
        Some(true)
    );
    app.step();

    let (key, coord) = app.view.gestures.module_popup.clone().ok_or("module popup closed")?;
    assert_eq!(coord, target);
    assert_eq!(key.instance, source_root(&app)?);
    assert!(app.module_buttons.is_some());

    Ok(())
}

#[test]
fn dragging_the_gizmo_of_a_selected_root_moves_it() -> TestResult {
    let _guard = IMGUI_CONTEXT.lock().map_err(|error| error.to_string())?;
    let (_fixture, mut app) = harness()?;
    app.click(app.tile(2, 1));
    app.key(Key::Escape, true);
    app.key(Key::Escape, false);
    let root = source_root(&app)?;
    assert_eq!(app.session.selected_instance(), Some(root));

    app.key(Key::ModCtrl, true);
    app.drag(app.tile(2, 1), app.tile(5, 1));
    app.key(Key::ModCtrl, false);

    assert_eq!(
        app.session.selected_location().map(|location| location.coord),
        Some(Coord::new(5, 1, 1))
    );
    assert_eq!(app.session.module_roots()[0].coord, Coord::new(5, 1, 1));

    Ok(())
}

#[test]
fn a_root_drag_leaves_the_bake_alone_until_release() -> TestResult {
    let _guard = IMGUI_CONTEXT.lock().map_err(|error| error.to_string())?;
    let (_fixture, mut app) = harness()?;
    app.click(app.tile(2, 1));
    app.key(Key::Escape, true);
    app.key(Key::Escape, false);
    let before = app.session.baked_preview_positions();
    assert!(!before.is_empty());

    app.key(Key::ModCtrl, true);
    app.pointer(app.tile(2, 1), false);
    app.pointer(app.tile(2, 1), true);
    app.pointer(app.tile(5, 1), true);

    assert!(app.session.is_module_bake_deferred());
    assert_eq!(app.session.module_roots()[0].coord, Coord::new(5, 1, 1));
    assert_eq!(app.session.baked_preview_positions(), before);

    app.pointer(app.tile(5, 1), false);
    app.key(Key::ModCtrl, false);

    assert!(!app.session.is_module_bake_deferred());
    let after = app.session.baked_preview_positions();
    assert_eq!(after.len(), before.len());
    assert_ne!(after, before);
    assert!(after.iter().all(|(_, position)| position.is_some()));

    Ok(())
}

#[test]
fn the_arrows_skip_variants_with_errors_and_leave_the_map_alone() -> TestResult {
    let _guard = IMGUI_CONTEXT.lock().map_err(|error| error.to_string())?;
    let (_fixture, mut app) = harness()?;
    let before = source(&app)?;
    app.click(app.tile(2, 1));
    assert_eq!(selected(&app), Some(1));
    assert_eq!(app.session.module_roots().len(), 2);

    app.click(buttons(&app)?[1]);
    assert_eq!(selected(&app), Some(1), "the first file is invalid");

    app.click(buttons(&app)?[2]);
    assert_eq!(selected(&app), Some(2));
    assert_eq!(
        app.session.module_roots().len(),
        1,
        "the nested root left with its variant"
    );

    app.click(buttons(&app)?[2]);
    assert_eq!(selected(&app), Some(2));

    app.click(buttons(&app)?[1]);
    assert_eq!(selected(&app), Some(1));
    assert_eq!(app.session.module_roots().len(), 2);
    assert!(app.view.gestures.module_popup.is_some());
    assert!(app.session.bake_view().is_none());
    assert!(!app.session.state.active_document().ok_or("no document")?.is_dirty());
    assert_eq!(source(&app)?, before);

    Ok(())
}

#[test]
fn the_combo_chooses_a_variant_without_editing() -> TestResult {
    let _guard = IMGUI_CONTEXT.lock().map_err(|error| error.to_string())?;
    let (_fixture, mut app) = harness()?;
    let before = source(&app)?;
    app.click(app.tile(2, 1));
    app.click(buttons(&app)?[2]);
    assert_eq!(selected(&app), Some(2));

    app.click(buttons(&app)?[0]);
    app.step();
    let option = unsafe {
        let context = &*sys::igGetCurrentContext();
        assert!(context.OpenPopupStack.Size > 0);
        let popup = &*(*context.OpenPopupStack.Data).Window;
        let style = &context.Style;
        [
            popup.WorkRect.Min.x + 20.0,
            popup.WorkRect.Min.y + context.FontSize * 1.5 + style.ItemSpacing.y,
        ]
    };
    app.click(option);

    assert_eq!(selected(&app), Some(1));
    assert!(app.view.gestures.module_popup.is_some());
    assert!(app.session.bake_view().is_none());
    assert!(!app.session.state.active_document().ok_or("no document")?.is_dirty());
    assert_eq!(source(&app)?, before);

    Ok(())
}

#[test]
fn escape_and_outside_clicks_close_the_popup() -> TestResult {
    let _guard = IMGUI_CONTEXT.lock().map_err(|error| error.to_string())?;
    let (_fixture, mut app) = harness()?;
    app.click(app.tile(2, 1));
    assert!(app.view.gestures.module_popup.is_some());

    app.key(Key::Escape, true);
    app.key(Key::Escape, false);
    assert!(app.view.gestures.module_popup.is_none());

    app.click(app.tile(2, 1));
    app.click(app.tile(3, 1));
    assert_eq!(
        app.view
            .gestures
            .module_popup
            .as_ref()
            .ok_or("module popup is missing")?
            .0
            .ancestry
            .len(),
        1,
        "the nested root takes the popup"
    );

    app.click([app.tile(3, 1)[0], app.tile(3, 1)[1] + 100.0]);
    assert!(app.view.gestures.module_popup.is_none());

    Ok(())
}

#[test]
fn the_popup_survives_switching_to_another_map_and_back() -> TestResult {
    let _guard = IMGUI_CONTEXT.lock().map_err(|error| error.to_string())?;
    let (_fixture, mut session) = module_viewport_session()?;
    let modular = session.state.active().ok_or("no document")?;
    let root = session.module_roots().first().ok_or("no module root")?;
    let popup = (root.key.clone(), root.coord);
    session.apply_map(crate::loader::LoadedMap {
        path: std::env::temp_dir().join("rmd-module-popup-other.dmm"),
        map: dmm::Map::new(Size { x: 4, y: 4, z: 1 }),
        z: 1,
        errors: vec![],
        repo: None,
        conflict: None,
    });
    let other = session.state.active().ok_or("no document")?;
    session.set_active_document(modular);

    let mut context = rectangle_context();
    let mut state = UiState::new(false)?;
    let mut settings = Settings::default();
    let mut frames = |state: &mut UiState, session: &mut Session, focus: DocumentId| {
        if let Some(view) = state.map_views.get_mut(&focus) {
            view.focus = true;
        }

        for _ in 0..3 {
            let ui = context.frame();
            for (index, id) in [modular, other].into_iter().enumerate() {
                let name = format!("###viewport-{}", id.get());
                ui.set_window_pos_by_name(&name, [index as f32 * 400.0, 0.0]);
                ui.set_window_size_by_name(&name, [400.0, 600.0]);
            }

            state.draw_map_views(ui, session, &mut settings, false);
            assert!(context.render_legacy().valid());
        }
    };
    frames(&mut state, &mut session, modular);
    frames(&mut state, &mut session, modular);
    assert_eq!(session.state.active(), Some(modular));
    state
        .map_views
        .get_mut(&modular)
        .ok_or("no modular view")?
        .gestures
        .module_popup = Some(popup.clone());

    frames(&mut state, &mut session, other);
    assert_eq!(session.state.active(), Some(other));
    frames(&mut state, &mut session, modular);
    assert_eq!(session.state.active(), Some(modular));
    assert_eq!(state.map_views[&modular].gestures.module_popup, Some(popup));

    Ok(())
}
