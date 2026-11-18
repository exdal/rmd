use base::{path::TreePath, types::Value};
use std::{collections::HashSet, env, error::Error, fs, process, ptr};

use dmm::{Map, MapFormat, Size, parser, writer::MapWriter};
use net::{LevelChange, LevelContents};
use objtree::ObjectTree;
use prefab::Prefab;

use super::{Coord, MapDocument, instance::VarMutation, sanitize_vars, selection::Selection};
use crate::{
    document::{
        command::{Edit, EditGroupId},
        grid::TileView,
        patch,
    },
    focus::AreaFocus,
    tool::RegionSelectionMode,
};

#[test]
fn undo_and_redo_report_the_atoms_they_replace_even_after_a_remote_edit() {
    let coord = Coord::new(1, 1, 1);
    let floor = Prefab::new(TreePath::parse("/turf/floor"));
    let wall = Prefab::new(TreePath::parse("/turf/wall"));
    let mut document = MapDocument::new(Map::new(Size { x: 1, y: 1, z: 1 }), 1);
    let placed = vec![document.instantiate(floor.clone())];
    let mut edit = Edit::new("floor");
    edit.change(&document, coord, placed);
    assert!(document.apply(edit));

    let placed = vec![document.instantiate(wall.clone())];
    let mut edit = Edit::new("wall");
    edit.change(&document, coord, placed);
    assert!(document.apply(edit));

    let marker = Prefab::new(TreePath::parse("/obj/remote"));
    document.apply_remote(vec![(coord, vec![wall.clone(), marker.clone()])]);
    let remote = document.instance_ids_at(coord).to_vec();

    let affected = document.undo_with_affected().unwrap();
    for id in &remote {
        assert!(affected.contains(id), "undo left {id:?} behind for the baker");
    }

    let restored = document.instance_ids_at(coord).to_vec();
    document.apply_remote(vec![(coord, vec![floor, marker])]);
    let remote = document.instance_ids_at(coord).to_vec();
    assert_ne!(remote, restored);

    let affected = document.redo_with_affected().unwrap();
    for id in &remote {
        assert!(affected.contains(id), "redo left {id:?} behind for the baker");
    }
}

#[test]
fn a_remote_edit_keeps_the_ids_of_unchanged_prefabs_and_reports_only_the_rest() {
    let mut document = MapDocument::new(shared_tile_map(), 1);
    let (from, to) = (Coord::new(1, 1, 1), Coord::new(2, 1, 1));
    let floor = Prefab::new(TreePath::parse("/turf/floor"));
    let table = Prefab::new(TreePath::parse("/obj/table"));
    let chair_prefab = Prefab::new(TreePath::parse("/obj/chair"));
    let [from_floor, from_table] = document.instance_ids_at(from).try_into().unwrap();
    let [to_floor, to_table] = document.instance_ids_at(to).try_into().unwrap();

    let affected = document.apply_remote(vec![
        (from, vec![floor.clone()]),
        (to, vec![floor.clone(), table.clone(), chair_prefab.clone()]),
    ]);

    assert_eq!(document.instance_ids_at(from), [from_floor]);
    assert_eq!(&document.instance_ids_at(to)[..2], [to_floor, to_table]);
    let chair = document.instance_ids_at(to)[2];
    assert_eq!(affected, [from_table, chair]);

    assert!(document.apply_remote(vec![(from, vec![floor.clone()])]).is_empty());
    assert_eq!(document.instance_ids_at(from), [from_floor]);

    let affected = document.apply_remote(vec![(to, vec![table, floor.clone(), chair_prefab])]);
    assert_eq!(document.instance_ids_at(to), [to_table, to_floor, chair]);
    assert_eq!(
        affected,
        [to_floor, to_table, chair],
        "a reorder repaints the whole tile"
    );
}

#[test]
fn sanitizing_drops_constant_defaults_and_merges_tiles_that_become_equal() {
    let mut tree = objtree::ObjectTree::new();
    let table = tree.register(&TreePath::parse("/obj/table"), base::location::Location::default());
    for (name, value) in [
        ("name", Value::Text(String::from("table"))),
        ("desc", Value::Unevaluated),
    ] {
        tree.get_mut(table).unwrap().vars.insert(
            name.into(),
            objtree::VarDecl {
                name: name.into(),
                declared_type: None,
                modifiers: Default::default(),
                value,
                declared: true,
                location: Default::default(),
                resolved_type: None,
                shared: None,
            },
        );
    }
    let with = |vars: &[(&str, Value)]| {
        let mut prefab = Prefab::new(TreePath::parse("/obj/table"));
        for (name, value) in vars {
            prefab.set_var((*name).into(), value.clone());
        }
        vec![prefab]
    };
    let mut map = Map::new(Size { x: 4, y: 1, z: 1 });
    let plain = map.intern_tile(with(&[]));
    let named = map.intern_tile(with(&[("name", Value::Text(String::from("table")))]));
    let renamed = map.intern_tile(with(&[("name", Value::Text(String::from("desk")))]));
    let described = map.intern_tile(with(&[("desc", Value::Unevaluated)]));
    map.grid[0][0] = vec![plain, named, renamed, described];

    assert_eq!(sanitize_vars(&mut map, &tree), 1);
    assert_eq!(map.grid[0][0], [plain, plain, renamed, described]);
    assert!(!map.dictionary.contains_key(&named));
    assert_eq!(
        map.dictionary[&renamed],
        with(&[("name", Value::Text(String::from("desk")))])
    );
    assert_eq!(
        map.dictionary[&described],
        with(&[("desc", Value::Unevaluated)]),
        "a runtime default is kept"
    );

    let mut map = Map::new(Size { x: 2, y: 1, z: 1 });
    let first = map.intern_tile(with(&[]));
    let copy = dmm::key::Key(first.0 + 1);
    map.dictionary.insert(copy, with(&[]));
    map.grid[0][0] = vec![first, copy];
    assert_eq!(sanitize_vars(&mut map, &tree), 0);
    assert_eq!(
        map.grid[0][0],
        [first, first],
        "existing duplicates merge even with nothing to drop"
    );
}

