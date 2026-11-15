use base::path::TreePath;
use std::{error::Error, path::PathBuf};

use dear_imgui_rs::Key;
use dmm::{Coord, Map, MapFormat, Prefab, Size};
use net::LevelContents;
use ui::testing::{CONTEXT_LOCK, rectangle_context};

use super::{
    DeleteLevelDialog, GO_TO_POPUP, GoToDialog, NewLevelDialog, NewMapDialog, RESIZE_MAP_POPUP, ResizeMapDialog,
    TileFillPaths, TileFillSearch, draw_delete_level_dialog, draw_go_to_dialog, draw_new_level_dialog,
    draw_new_map_dialog, draw_resize_map_dialog, resolve_new_map_path,
};
use crate::{
    document::{MapDocument, patch},
    frontend::fixtures::click_dialog_button,
    session::Session,
    testing::TempDir,
};

#[test]
fn enter_creates_a_z_level_filled_with_the_default_turf_and_area() {
    let _guard = CONTEXT_LOCK.lock().unwrap();
    let mut context = ui::testing::rectangle_context();
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/env");
    let mut session = Session::new();
    session.load_environment(&root.join("test.dme")).unwrap();
    let document = session
        .state
        .open_document(MapDocument::new(dmm::Map::new(Size { x: 2, y: 1, z: 1 }), 1));
    let mut dialog = Some(NewLevelDialog::new(document));
    let mut remembered = None;

    let ui = context.frame();
    draw_new_level_dialog(ui, &mut session, &mut dialog, &mut remembered);
    assert!(context.render_legacy().valid());
    assert_eq!(session.level_count(), 1, "opening the dialog creates nothing");
    assert_eq!(
        dialog.as_ref().unwrap().fill.paths,
        TileFillPaths::defaults(&session),
        "the search starts at the defaults"
    );

    context.io_mut().add_key_event(dear_imgui_rs::Key::Enter, true);
    let ui = context.frame();
    draw_new_level_dialog(ui, &mut session, &mut dialog, &mut remembered);
    assert!(context.render_legacy().valid());

    assert!(dialog.is_none());
    assert_eq!(remembered, None, "unchanged defaults are not remembered");
    assert_eq!(session.level_count(), 2);
    assert_eq!(
        session.map().unwrap().tile_at(Coord::new(2, 1, 2)),
        session.default_fill().ok().as_ref()
    );
}

#[test]
fn a_changed_fill_is_remembered_for_the_next_z_level_and_resize() {
    let _guard = CONTEXT_LOCK.lock().unwrap();
    let mut context = ui::testing::rectangle_context();
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/env");
    let mut session = Session::new();
    session.load_environment(&root.join("test.dme")).unwrap();
    let document = session
        .state
        .open_document(MapDocument::new(dmm::Map::new(Size { x: 2, y: 1, z: 1 }), 1));
    let mut dialog = Some(NewLevelDialog::new(document));
    let mut remembered = None;

    let ui = context.frame();
    draw_new_level_dialog(ui, &mut session, &mut dialog, &mut remembered);
    assert!(context.render_legacy().valid());
    dialog.as_mut().unwrap().fill.paths.turf = String::from("/turf/open/floor");

    context.io_mut().add_key_event(dear_imgui_rs::Key::Enter, true);
    let ui = context.frame();
    draw_new_level_dialog(ui, &mut session, &mut dialog, &mut remembered);
    assert!(context.render_legacy().valid());

    let expected = TileFillPaths {
        turf: String::from("/turf/open/floor"),
        area: TileFillPaths::defaults(&session).area,
    };
    assert_eq!(remembered.as_ref(), Some(&expected));
    assert_eq!(
        session.map().unwrap().tile_at(Coord::new(1, 1, 2)).unwrap()[0].path,
        TreePath::parse("/turf/open/floor")
    );
    assert_eq!(
        ResizeMapDialog::new(
            Size { x: 2, y: 1, z: 2 },
            TileFillSearch::new(&session, remembered.as_ref())
        )
        .fill
        .paths,
        expected
    );
}

#[test]
fn enter_in_an_open_fill_search_does_not_create_a_z_level() {
    let _guard = CONTEXT_LOCK.lock().unwrap();
    let mut context = ui::testing::rectangle_context();
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/env");
    let mut session = Session::new();
    session.load_environment(&root.join("test.dme")).unwrap();
    let document = session
        .state
        .open_document(MapDocument::new(dmm::Map::new(Size { x: 2, y: 1, z: 1 }), 1));
    let mut dialog = Some(NewLevelDialog::new(document));
    let mut remembered = None;

    let ui = context.frame();
    draw_new_level_dialog(ui, &mut session, &mut dialog, &mut remembered);
    assert!(context.render_legacy().valid());
    dialog.as_mut().unwrap().fill.searching = true;

    context.io_mut().add_key_event(dear_imgui_rs::Key::Enter, true);
    let ui = context.frame();
    draw_new_level_dialog(ui, &mut session, &mut dialog, &mut remembered);
    assert!(context.render_legacy().valid());

    assert!(dialog.is_some());
    assert_eq!(session.level_count(), 1);
}

