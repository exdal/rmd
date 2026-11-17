use base::{path::TreePath, types::Value};

use dmm::{Coord, Map, Prefab, Size};

use super::{Edit, EditGroupId};
use crate::document::{MapDocument, grid::TileView};

fn document() -> MapDocument {
    let mut map = Map::new(Size { x: 3, y: 1, z: 1 });
    let occupied = map.intern_tile(vec![
        Prefab::new(TreePath::parse("/turf/floor")),
        Prefab::new(TreePath::parse("/obj/table")),
    ]);
    let empty = map.intern_tile(Vec::new());
    map.grid[0][0] = vec![occupied, occupied, empty];

    MapDocument::new(map, 1)
}

fn rename_table(document: &MapDocument, label: &str, name: &str) -> Edit {
    let coord = Coord::new(1, 1, 1);
    let mut after = document.placed_tile(coord).unwrap();
    after[1]
        .prefab_mut()
        .set_var("name".into(), Value::Text(String::from(name)));
    let mut edit = Edit::new(label);
    edit.change(document, coord, after);

    edit
}

#[test]
fn retracting_an_edit_brings_back_the_redo_it_discarded() {
    let mut document = document();
    document.apply(rename_table(&document, "rename", "first"));
    document.history.mark_saved();
    assert!(document.undo());
    let group = EditGroupId::new();
    assert!(document.apply_grouped(rename_table(&document, "live", "second"), Some(group)));
    assert!(!document.history.can_redo());

    assert!(document.retract_with_affected(group).is_some());
    assert_eq!(document.redo_label(), Some("rename"));
    assert!(document.redo());
    assert!(!document.history.is_dirty(), "the save point comes back with the redo");
}

#[test]
fn retracting_an_edit_saved_in_the_meantime_discards_the_redo() {
    let mut document = document();
    document.apply(rename_table(&document, "rename", "first"));
    assert!(document.undo());
    let group = EditGroupId::new();
    assert!(document.apply_grouped(rename_table(&document, "live", "second"), Some(group)));
    document.history.mark_saved();

    assert!(document.retract_with_affected(group).is_some());
    assert_eq!(document.redo_label(), None);
    assert!(document.history.is_dirty(), "the file holds the retracted edit");
}

#[test]
fn changing_one_prefab_preserves_its_id_and_only_changes_its_cell() {
    let mut document = document();
    let source = Coord::new(1, 1, 1);
    let untouched = Coord::new(2, 1, 1);
    let id = document.instance_ids_at(source)[1];
    let untouched_before = document.map.tile_at(untouched).map(TileView::to_tile);
    let mut after = document.placed_tile(source).unwrap();
    after[1]
        .prefab_mut()
        .set_var("name".into(), Value::Text(String::from("moved table")));

    let mut edit = Edit::new("change table");
    edit.change(&document, source, after);
    document.apply(edit);

    assert_eq!(document.instance_ids_at(source)[1], id);
    assert_eq!(document.instance_location(id).unwrap().coord, source);
    assert_eq!(document.map.tile_at(untouched).map(TileView::to_tile), untouched_before);
}

#[test]
fn moving_a_prefab_transfers_its_id_and_undo_restores_it() {
    let mut document = document();
    let source = Coord::new(1, 1, 1);
    let destination = Coord::new(3, 1, 1);
    let mut source_after = document.placed_tile(source).unwrap();
    let moved = source_after.remove(1);
    let id = moved.id();
    let mut destination_after = document.placed_tile(destination).unwrap();
    destination_after.push(moved);

    let mut edit = Edit::new("move table");
    edit.change(&document, source, source_after);
    edit.change(&document, destination, destination_after);
    document.apply(edit);

    assert_eq!(document.instance_location(id).unwrap().coord, destination);
    assert!(document.undo());
    assert_eq!(document.instance_location(id).unwrap().coord, source);
    assert!(document.redo());
    assert_eq!(document.instance_location(id).unwrap().coord, destination);
}