#[test]
fn sanitizing_compares_text_after_decoding_its_escapes() {
    let mut tree = objtree::ObjectTree::new();
    let table = tree.register(&TreePath::parse("/obj/table"), base::location::Location::default());
    for (name, value) in [
        ("name", Value::Text(String::from("\u{FFFD}\u{16}Table"))),
        ("desc", Value::Text(String::from("a \"quoted\" desk"))),
    ] {
        tree.get_mut(table).unwrap().vars.insert(
            name.into(),
            objtree::VarDecl {
                name: name.into(),
                declared_type: None,
                modifiers: Default::default(),
                value,
                declared: true,
                location: Default::default(),
                resolved_type: None,
                shared: None,
            },
        );
    }
    let with = |vars: &[(&str, &str)]| {
        let mut prefab = Prefab::new(TreePath::parse("/obj/table"));
        for (name, value) in vars {
            prefab.set_var((*name).into(), Value::Text((*value).to_string()));
        }
        vec![prefab]
    };
    let mut map = Map::new(Size { x: 3, y: 1, z: 1 });
    let plain = map.intern_tile(with(&[]));
    let same = map.intern_tile(with(&[("name", r"\improper Table"), ("desc", r#"a \"quoted\" desk"#)]));
    let different = map.intern_tile(with(&[("name", r"\improper Chair")]));
    map.grid[0][0] = vec![plain, same, different];

    assert_eq!(sanitize_vars(&mut map, &tree), 2);
    assert_eq!(map.grid[0][0], [plain, plain, different]);
    assert_eq!(map.dictionary[&different], with(&[("name", r"\improper Chair")]));
}

fn shared_tile_map() -> Map {
    let mut map = Map::new(Size { x: 2, y: 1, z: 2 });
    let key = map.intern_tile(vec![
        Prefab::new(TreePath::parse("/turf/floor")),
        Prefab::new(TreePath::parse("/obj/table")),
    ]);

    for level in &mut map.grid {
        level[0].fill(key);
    }

    map
}

#[test]
fn shared_dictionary_prefabs_receive_unique_placement_ids() -> Result<(), &'static str> {
    let document = MapDocument::new(shared_tile_map(), 1);
    let mut ids = Vec::new();

    for z in 1..=2 {
        for x in 1..=2 {
            ids.extend_from_slice(document.instance_ids_at(Coord::new(x, 1, z)));
        }
    }

    assert_eq!(ids.len(), 8);
    assert_eq!(ids.iter().copied().collect::<HashSet<_>>().len(), ids.len());
    let first = document.prefab_instance(ids[0]).ok_or("first instance is missing")?;
    let next_tile = document
        .prefab_instance(ids[2])
        .ok_or("next tile instance is missing")?;
    assert_ne!(first.id, next_tile.id);
    assert_ne!(first.location.coord, next_tile.location.coord);
    assert!(
        ptr::eq(first.prefab(), next_tile.prefab()),
        "equal tiles share prefab storage"
    );
    for instance in document.prefab_instances() {
        assert_eq!(document.prefab_instance(instance.id), Some(instance));
        assert_eq!(document.instance_location(instance.id), Some(instance.location));
    }

    Ok(())
}

#[test]
fn identical_instances_are_found_on_every_level_and_edited_as_one_undo() {
    let mut document = MapDocument::new(shared_tile_map(), 1);
    let table = Prefab::new(TreePath::parse("/obj/table"));
    let tables = [(1, 1), (2, 1), (1, 2), (2, 2)]
        .map(|(x, z)| document.instance_ids_at(Coord::new(x, 1, z))[1])
        .into_iter()
        .collect::<HashSet<_>>();

    let found = document.identical_instances(&table);
    assert_eq!(found.iter().copied().collect::<HashSet<_>>(), tables);
    assert_eq!(document.identical_instances(&table).len(), 4);

    let name = VarMutation::Set("name".into(), Value::Text(String::from("Oak")));
    assert_eq!(
        document.edit_instances(&found, "set name", None, &[name], None),
        Some(true)
    );
    let mut oak = table.clone();
    oak.set_var("name".into(), Value::Text(String::from("Oak")));
    assert!(document.identical_instances(&table).is_empty());
    assert_eq!(
        document.identical_instances(&oak).into_iter().collect::<HashSet<_>>(),
        tables
    );

    assert!(document.undo());
    assert_eq!(document.identical_instances(&table).len(), 4);
    assert!(!document.undo(), "the whole edit was one step");
}

#[test]
fn identical_instances_include_repeats_within_one_tile() {
    let table = Prefab::new(TreePath::parse("/obj/table"));
    let mut map = Map::new(Size { x: 1, y: 2, z: 1 });
    let doubled = map.intern_tile(vec![table.clone(), table.clone()]);
    let floor = map.intern_tile(vec![Prefab::new(TreePath::parse("/turf/floor"))]);
    map.grid[0][0][0] = doubled;
    map.grid[0][1][0] = floor;
    let document = MapDocument::new(map, 1);

    assert_eq!(
        document.identical_instances(&table),
        document.instance_ids_at(Coord::new(1, 2, 1))
    );
}

fn resize_fill() -> Vec<Prefab> {
    vec![
        Prefab::new(TreePath::parse("/turf/space")),
        Prefab::new(TreePath::parse("/area/space")),
    ]
}

#[test]
fn growing_the_map_fills_new_tiles_and_undo_restores_the_old_size() {
    let mut document = MapDocument::new(shared_tile_map(), 1);
    let grid = document.map.clone();
    let corner = document.instance_ids_at(Coord::new(1, 1, 2)).to_vec();

    assert!(document.resize(3, 2, &resize_fill()));
    assert_eq!(
        (document.map.size().x, document.map.size().y, document.map.size().z),
        (3, 2, 2)
    );
    assert_eq!(document.instance_ids_at(Coord::new(1, 1, 2)), corner);
    for coord in [Coord::new(3, 1, 1), Coord::new(1, 2, 1), Coord::new(3, 2, 2)] {
        assert_eq!(
            document.map.tile_at(coord).map(TileView::to_tile),
            Some(resize_fill()),
            "{coord:?}"
        );
        assert_eq!(document.instance_ids_at(coord).len(), 2);
    }
    assert_eq!(document.undo_label(), Some("resize map to 3x2"));

    assert!(document.undo());
    assert_eq!((document.map.size().x, document.map.size().y), (2, 1));
    assert_eq!(document.map, grid);
    assert!(document.instance_ids_at(Coord::new(3, 1, 1)).is_empty());
    assert!(!document.to_map().dictionary.values().any(|tile| *tile == resize_fill()));

    assert!(document.redo());
    assert_eq!(
        document.map.tile_at(Coord::new(3, 2, 1)).map(TileView::to_tile),
        Some(resize_fill())
    );
}

#[test]
fn shrinking_the_map_drops_tiles_that_undo_brings_back_with_their_ids() {
    let mut document = MapDocument::new(shared_tile_map(), 1);
    let removed = Coord::new(2, 1, 2);
    let removed_ids = document.instance_ids_at(removed).to_vec();
    let table = document.instance_ids_at(Coord::new(1, 1, 1))[1];
    document.pick_instance(Some(document.instance_ids_at(removed)[1]));

    assert!(document.resize(1, 1, &resize_fill()));
    assert_eq!(document.map.size().x, 1);
    assert_eq!(document.map.tile_at(removed), None);
    assert!(removed_ids.iter().all(|id| document.instance_location(*id).is_none()));
    assert_eq!(document.picked_instance(), None);
    assert!(document.instance_location(table).is_some());

    assert!(document.undo());
    assert_eq!(document.instance_ids_at(removed), removed_ids);
    assert!(!document.resize(2, 1, &resize_fill()), "the size is unchanged");
}

#[test]
fn a_resized_map_saves_at_its_new_size() {
    let mut document = MapDocument::new(shared_tile_map(), 1);
    assert!(document.resize(4, 3, &resize_fill()));

    let written = MapWriter::new(&document.to_map()).write();
    let (parsed, errors) = dmm::parser::parse(&written);
    assert!(errors.is_empty(), "{errors:?}");

    assert_eq!((parsed.size.x, parsed.size.y, parsed.size.z), (4, 3, 2));
    assert_eq!(
        parsed.tile_at(Coord::new(1, 1, 1)).map(TileView::from),
        document.map.tile_at(Coord::new(1, 1, 1))
    );
    assert_eq!(parsed.tile_at(Coord::new(4, 3, 2)), Some(&resize_fill()));
}

#[test]
fn placement_ids_are_not_serialized() {
    let map = shared_tile_map();
    let before = MapWriter::new(&map).write();
    let document = MapDocument::new(map, 1);

    assert_eq!(MapWriter::new(&document.to_map()).write(), before);
}

#[test]
fn deleted_instance_is_removed_from_selection() {
    let mut document = MapDocument::new(shared_tile_map(), 1);
    let coord = Coord::new(1, 1, 1);
    let picked = document.instance_ids_at(coord)[1];
    document.pick_instance(Some(picked));
    assert_eq!(document.picked_instance(), Some(picked));

    let mut after = document.placed_tile(coord).unwrap();
    after.remove(1);
    let mut edit = Edit::new("delete selected prefab");
    edit.change(&document, coord, after);
    document.apply(edit);

    assert_eq!(document.picked_instance(), None);
}

#[test]
fn setting_an_instance_var_changes_only_that_placement_and_is_undoable() {
    let mut document = MapDocument::new(shared_tile_map(), 1);
    let changed = Coord::new(1, 1, 1);
    let untouched = Coord::new(2, 1, 1);
    let id = document.instance_ids_at(changed)[1];
    let untouched_id = document.instance_ids_at(untouched)[1];
    document.pick_instance(Some(id));

    assert_eq!(
        document.set_instance_var(id, "name".into(), base::types::Value::Text("selected".into())),
        Some(true),
    );
    assert_eq!(document.picked_instance(), Some(id));
    assert_eq!(document.instance_ids_at(changed)[1], id);
    assert_eq!(document.instance_ids_at(untouched)[1], untouched_id);
    assert_eq!(
        document
            .prefab_instance(id)
            .and_then(|instance| instance.prefab().var(&"name".into())),
        Some(&base::types::Value::Text("selected".into())),
    );
    assert_eq!(
        document
            .prefab_instance(untouched_id)
            .and_then(|instance| instance.prefab().var(&"name".into())),
        None,
    );
    assert!(document.is_dirty());
    assert!(
        MapWriter::new(&document.to_map())
            .write()
            .contains("/obj/table{name = \"selected\"}")
    );

    assert_eq!(
        document.set_instance_var(id, "name".into(), base::types::Value::Text("selected".into())),
        Some(false),
    );

    assert!(document.undo());
    assert_eq!(
        document
            .prefab_instance(id)
            .and_then(|instance| instance.prefab().var(&"name".into())),
        None
    );
    assert_eq!(document.picked_instance(), Some(id));

    assert!(document.redo());
    assert_eq!(
        document
            .prefab_instance(id)
            .and_then(|instance| instance.prefab().var(&"name".into())),
        Some(&base::types::Value::Text("selected".into())),
    );
}

#[test]
fn a_read_only_document_refuses_local_edits_but_takes_remote_ones() {
    let mut document = MapDocument::new(shared_tile_map(), 1);
    let coord = Coord::new(1, 1, 1);
    let id = document.instance_ids_at(coord)[1];
    let top = |document: &MapDocument| {
        document
            .prefab_instance(document.instance_ids_at(coord)[0])
            .map(|instance| instance.prefab().path.to_string())
    };
    assert_eq!(
        document.set_instance_var(id, "name".into(), base::types::Value::Text("before".into())),
        Some(true),
    );

    document.set_read_only(true);
    assert!(!document.allows_edit_at(coord));
    assert_eq!(
        document.set_instance_var(id, "name".into(), base::types::Value::Text("after".into())),
        Some(false),
    );
    assert!(!document.resize(3, 1, &[]));
    assert!(document.append_level(&[]).is_none());
    assert!(!document.undo());

    document.apply_remote(vec![(coord, vec![Prefab::new(TreePath::parse("/turf/wall"))])]);
    assert_eq!(top(&document).as_deref(), Some("/turf/wall"));

    document.replace_map(shared_tile_map(), false);
    assert!(!document.is_read_only());
}

#[test]
fn a_live_variable_edit_group_is_one_undo_step() {
    let mut document = MapDocument::new(shared_tile_map(), 1);
    let id = document.instance_ids_at(Coord::new(1, 1, 1))[1];
    let group = EditGroupId::new();

    for value in 1..=1_000 {
        assert_eq!(
            document.edit_instance_vars(
                id,
                "change pixel offset",
                &[VarMutation::Set(
                    "pixel_x".into(),
                    base::types::Value::Num(value as f32),
                )],
                Some(group),
            ),
            Some(true),
        );
    }

    // Intermediate drag values must not accumulate unreachable map keys.
    assert_eq!(document.to_map().dictionary.len(), 2);

    assert_eq!(
        document
            .prefab_instance(id)
            .and_then(|instance| instance.prefab().var(&"pixel_x".into())),
        Some(&base::types::Value::Num(1_000.0)),
    );
    assert!(document.undo());
    assert_eq!(
        document
            .prefab_instance(id)
            .and_then(|instance| instance.prefab().var(&"pixel_x".into())),
        None,
    );
    assert!(!document.undo());
    assert!(document.redo());
    assert_eq!(
        document
            .prefab_instance(id)
            .and_then(|instance| instance.prefab().var(&"pixel_x".into())),
        Some(&base::types::Value::Num(1_000.0)),
    );
}

#[test]
fn moving_an_instance_applies_mutations_and_undo_restores_both() -> Result<(), &'static str> {
    let mut document = MapDocument::new(shared_tile_map(), 1);
    let source = Coord::new(1, 1, 1);
    let destination = Coord::new(2, 1, 1);
    let id = document.instance_ids_at(source)[1];
    let source_before = document.placed_tile(source).unwrap();
    let destination_before = document.placed_tile(destination).unwrap();

    assert_eq!(
        document.move_instance(
            &ObjectTree::default(),
            id,
            destination,
            "re-anchor table",
            &[VarMutation::Set("pixel_x".into(), base::types::Value::Num(-32.0))],
            None,
        ),
        Some(true),
    );
    assert_eq!(document.instance_location(id).unwrap().coord, destination);
    assert_eq!(
        document
            .prefab_instance(id)
            .ok_or("prefab instance is missing")?
            .prefab()
            .var(&"pixel_x".into()),
        Some(&base::types::Value::Num(-32.0)),
    );
    assert_eq!(
        document.placed_tile(source).unwrap(),
        source_before
            .iter()
            .filter(|placed| placed.id() != id)
            .cloned()
            .collect::<Vec<_>>(),
    );
    assert_eq!(
        document.placed_tile(destination).unwrap().len(),
        destination_before.len() + 1
    );

    assert!(document.undo());
    assert_eq!(document.instance_location(id).unwrap().coord, source);
    assert_eq!(document.instance_location(id).unwrap().prefab_index, 1);
    assert_eq!(
        document
            .prefab_instance(id)
            .ok_or("prefab instance is missing")?
            .prefab()
            .var(&"pixel_x".into()),
        None
    );
    assert_eq!(document.placed_tile(source).unwrap(), source_before);
    assert_eq!(document.placed_tile(destination).unwrap(), destination_before);
    Ok(())
}

#[test]
fn moving_onto_the_same_or_out_of_bounds_tile_only_edits_variables() -> Result<(), &'static str> {
    let mut document = MapDocument::new(shared_tile_map(), 1);
    let source = Coord::new(1, 1, 1);
    let id = document.instance_ids_at(source)[1];

    assert_eq!(
        document.move_instance(
            &ObjectTree::default(),
            id,
            source,
            "change pixel offset",
            &[VarMutation::Set("pixel_x".into(), base::types::Value::Num(4.0))],
            None,
        ),
        Some(true),
    );
    assert_eq!(
        document.move_instance(
            &ObjectTree::default(),
            id,
            Coord::new(9, 9, 1),
            "off the map",
            &[],
            None
        ),
        Some(false)
    );
    assert_eq!(document.instance_location(id).unwrap().coord, source);
    assert_eq!(
        document
            .prefab_instance(id)
            .ok_or("prefab instance is missing")?
            .prefab()
            .var(&"pixel_x".into()),
        Some(&base::types::Value::Num(4.0)),
    );
    Ok(())
}

#[test]
fn replacing_an_instance_path_preserves_its_id_variables_and_undo_history() -> Result<(), &'static str> {
    let mut document = MapDocument::new(shared_tile_map(), 1);
    let coord = Coord::new(1, 1, 1);
    let id = document.instance_ids_at(coord)[1];
    document
        .set_instance_var(id, "name".into(), base::types::Value::Text("custom".into()))
        .unwrap();
    document
        .set_instance_var(id, "dir".into(), base::types::Value::Num(8.0))
        .unwrap();
    let replacement = TreePath::parse("/obj/table/directional/north");

    assert_eq!(
        document.replace_instance_path(
            id,
            "set directional type",
            replacement.clone(),
            &[VarMutation::Remove("dir".into())],
            None,
        ),
        Some(true)
    );
    let instance = document.prefab_instance(id).ok_or("prefab instance is missing")?;
    let prefab = instance.prefab();
    let location = instance.location;
    assert_eq!(location.coord, coord);
    assert_eq!(prefab.path, replacement);
    assert_eq!(
        prefab.var(&"name".into()),
        Some(&base::types::Value::Text("custom".into()))
    );
    assert_eq!(prefab.var(&"dir".into()), None);

    assert!(document.undo());
    let prefab = document
        .prefab_instance(id)
        .ok_or("prefab instance is missing")?
        .prefab();
    assert_eq!(prefab.path, TreePath::parse("/obj/table"));
    assert_eq!(
        prefab.var(&"name".into()),
        Some(&base::types::Value::Text("custom".into()))
    );
    assert_eq!(prefab.var(&"dir".into()), Some(&base::types::Value::Num(8.0)));
    assert!(document.redo());
    assert_eq!(
        document
            .prefab_instance(id)
            .ok_or("prefab instance is missing")?
            .prefab()
            .path,
        replacement
    );
    Ok(())
}