#[test]
fn an_invalid_fill_path_blocks_z_level_creation() {
    let _guard = CONTEXT_LOCK.lock().unwrap();
    let mut context = ui::testing::rectangle_context();
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/env");
    let mut session = Session::new();
    session.load_environment(&root.join("test.dme")).unwrap();
    let document = session
        .state
        .open_document(MapDocument::new(dmm::Map::new(Size { x: 2, y: 1, z: 1 }), 1));
    let mut dialog = Some(NewLevelDialog::new(document));
    let mut remembered = None;

    let ui = context.frame();
    draw_new_level_dialog(ui, &mut session, &mut dialog, &mut remembered);
    assert!(context.render_legacy().valid());
    dialog.as_mut().unwrap().fill.paths.area = String::from("/turf");

    context.io_mut().add_key_event(dear_imgui_rs::Key::Enter, true);
    let ui = context.frame();
    draw_new_level_dialog(ui, &mut session, &mut dialog, &mut remembered);
    assert!(context.render_legacy().valid());

    assert!(dialog.is_some());
    assert_eq!(remembered, None);
    assert_eq!(session.level_count(), 1);
}

#[test]
fn the_go_to_dialog_clamps_to_the_map_and_enter_returns_the_tile() {
    let _guard = CONTEXT_LOCK.lock().unwrap();
    let mut context = ui::testing::rectangle_context();
    let mut session = Session::new();
    session
        .state
        .open_document(MapDocument::new(dmm::Map::new(Size { x: 3, y: 2, z: 1 }), 1));
    let mut dialog = Some(GoToDialog::new(Coord::new(9, 1, 5)));

    let ui = context.frame();
    ui.open_popup(GO_TO_POPUP);
    assert_eq!(draw_go_to_dialog(ui, &session, &mut dialog), None);
    assert!(context.render_legacy().valid());
    assert_eq!(dialog, Some(GoToDialog { x: 3, y: 1, z: 1 }));

    context.io_mut().add_key_event(dear_imgui_rs::Key::Enter, true);
    let ui = context.frame();
    assert_eq!(draw_go_to_dialog(ui, &session, &mut dialog), Some(Coord::new(3, 1, 1)));
    assert!(context.render_legacy().valid());
    assert!(dialog.is_none());
}

#[test]
fn the_resize_dialog_warns_about_the_tiles_a_shrink_deletes() {
    let _guard = CONTEXT_LOCK.lock().unwrap();
    let mut context = ui::testing::rectangle_context();
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/env");
    let mut session = Session::new();
    session.load_environment(&root.join("test.dme")).unwrap();
    let mut map = dmm::Map::new(Size { x: 3, y: 1, z: 1 });
    let table = map.intern_tile(vec![Prefab::new(TreePath::parse("/obj/structure/table"))]);
    map.grid[0][0].fill(table);
    session.state.open_document(MapDocument::new(map, 1));
    let fill = TileFillSearch::new(&session, None);
    let mut dialog = Some(ResizeMapDialog::new(Size { x: 3, y: 1, z: 1 }, fill));
    let mut remembered = None;

    for width in [3, 1] {
        dialog.as_mut().unwrap().width = width;
        let ui = context.frame();
        ui.open_popup(RESIZE_MAP_POPUP);
        assert!(!draw_resize_map_dialog(ui, &mut session, &mut dialog, &mut remembered));
        assert!(context.render_legacy().valid());
    }

    let losses = dialog
        .unwrap()
        .losses
        .map(|(width, height, _, losses)| (width, height, losses));
    assert_eq!(losses, Some((1, 1, 2)));
    assert_eq!(
        session.map().unwrap().size().x,
        3,
        "nothing changes until Resize is pressed"
    );
}

#[test]
fn the_resize_dialog_accepts_dimensions_above_255() {
    let _guard = CONTEXT_LOCK.lock().unwrap();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/env");
    for (width, height) in [(256, 1), (1, 256)] {
        let mut context = rectangle_context();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).unwrap();
        session
            .state
            .open_document(MapDocument::new(Map::new(Size { x: 1, y: 1, z: 1 }), 1));
        let fill = session.default_fill().unwrap();
        let mut dialog = Some(ResizeMapDialog::new(
            session.map().unwrap().size(),
            TileFillSearch::new(&session, None),
        ));
        dialog.as_mut().unwrap().width = width;
        dialog.as_mut().unwrap().height = height;
        let mut remembered = None;
        let mut resized = false;

        click_dialog_button(
            &mut context,
            c"Resize map##resize-map",
            &["Cancel", "Resize Map"],
            |ui| resized |= draw_resize_map_dialog(ui, &mut session, &mut dialog, &mut remembered),
        );

        assert!(resized);
        assert!(dialog.is_none());
        assert_eq!(
            session.map().unwrap().size(),
            Size {
                x: width,
                y: height,
                z: 1,
            }
        );
        assert_eq!(
            session.map().unwrap().tile_at(Coord::new(width, height, 1)),
            Some(&fill)
        );
    }
}