#[test]
fn copies_get_new_ids_and_deleted_ids_return_on_undo() {
    let mut document = document();
    let source = Coord::new(1, 1, 1);
    let destination = Coord::new(3, 1, 1);
    let original = document.placed_tile(source).unwrap()[1].clone();
    let copy = document.instantiate(original.prefab().clone());
    let copied_id = copy.id();
    assert_ne!(copied_id, original.id());

    let mut edit = Edit::new("copy table");
    edit.change(&document, destination, vec![copy]);
    document.apply(edit);
    assert_eq!(document.instance_location(copied_id).unwrap().coord, destination);

    let mut delete = Edit::new("delete table");
    delete.change(&document, destination, Vec::new());
    document.apply(delete);
    assert_eq!(document.instance_location(copied_id), None);
    assert!(document.undo());
    assert_eq!(document.instance_location(copied_id).unwrap().coord, destination);
}

#[test]
fn undo_and_redo_move_through_the_saved_position() {
    let mut document = document();
    let coord = Coord::new(1, 1, 1);
    let mut edit = Edit::new("clear tile");
    edit.change(&document, coord, Vec::new());
    document.apply(edit);
    document.history.mark_saved();

    assert!(!document.is_dirty());
    assert_eq!(document.undo_label(), Some("clear tile"));
    assert!(document.undo());
    assert!(document.is_dirty());
    assert_eq!(document.redo_label(), Some("clear tile"));
    assert!(document.redo());
    assert!(!document.is_dirty());
}

#[test]
fn replacing_the_saved_redo_branch_stays_dirty() {
    let mut document = document();
    let mut saved = Edit::new("clear first tile");
    saved.change(&document, Coord::new(1, 1, 1), Vec::new());
    document.apply(saved);
    document.history.mark_saved();
    assert!(document.undo());

    let mut replacement = Edit::new("clear second tile");
    replacement.change(&document, Coord::new(2, 1, 1), Vec::new());
    document.apply(replacement);

    assert!(document.is_dirty());
    assert_eq!(document.undo_label(), Some("clear second tile"));
    assert_eq!(document.redo_label(), None);
}

#[test]
fn extending_a_saved_edit_group_stays_dirty() {
    let mut document = document();
    let coord = Coord::new(1, 1, 1);
    let group = EditGroupId::new();
    let mut first = document.placed_tile(coord).unwrap();
    first.pop();
    let mut edit = Edit::new("change tile");
    edit.change(&document, coord, first);
    document.apply_grouped(edit, Some(group));
    document.history.mark_saved();

    let mut edit = Edit::new("change tile");
    edit.change(&document, coord, Vec::new());
    document.apply_grouped(edit, Some(group));

    assert!(document.is_dirty());
    assert!(document.undo());
    assert!(!document.undo());
}

#[test]
fn an_empty_recorded_edit_is_undoable_and_tracked_by_group() {
    let mut document = document();
    let group = EditGroupId::new();

    document.apply_grouped(Edit::new("keep tile"), Some(EditGroupId::new()));
    assert!(!document.history.can_undo());

    document.apply_grouped(Edit::new("keep tile").recorded_when_empty(), Some(group));
    assert!(document.history.is_applied(group));
    assert_eq!(document.undo_label(), Some("keep tile"));

    assert!(document.undo());
    assert!(!document.history.is_applied(group));
    assert!(document.redo());
    assert!(document.history.is_applied(group));
}

#[test]
fn the_last_change_to_a_tile_wins_and_undo_restores_the_first_before() {
    let mut document = document();
    let coord = Coord::new(1, 1, 1);
    let before = document.placed_tile(coord).unwrap();
    let table = document.instantiate(Prefab::new(TreePath::parse("/obj/table")));
    let id = table.id();

    let mut edit = Edit::new("twice");
    edit.change(&document, coord, vec![table]);
    edit.change(&document, coord, Vec::new());
    assert!(document.apply(edit));

    assert_eq!(document.placed_tile(coord).unwrap(), Vec::new());
    assert_eq!(document.instance_location(id), None);

    assert!(document.undo());
    assert_eq!(document.placed_tile(coord).unwrap(), before);

    assert!(document.redo());
    assert_eq!(document.placed_tile(coord).unwrap(), Vec::new());
}