#[test]
fn grouped_instance_path_replacements_are_one_undo_step() -> Result<(), &'static str> {
    let mut document = MapDocument::new(shared_tile_map(), 1);
    let id = document.instance_ids_at(Coord::new(1, 1, 1))[1];
    let group = EditGroupId::new();

    assert_eq!(
        document.replace_instance_path(
            id,
            "set direction",
            TreePath::parse("/obj/table/directional/north"),
            &[],
            Some(group),
        ),
        Some(true)
    );
    assert_eq!(
        document.replace_instance_path(
            id,
            "set direction",
            TreePath::parse("/obj/table/directional/east"),
            &[],
            Some(group),
        ),
        Some(true)
    );

    assert!(document.undo());
    assert_eq!(
        document
            .prefab_instance(id)
            .ok_or("prefab instance is missing")?
            .prefab()
            .path,
        TreePath::parse("/obj/table")
    );
    assert!(!document.undo());
    Ok(())
}

#[test]
fn grouped_instance_path_replacements_collapse_when_returned_to_the_start() {
    let mut document = MapDocument::new(shared_tile_map(), 1);
    let id = document.instance_ids_at(Coord::new(1, 1, 1))[1];
    let group = EditGroupId::new();

    document
        .replace_instance_path(
            id,
            "set direction",
            TreePath::parse("/obj/table/directional/north"),
            &[],
            Some(group),
        )
        .unwrap();
    document
        .replace_instance_path(id, "set direction", TreePath::parse("/obj/table"), &[], Some(group))
        .unwrap();

    assert!(!document.history.can_undo());
}