#[test]
fn the_resize_dialog_preserves_dimensions_up_to_u32_max() {
    let _guard = CONTEXT_LOCK.lock().unwrap();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/env");
    let mut session = Session::new();
    session.load_environment(&root.join("test.dme")).unwrap();
    session
        .state
        .open_document(MapDocument::new(Map::new(Size { x: 1, y: 1, z: 1 }), 1));

    for dimension in [i32::MAX as u32 + 1, u32::MAX] {
        let mut context = rectangle_context();
        let mut dialog = Some(ResizeMapDialog::new(
            Size {
                x: dimension,
                y: dimension,
                z: 1,
            },
            TileFillSearch::new(&session, None),
        ));
        let mut remembered = None;

        let ui = context.frame();
        ui.open_popup(RESIZE_MAP_POPUP);
        assert!(!draw_resize_map_dialog(ui, &mut session, &mut dialog, &mut remembered));
        assert!(context.render_legacy().valid());

        let state = dialog.unwrap();
        assert_eq!((state.width, state.height), (dimension, dimension));
        assert_eq!(state.losses.unwrap().0, dimension);
        assert_eq!(session.map().unwrap().size(), Size { x: 1, y: 1, z: 1 });
    }
}

#[test]
fn new_map_defaults_are_255_by_255_with_one_level() {
    let dialog = NewMapDialog::default();

    assert!(dialog.path.is_empty());
    assert_eq!(dialog.format, MapFormat::Tgm);
    assert_eq!((dialog.width, dialog.height, dialog.levels), (255, 255, 1));
    assert!(dialog.error.is_none());
}

#[test]
fn the_new_map_dialog_accepts_dimensions_above_255() {
    let _guard = CONTEXT_LOCK.lock().unwrap();
    let directory = TempDir::new("large-map-dialog");
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/env");
    let mut session = Session::new();
    session.load_environment(&root.join("test.dme")).unwrap();
    let fill = session.default_fill().unwrap();

    for (width, height, levels) in [(256, 1, 1), (1, 256, 1), (1, 1, 256)] {
        let mut context = rectangle_context();
        let path = directory.path().join(format!("map-{width}-{height}-{levels}.dmm"));
        let mut dialog = Some(NewMapDialog {
            path: path.to_str().unwrap().to_owned(),
            width,
            height,
            levels,
            ..NewMapDialog::default()
        });
        let mut created = false;

        click_dialog_button(&mut context, c"New map##new-map", &["Cancel", "Create Map"], |ui| {
            created |= draw_new_map_dialog(ui, &mut session, &mut dialog).1;
        });

        assert!(created);
        assert!(dialog.is_none());
        let document = session.state.active_document().unwrap();
        assert_eq!(document.path.as_deref(), Some(path.as_path()));
        assert_eq!(
            document.map.size(),
            Size {
                x: width,
                y: height,
                z: levels,
            }
        );
        assert_eq!(document.map.tile_at(Coord::new(width, height, levels)), Some(&fill));
    }
}

#[test]
fn the_new_map_dialog_preserves_dimensions_up_to_u32_max() {
    let _guard = CONTEXT_LOCK.lock().unwrap();
    let mut session = Session::new();

    for dimension in [i32::MAX as u32 + 1, u32::MAX] {
        let mut context = rectangle_context();
        let mut dialog = Some(NewMapDialog {
            width: dimension,
            height: dimension,
            levels: dimension,
            ..NewMapDialog::default()
        });

        let ui = context.frame();
        ui.open_popup(super::NEW_MAP_POPUP);
        assert_eq!(draw_new_map_dialog(ui, &mut session, &mut dialog), (false, false));
        assert!(context.render_legacy().valid());

        let state = dialog.unwrap();
        assert_eq!(
            (state.width, state.height, state.levels),
            (dimension, dimension, dimension)
        );
        assert!(session.state.active_document().is_none());
    }
}

