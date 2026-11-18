use base::{coord::Coord, path::TreePath};
use std::path::Path;

use dmm::{Map, Size};
use prefab::Prefab;

use super::StashMode;
use crate::{
    document::{DocumentId, MapDocument, selection::Selection},
    session::{
        Session,
        fixtures::{examples, settle_bake},
    },
    tool::{RegionSelectionMode, SelectionRotation, Tool},
};

const TABLES: [(u32, u32); 2] = [(1, 1), (2, 2)];

fn session_with_map() -> (Session, DocumentId) {
    let mut session = Session::new();
    session.load_environment(&examples().join("test.dme")).unwrap();

    let mut map = Map::new(Size { x: 6, y: 6, z: 1 });
    let plain = map.intern_tile(vec![
        Prefab::new(TreePath::parse("/turf/open/floor")),
        Prefab::new(TreePath::parse("/area/station")),
    ]);
    let table = map.intern_tile(vec![
        Prefab::new(TreePath::parse("/turf/open/floor")),
        Prefab::new(TreePath::parse("/obj/structure/table")),
        Prefab::new(TreePath::parse("/area/station")),
    ]);
    for row in &mut map.grid[0] {
        row.fill(plain);
    }
    for (x, y) in TABLES {
        map.grid[0][(6 - y) as usize][(x - 1) as usize] = table;
    }

    let id = session.activate_document(MapDocument::new(map, 1));
    session.set_tool(Tool::Select);

    (session, id)
}

fn paths(session: &Session, id: DocumentId, x: u32, y: u32) -> Vec<String> {
    session
        .state
        .document(id)
        .and_then(|document| document.map.tile_at(Coord::new(x, y, 1)))
        .map(|tile| tile.iter().map(|prefab| prefab.path.to_string()).collect())
        .unwrap_or_default()
}

fn has_table(session: &Session, id: DocumentId, x: u32, y: u32) -> bool {
    paths(session, id, x, y)
        .iter()
        .any(|path| path == "/obj/structure/table")
}

fn select(session: &mut Session, from: (u32, u32), to: (u32, u32)) {
    let selection = Selection::from_drag(Coord::new(from.0, from.1, 1), Coord::new(to.0, to.1, 1));
    assert!(session.select_region(Some(selection)));
}

fn stash_two(session: &mut Session) -> [DocumentId; 2] {
    select(session, (1, 1), (2, 2));
    let first = session.stash_selection(StashMode::Copy).unwrap();
    select(session, (3, 1), (5, 1));
    let second = session.stash_selection(StashMode::Copy).unwrap();

    [first, second]
}

#[test]
fn copying_to_the_stash_leaves_the_map_and_clipboard_alone() {
    let (mut session, id) = session_with_map();
    select(&mut session, (1, 1), (2, 2));

    assert!(session.can_stash_selection(StashMode::Copy));
    let entry = session.stash_selection(StashMode::Copy).unwrap();

    assert_eq!(session.stash_ids(), [entry]);
    assert!(session.clipboard().is_none());
    assert!(!session.state.document(id).unwrap().is_dirty());
    assert!(has_table(&session, id, 1, 1));
    assert!(has_table(&session, entry, 1, 1));
    assert!(has_table(&session, entry, 2, 2));
    assert!(!has_table(&session, entry, 2, 1));
    assert!(session.caches.contains_key(&entry));
}

#[test]
fn moving_to_the_stash_clears_the_map_in_one_undo_step() {
    let (mut session, id) = session_with_map();
    select(&mut session, (1, 1), (2, 2));

    let entry = session.stash_selection(StashMode::Move).unwrap();

    assert!(session.clipboard().is_none());
    assert!(!has_table(&session, id, 1, 1));
    assert!(!has_table(&session, id, 2, 2));
    assert!(has_table(&session, entry, 1, 1));
    assert_eq!(session.undo_label(), Some("move to stash"));

    assert!(session.undo());
    assert!(has_table(&session, id, 1, 1));
    assert!(has_table(&session, id, 2, 2));
    assert_eq!(session.stash_len(), 1);
    assert!(has_table(&session, entry, 2, 2));
}

#[test]
fn a_hollow_selection_cannot_be_stashed() {
    let (mut session, _) = session_with_map();
    let selection = Selection::from_drag(Coord::new(1, 1, 1), Coord::new(5, 5, 1));
    assert!(session.select_region_with_mode(Some(selection), RegionSelectionMode::Hollow { line_width: 1 }));

    for mode in [StashMode::Copy, StashMode::Move] {
        assert!(!session.can_stash_selection(mode));
        assert!(session.stash_selection(mode).is_none());
    }
    assert_eq!(session.stash_len(), 0);
}