#[test]
fn variable_mutations_can_set_and_reset_a_pair_atomically() -> Result<(), &'static str> {
    let mut document = MapDocument::new(shared_tile_map(), 1);
    let id = document.instance_ids_at(Coord::new(1, 1, 1))[1];

    assert_eq!(
        document.edit_instance_vars(
            id,
            "set pixel offset",
            &[
                VarMutation::Set("pixel_x".into(), base::types::Value::Num(4.0)),
                VarMutation::Set("pixel_y".into(), base::types::Value::Num(-2.0)),
            ],
            None,
        ),
        Some(true),
    );
    assert_eq!(
        document.edit_instance_vars(
            id,
            "reset pixel offset",
            &[
                VarMutation::Remove("pixel_x".into()),
                VarMutation::Remove("pixel_y".into()),
            ],
            None,
        ),
        Some(true),
    );

    let prefab = document
        .prefab_instance(id)
        .ok_or("prefab instance is missing")?
        .prefab();
    assert_eq!(prefab.var(&"pixel_x".into()), None);
    assert_eq!(prefab.var(&"pixel_y".into()), None);
    Ok(())
}
#[test]
fn a_focused_area_confines_every_edit_without_blocking_undo() {
    let mut map = Map::new(Size { x: 3, y: 1, z: 1 });
    let key = map.intern_tile(vec![
        Prefab::new(TreePath::parse("/turf/floor")),
        Prefab::new(TreePath::parse("/obj/table")),
    ]);
    map.grid[0][0] = vec![key, key, key];
    let mut document = MapDocument::new(map, 1);
    let inside = Coord::new(1, 1, 1);
    let outside = Coord::new(3, 1, 1);
    let table = document.instance_ids_at(outside)[1];

    // an edit made before the focus was taken has to stay reversible
    assert_eq!(
        document.set_instance_var(table, "name".into(), Value::Text("early".into())),
        Some(true),
    );

    document.set_focus(Some(AreaFocus::new(
        inside,
        Prefab::new(TreePath::parse("/area/station")),
        document.instance_ids_at(inside)[0],
        [inside, Coord::new(2, 1, 1)].into_iter().collect(),
    )));
    assert!(document.allows_edit_at(inside));
    assert!(!document.allows_edit_at(outside));

    let outside_before = document.placed_tile(outside);
    let mut refused = Edit::new("delete outside the region");
    refused.change(&document, outside, Vec::new());
    assert!(!document.apply(refused));
    assert_eq!(document.placed_tile(outside), outside_before);
    assert_eq!(
        document.set_instance_var(table, "name".into(), Value::Text("late".into())),
        Some(false),
    );

    let mut allowed = Edit::new("delete inside the region");
    allowed.change(&document, inside, Vec::new());
    assert!(document.apply(allowed));
    assert_eq!(document.placed_tile(inside), Some(Vec::new()));

    // a move that leaves the region would otherwise clear its source and drop the instance
    let moved = document.instance_ids_at(Coord::new(2, 1, 1))[1];
    assert_eq!(
        document.move_instance(&ObjectTree::default(), moved, outside, "move out", &[], None),
        Some(false)
    );
    assert_eq!(
        document.instance_location(moved).map(|at| at.coord),
        Some(Coord::new(2, 1, 1))
    );

    assert!(document.undo());
    assert!(document.undo());
    assert_eq!(
        document
            .prefab_instance(table)
            .map(|instance| instance.prefab().var(&"name".into())),
        Some(None),
    );
}