#[test]
fn new_map_paths_resolve_from_the_codebase_and_validate_the_target() {
    let root = std::env::temp_dir().join(format!("rmd-new-map-paths-{}", std::process::id()));
    let codebase = root.join("codebase");
    let maps = codebase.join("maps");
    std::fs::create_dir_all(&maps).unwrap();

    assert_eq!(
        resolve_new_map_path(&codebase, "maps/station").unwrap(),
        maps.join("station.dmm")
    );
    let outside = root.join("outside.dmm");
    assert_eq!(
        resolve_new_map_path(&codebase, &outside.display().to_string()).unwrap(),
        outside
    );
    assert!(
        resolve_new_map_path(&codebase, "station.txt")
            .unwrap_err()
            .contains(".dmm extension")
    );
    assert!(
        resolve_new_map_path(&codebase, "missing/station.dmm")
            .unwrap_err()
            .contains("does not exist")
    );

    let existing = maps.join("existing.dmm");
    std::fs::write(&existing, "existing").unwrap();
    assert!(
        resolve_new_map_path(&codebase, &existing.display().to_string())
            .unwrap_err()
            .contains("already exists")
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn confirming_deletion_removes_the_selected_level_and_can_be_undone() -> Result<(), Box<dyn Error>> {
    let _guard = CONTEXT_LOCK.lock().map_err(|_| "UI context lock is poisoned")?;
    let mut context = rectangle_context();
    let mut session = Session::new();
    let id = session
        .state
        .open_document(MapDocument::new(Map::new(Size { x: 2, y: 1, z: 3 }), 2));
    let mut dialog = Some(DeleteLevelDialog::new(
        session.state.document(id).ok_or("document is missing")?,
    ));
    draw_delete_level_dialog(context.frame(), &mut session, &mut dialog);
    assert!(context.render_legacy().valid());
    assert_eq!(session.level_count(), 3);
    context.io_mut().add_key_event(Key::Enter, true);
    draw_delete_level_dialog(context.frame(), &mut session, &mut dialog);
    assert!(context.render_legacy().valid());
    assert!(dialog.is_none());
    assert_eq!(session.level_count(), 2);
    assert_eq!(session.z(), 2);
    assert!(session.undo());
    assert_eq!(session.level_count(), 3);
    assert!(session.redo());
    assert_eq!(session.level_count(), 2);
    Ok(())
}

#[test]
fn cancellation_or_a_changed_document_does_not_delete_a_level() -> Result<(), Box<dyn Error>> {
    let _guard = CONTEXT_LOCK.lock().map_err(|_| "UI context lock is poisoned")?;
    for stale in [false, true] {
        let mut context = rectangle_context();
        let mut session = Session::new();
        let id = session
            .state
            .open_document(MapDocument::new(Map::new(Size { x: 1, y: 1, z: 2 }), 2));
        let mut dialog = Some(DeleteLevelDialog::new(
            session.state.document(id).ok_or("document is missing")?,
        ));
        draw_delete_level_dialog(context.frame(), &mut session, &mut dialog);
        assert!(context.render_legacy().valid());
        if stale {
            let document = session.state.document_mut(id).ok_or("document is missing")?;
            let cells = document.level_cells(&LevelContents::Fill(patch::encode_tile(&[])));
            assert!(document.insert_remote_level(3, cells));
        }

        context
            .io_mut()
            .add_key_event(if stale { Key::Enter } else { Key::Escape }, true);
        draw_delete_level_dialog(context.frame(), &mut session, &mut dialog);
        assert!(context.render_legacy().valid());
        assert_eq!(session.level_count(), if stale { 3 } else { 2 });
        assert_eq!(dialog.is_some(), stale);
        assert!(
            !session
                .state
                .document(id)
                .ok_or("document is missing")?
                .history
                .can_undo()
        );
    }

    Ok(())
}

#[test]
fn read_only_and_last_level_restrictions_apply_at_confirmation() -> Result<(), Box<dyn Error>> {
    let _guard = CONTEXT_LOCK.lock().map_err(|_| "UI context lock is poisoned")?;
    for levels in [1, 2] {
        let mut context = rectangle_context();
        let mut session = Session::new();
        let id = session
            .state
            .open_document(MapDocument::new(Map::new(Size { x: 1, y: 1, z: levels }), levels));
        let mut dialog = Some(DeleteLevelDialog::new(
            session.state.document(id).ok_or("document is missing")?,
        ));
        draw_delete_level_dialog(context.frame(), &mut session, &mut dialog);
        assert!(context.render_legacy().valid());
        if levels == 2 {
            session
                .state
                .document_mut(id)
                .ok_or("document is missing")?
                .set_read_only(true);
        }

        context.io_mut().add_key_event(Key::Enter, true);
        draw_delete_level_dialog(context.frame(), &mut session, &mut dialog);
        assert!(context.render_legacy().valid());
        assert!(dialog.is_some());
        assert!(!session.can_delete_level(id));
        assert!(session.delete_level(id, levels).is_err());
        assert_eq!(session.level_count(), levels);
    }

    Ok(())
}