#[test]
fn a_region_inside_a_stash_entry_copies_and_pastes_into_a_map() {
    let (mut session, id) = session_with_map();
    select(&mut session, (1, 1), (2, 2));
    let entry = session.stash_selection(StashMode::Copy).unwrap();
    assert!(session.set_active_document(entry));
    assert!(!session.can_stash_selection(StashMode::Copy));

    select(&mut session, (2, 1), (2, 2));
    assert!(session.copy_selection(RegionSelectionMode::Full));
    assert!(session.set_active_document(id));

    let target = Coord::new(4, 4, 1);
    assert!(session.can_paste_clipboard(target, SelectionRotation::Original));
    assert!(session.paste_clipboard(target, SelectionRotation::Original));
    assert!(!has_table(&session, id, 4, 4));
    assert!(has_table(&session, id, 4, 5));
}

#[test]
fn a_copy_from_the_stash_keeps_the_types_hidden_when_it_was_stashed() {
    let (mut session, id) = session_with_map();
    assert!(!session.options.show_areas);
    select(&mut session, (1, 1), (2, 2));
    let entry = session.stash_selection(StashMode::Copy).unwrap();

    session.options.show_areas = true;
    assert!(session.set_active_document(entry));
    select(&mut session, (2, 1), (2, 2));
    assert!(session.copy_selection(RegionSelectionMode::Full));
    assert!(session.set_active_document(id));

    let target = Coord::new(4, 4, 1);
    assert!(session.paste_clipboard(target, SelectionRotation::Original));
    assert!(paths(&session, id, 4, 5).contains(&String::from("/area/station")));
    assert!(paths(&session, id, 4, 4).contains(&String::from("/area/station")));
}

#[test]
fn removing_an_entry_bakes_nothing() {
    let (mut session, _) = session_with_map();
    let [first, second] = stash_two(&mut session);
    settle_bake(&mut session);
    let generation = session.state.document(second).unwrap().generation();
    let sprites = session.caches[&second].instances.sprites.len();

    assert!(session.remove_stash_entry(first));

    assert!(!session.baker.is_busy());
    assert!(session.queued_bakes.is_empty());
    assert!(!session.caches.contains_key(&first));
    assert_eq!(session.state.document(second).unwrap().generation(), generation);
    assert_eq!(session.caches[&second].instances.sprites.len(), sprites);
}

#[test]
fn removing_the_active_entry_hands_back_a_map() {
    let (mut session, id) = session_with_map();
    let [first, second] = stash_two(&mut session);
    assert!(session.set_active_document(first));

    assert!(session.remove_stash_entry(first));
    assert!(!session.remove_stash_entry(first));
    assert_eq!(session.state.active(), Some(id));
    assert_eq!(session.stash_ids(), [second]);

    session.clear_stash();
    assert_eq!(session.stash_len(), 0);
    assert!(!session.caches.contains_key(&second));
}

#[test]
fn a_codebase_reload_keeps_and_rebuilds_the_stash() {
    let (mut session, _) = session_with_map();
    let entries = stash_two(&mut session);
    let sprites = entries.map(|entry| session.caches[&entry].instances.sprites.len());
    assert!(sprites.iter().all(|count| *count > 0));

    session.load_environment(&examples().join("test.dme")).unwrap();

    assert_eq!(session.stash_ids(), entries);
    for (entry, count) in entries.into_iter().zip(sprites) {
        assert_eq!(session.caches[&entry].instances.sprites.len(), count);
        assert!(session.caches[&entry].modules.is_blocked);
    }
}

#[test]
fn an_active_stash_entry_refuses_closing_saving_and_resizing() {
    let (mut session, _) = session_with_map();
    let [entry, _] = stash_two(&mut session);
    assert!(session.set_active_document(entry));

    assert!(!session.close_map(entry));
    assert!(!session.can_save_map_in_place());
    assert!(session.save_map().is_err());
    assert!(session.save_map_as(Path::new("stash.dmm"), Default::default()).is_err());
    assert!(session.resize_map(9, 9, &[]).is_err());
    assert!(session.create_level(entry, &[]).is_err());
    assert!(!session.can_delete_level(entry));
    assert!(!session.can_change_level(1));
    assert!(!session.can_share_coop_map());
    assert_eq!(
        session.state.document(entry).unwrap().map.size(),
        Size { x: 2, y: 2, z: 1 }
    );
    assert_eq!(session.stash_len(), 2);
}