#[test]
fn save_as_writes_the_chosen_format_and_retargets_the_document() {
    let dir = std::env::temp_dir().join(format!("rmd-save-as-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let original = dir.join("original.dmm");
    let exported = dir.join("exported.dmm");

    let mut document = MapDocument::open(&original, shared_tile_map(), 1);
    let coord = Coord::new(1, 1, 1);
    let mut edit = Edit::new("clear a tile");
    edit.change(&document, coord, Vec::new());
    assert!(document.apply(edit));
    assert!(document.is_dirty());

    document.save_as(&exported, dmm::MapFormat::Tgm).expect("save as tgm");

    let written = std::fs::read_to_string(&exported).expect("written map");
    assert!(written.starts_with("//MAP CONVERTED BY dmm2tgm.py"), "{written}");
    assert_eq!(document.path.as_deref(), Some(exported.as_path()));
    assert_eq!(document.format(), dmm::MapFormat::Tgm);
    assert!(!document.is_dirty());
    assert!(!original.exists(), "the original path must not be written to");

    // the format sticks, so a plain save keeps writing TGM
    std::fs::remove_file(&exported).expect("remove export");
    document.save().expect("save");
    assert!(
        std::fs::read_to_string(&exported)
            .expect("written map")
            .starts_with("//MAP CONVERTED BY dmm2tgm.py")
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn starting_the_journal_again_forgets_earlier_edits() {
    let mut document = MapDocument::new(shared_tile_map(), 2);
    document.start_journal();
    assert_eq!(document.append_level(&[Prefab::new(TreePath::parse("/turf"))]), Some(3));

    document.start_journal();

    assert!(document.take_journal().is_none());
}

#[test]
fn save_retains_appended_levels_with_remote_edits() {
    let dir = std::env::temp_dir().join(format!("rmd-remote-levels-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let target = dir.join("map.dmm");
    let mut document = MapDocument::new(shared_tile_map(), 2);
    let fill = [Prefab::new(TreePath::parse("/turf"))];
    assert_eq!(document.append_level(&fill), Some(3));

    let marker = vec![Prefab::new(TreePath::parse("/obj/marker"))];
    document.apply_remote(vec![(Coord::new(1, 1, 3), marker)]);
    document.save_as(&target, dmm::MapFormat::Standard).expect("save");

    let (written, errors) = dmm::parser::parse(&std::fs::read_to_string(&target).expect("written map"));
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(written.size.z, 3);
    assert_eq!(document.map.size().z, 3);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn failed_save_does_not_remove_untouched_appended_levels() {
    let missing = std::env::temp_dir().join(format!("rmd-missing-save-directory-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&missing);
    let target = missing.join("map.dmm");
    let mut document = MapDocument::new(shared_tile_map(), 2);
    let fill = [Prefab::new(TreePath::parse("/turf"))];
    assert_eq!(document.append_level(&fill), Some(3));
    document.z = 3;

    document
        .save_as(&target, dmm::MapFormat::Standard)
        .expect_err("the parent directory does not exist");

    assert_eq!(document.map.size().z, 3);
    assert_eq!(document.z, 3);
    assert_eq!(document.instance_ids_at(Coord::new(1, 1, 3)).len(), 1);
    assert!(document.is_dirty());
}

#[test]
fn a_created_document_is_dirty_and_will_not_replace_its_first_target() {
    let dir = std::env::temp_dir().join(format!("rmd-created-document-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let target = dir.join("new-map.dmm");
    let _ = std::fs::remove_file(&target);
    let mut document = MapDocument::create(&target, shared_tile_map(), 1);

    assert_eq!(document.path.as_deref(), Some(target.as_path()));
    assert!(document.is_dirty());

    std::fs::write(&target, "existing map").expect("existing target");
    let error = document.save().expect_err("the first save must not overwrite");
    assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "existing map");
    assert!(document.is_dirty());

    std::fs::remove_file(&target).expect("remove collision");
    document.save().expect("first save");
    assert!(!document.is_dirty());

    std::fs::write(&target, "replace me").expect("replace target contents");
    document.save().expect("subsequent save");
    assert_ne!(std::fs::read_to_string(&target).unwrap(), "replace me");

    let _ = std::fs::remove_dir_all(&dir);
}

fn level_document() -> MapDocument {
    let mut map = Map::new(Size { x: 2, y: 2, z: 3 });
    for z in 1..=3 {
        let tile = map.intern_tile(vec![Prefab::new(TreePath::parse(&format!("/obj/level{z}")))]);
        for row in &mut map.grid[z - 1] {
            row.fill(tile);
        }
    }

    MapDocument::new(map, 1)
}

#[test]
fn deletion_shifts_levels_and_undo_restores_every_instance() -> Result<(), Box<dyn Error>> {
    for z in 1..=3 {
        let mut document = level_document();
        let original = document.map.clone();
        let generation = document.generation();
        document.start_journal();
        assert!(document.delete_level(z));
        assert_eq!(document.map.size().z, 2);
        assert_eq!(document.z, z.min(2));
        assert!(document.is_dirty());
        assert!(document.generation() > generation);
        assert!(document.take_journal().is_some_and(|journal| journal.reshaped));
        for instance in original.prefab_instances() {
            let location = instance.location;
            if location.coord.z == z {
                assert!(document.map.prefab_instance(instance.id).is_none());
            } else {
                let now = document
                    .map
                    .prefab_instance(instance.id)
                    .ok_or("surviving instance is missing")?;
                assert_eq!(now.prefab(), instance.prefab());
                assert_eq!(
                    now.location.coord,
                    Coord::new(
                        location.coord.x,
                        location.coord.y,
                        location.coord.z - u32::from(location.coord.z > z)
                    )
                );
            }
        }

        assert!(document.undo());
        assert_eq!(document.map, original);
        assert!(!document.is_dirty());
        assert_eq!(document.z, z);
        assert!(document.take_journal().is_some_and(|journal| journal.reshaped));
        assert!(document.redo());
        assert_eq!(document.map.size().z, 2);
        assert!(document.undo());
        assert_eq!(document.map, original);
    }

    Ok(())
}

#[test]
fn creation_is_one_undo_step_and_ids_are_never_reused() {
    let mut document = level_document();
    let original = document.map.clone();
    let tile = [Prefab::new(TreePath::parse("/turf/floor"))];
    assert_eq!(document.append_level(&tile), Some(4));
    let created = document.map.clone();
    let ids = document.instance_ids_at(Coord::new(1, 1, 4)).to_vec();
    assert_eq!(document.history.undo_depth(), 1);
    assert!(document.undo());
    assert_eq!(document.map, original);
    assert!(!document.is_dirty());
    assert!(document.redo());
    assert_eq!(document.map, created);
    assert!(document.undo());
    assert_eq!(document.append_level(&tile), Some(4));
    assert_ne!(document.instance_ids_at(Coord::new(1, 1, 4)), ids);
    assert!(!document.redo());
}

#[test]
fn invalid_or_read_only_deletions_leave_history_and_state_unchanged() {
    let mut document = level_document();
    let original = document.map.clone();
    for z in [0, 4] {
        assert!(!document.delete_level(z));
    }

    document.set_read_only(true);
    assert!(!document.delete_level(2));
    assert!(document.append_level(&[]).is_none());
    assert!(!document.undo());
    assert_eq!(document.map, original);
    assert_eq!(document.history.undo_depth(), 0);
    document.set_read_only(false);
    assert!(document.delete_level(3));
    assert!(document.delete_level(2));
    assert!(!document.delete_level(1));
    assert_eq!(document.map.size().z, 1);
}

#[test]
fn structural_edits_clear_selection_and_save_preserves_all_levels() -> Result<(), Box<dyn Error>> {
    for format in [MapFormat::Standard, MapFormat::Tgm] {
        let mut document = level_document();
        assert_eq!(document.append_level(&[]), Some(4));
        let selected = Selection::from_drag(Coord::new(1, 1, 4), Coord::new(2, 2, 4));
        document.set_selection(Some(selected), RegionSelectionMode::Full);
        let original = document.map.clone();
        let path = env::temp_dir().join(format!("rmd-explicit-levels-{}-{format:?}.dmm", process::id()));
        document.start_journal();
        document.save_as(&path, format)?;
        document.save()?;
        assert_eq!(document.map, original);
        assert_eq!(document.z, 4);
        assert_eq!(document.selection(), Some(&selected));
        assert!(!document.is_dirty());
        assert!(document.take_journal().is_none());
        let (written, errors) = parser::parse(&fs::read_to_string(&path)?);
        assert!(errors.is_empty());
        assert_eq!(written.size.z, 4);
        assert!(document.delete_level(2));
        assert_eq!(document.selection(), None);
        document.save()?;
        assert!(!document.is_dirty());
        assert!(document.undo());
        assert!(document.is_dirty());
        assert_eq!(document.map, original);
        assert!(document.redo());
        assert!(!document.is_dirty());
        fs::remove_file(path)?;
    }

    Ok(())
}

#[test]
fn acknowledged_creation_records_only_the_authors_history_and_does_not_echo() {
    let mut author = level_document();
    let mut peer = level_document();
    author.start_journal();
    peer.start_journal();
    let empty = LevelContents::Fill(patch::encode_tile(&[]));
    let cells = author.level_cells(&empty);
    assert!(author.insert_requested_level(4, cells));
    let cells = peer.level_cells(&empty);
    assert!(peer.insert_remote_level(4, cells));
    assert_eq!(author.history.undo_depth(), 1);
    assert_eq!(peer.history.undo_depth(), 0);
    assert!(author.take_journal().is_none());
    assert!(peer.take_journal().is_none());
    assert!(author.is_dirty());
    assert!(peer.is_dirty());
    assert!(author.undo());
    assert!(author.take_journal().is_some_and(|journal| journal.reshaped));
}

#[test]
fn read_only_documents_refuse_requested_and_remote_level_changes() {
    let mut document = level_document();
    let empty = LevelContents::Fill(patch::encode_tile(&[]));
    assert_eq!(document.append_level(&[]), Some(4));
    let original = document.map.clone();
    document.set_read_only(true);

    let cells = document.level_cells(&empty);
    assert!(!document.insert_requested_level(5, cells));
    let cells = document.level_cells(&empty);
    assert!(!document.insert_remote_level(5, cells));
    assert!(!document.delete_requested_level(2));
    assert!(!document.delete_remote_level(2));
    assert!(!document.undo_requested_level(LevelChange::Deleted(4)));
    assert_eq!(document.map, original);
    assert_eq!(document.history.undo_depth(), 1);

    document.set_read_only(false);
    assert!(document.undo_requested_level(LevelChange::Deleted(4)));
    document.set_read_only(true);
    assert!(!document.redo_requested_level(LevelChange::Inserted(4)));
    assert_eq!(document.history.undo_depth(), 0);

    document.set_read_only(false);
    assert!(document.redo_requested_level(LevelChange::Inserted(4)));
    assert_eq!(document.map, original);
}
