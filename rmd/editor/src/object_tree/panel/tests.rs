use base::{
    coord::Coord,
    location::Location,
    path::TreePath,
    types::{Identifier, Value, VarModifiers},
};

use dmm::{Map, Size};
use objtree::VarDecl;
use prefab::Prefab;
use ui::testing::CONTEXT_LOCK;

use super::{
    type_search::{MAX_CUSTOM_FILL_SEARCH_RESULTS, matching_type_paths_up_to_filtered},
    *,
};
use crate::{document::MapDocument, tool::Tool};

fn set_type_name(tree: &mut ObjectTree, id: TypeId, value: &str) {
    let name = Identifier::from("name");
    tree.get_mut(id).unwrap().vars.insert(
        name.clone(),
        VarDecl {
            name,
            declared_type: None,
            modifiers: VarModifiers::default(),
            value: Value::Text(value.to_owned()),
            declared: true,
            location: Location::default(),
            resolved_type: None,
            shared: None,
        },
    );
}

#[test]
fn object_tree_search_is_not_limited_to_custom_fill_result_count() {
    let mut tree = ObjectTree::new();
    for index in 0..60 {
        tree.register(
            &TreePath::parse(&format!("/obj/floor/type_{index}")),
            Location::default(),
        );
    }

    assert_eq!(
        matching_type_paths_up_to_filtered(&tree, "floor", MAX_CUSTOM_FILL_SEARCH_RESULTS, |_| true).len(),
        MAX_CUSTOM_FILL_SEARCH_RESULTS
    );
    assert_eq!(
        matching_object_types(&tree, "floor", ObjectTreeSearchOptions::default()).len(),
        61
    );
}

#[test]
fn object_tree_search_matches_inherited_atom_names() {
    let mut tree = ObjectTree::new();
    let atom = tree.register(&TreePath::parse("/atom"), Location::default());
    let parent = tree.register(&TreePath::parse("/atom/movable/tool"), Location::default());
    let child = tree.register(&TreePath::parse("/atom/movable/tool/wrench"), Location::default());
    tree.register(&TreePath::parse("/atom/structure/table"), Location::default());
    set_type_name(&mut tree, parent, "Portable Tool");

    let options = ObjectTreeSearchOptions {
        type_paths: false,
        names: true,
    };
    let matches = matching_object_types(&tree, "PORTABLE", options);
    let filter = ObjectTreeFilter::new(&tree, atom, "PORTABLE", options, ObjectTreeTypeFilter::default());

    assert_eq!(matches, [parent, child]);
    assert_eq!(filter.roots, [parent]);
    assert_eq!(filter.children(parent), [child]);
}

#[test]
fn object_tree_search_combines_enabled_fields() {
    let mut tree = ObjectTree::new();
    tree.register(&TreePath::parse("/atom"), Location::default());
    let path_match = tree.register(&TreePath::parse("/atom/movable/needle"), Location::default());
    let name_match = tree.register(&TreePath::parse("/atom/movable/scalpel"), Location::default());
    set_type_name(&mut tree, name_match, "Needle tool");

    let path_only = matching_object_types(
        &tree,
        "needle",
        ObjectTreeSearchOptions {
            type_paths: true,
            names: false,
        },
    );
    let name_only = matching_object_types(
        &tree,
        "needle",
        ObjectTreeSearchOptions {
            type_paths: false,
            names: true,
        },
    );
    let combined = matching_object_types(
        &tree,
        "needle",
        ObjectTreeSearchOptions {
            type_paths: true,
            names: true,
        },
    );

    assert_eq!(path_only, [path_match]);
    assert_eq!(name_only, [name_match]);
    assert_eq!(combined, [path_match, name_match]);
}

#[test]
fn object_tree_children_are_sorted_by_path() {
    let mut tree = ObjectTree::new();
    tree.register(&TreePath::parse("/obj/zeta"), Location::default());
    tree.register(&TreePath::parse("/obj/alpha"), Location::default());
    let obj = tree.id_of(&TreePath::parse("/obj")).expect("registered parent");
    let paths = sorted_children(&tree, obj)
        .into_iter()
        .filter_map(|id| tree.get(id))
        .map(|decl| decl.path.to_string())
        .collect::<Vec<_>>();

    assert_eq!(paths, ["/obj/alpha", "/obj/zeta"]);
}

#[test]
fn built_in_object_tree_filters_prune_the_base_types_and_all_subtypes() {
    let mut tree = ObjectTree::new();
    let atom = tree.register(&TreePath::parse("/atom"), Location::default());
    let movable = tree.register(&TreePath::parse("/atom/movable"), Location::default());
    let item = tree.register(&TreePath::parse("/obj/item"), Location::default());
    let open_turf = tree.register(&TreePath::parse("/turf/open"), Location::default());
    let roots = tree.roots();
    tree.get_mut(roots.obj.unwrap()).unwrap().parent_type = Some(TreePath::parse("/atom/movable"));
    tree.get_mut(roots.turf.unwrap()).unwrap().parent_type = Some(TreePath::parse("/atom"));
    tree.resolve_parent_types();

    for (options, filtered_root, filtered_subtype) in [
        (
            ObjectTreeFilterOptions {
                atom: true,
                ..ObjectTreeFilterOptions::default()
            },
            atom,
            open_turf,
        ),
        (
            ObjectTreeFilterOptions {
                movable: true,
                ..ObjectTreeFilterOptions::default()
            },
            movable,
            item,
        ),
        (
            ObjectTreeFilterOptions {
                obj: true,
                ..ObjectTreeFilterOptions::default()
            },
            roots.obj.unwrap(),
            item,
        ),
        (
            ObjectTreeFilterOptions {
                turf: true,
                ..ObjectTreeFilterOptions::default()
            },
            roots.turf.unwrap(),
            open_turf,
        ),
    ] {
        let filter = ObjectTreeTypeFilter::new(&tree, &options);

        assert_eq!(filter.action(&tree, filtered_root), ObjectTreeFilterAction::Prune);
        assert_eq!(filter.action(&tree, filtered_subtype), ObjectTreeFilterAction::Prune);
    }
}

#[test]
fn custom_object_tree_filter_prunes_the_type_and_all_subtypes() {
    let mut tree = ObjectTree::new();
    let atom = tree.register(&TreePath::parse("/atom"), Location::default());
    let keep = tree.register(&TreePath::parse("/atom/keep"), Location::default());
    let removed = tree.register(&TreePath::parse("/atom/remove"), Location::default());
    let removed_child = tree.register(&TreePath::parse("/atom/remove/child"), Location::default());
    let options = ObjectTreeFilterOptions {
        custom_enabled: true,
        custom_type_path: String::from("/atom/remove"),
        ..ObjectTreeFilterOptions::default()
    };
    let filter = ObjectTreeTypeFilter::new(&tree, &options);

    assert_eq!(visible_type_children(&tree, atom, filter), [keep]);
    assert_eq!(filter.action(&tree, removed), ObjectTreeFilterAction::Prune);
    assert_eq!(filter.action(&tree, removed_child), ObjectTreeFilterAction::Prune);
}

#[test]
fn object_tree_search_respects_the_active_type_filters() {
    let mut tree = ObjectTree::new();
    let atom = tree.register(&TreePath::parse("/atom"), Location::default());
    let keep = tree.register(&TreePath::parse("/atom/keep_match"), Location::default());
    tree.register(&TreePath::parse("/atom/remove/keep_match"), Location::default());
    let type_filter = ObjectTreeTypeFilter::new(
        &tree,
        &ObjectTreeFilterOptions {
            custom_enabled: true,
            custom_type_path: String::from("/atom/remove"),
            ..ObjectTreeFilterOptions::default()
        },
    );

    let filter = ObjectTreeFilter::new(
        &tree,
        atom,
        "keep_match",
        ObjectTreeSearchOptions::default(),
        type_filter,
    );

    assert_eq!(filter.roots, [keep]);
    assert!(filter.children(keep).is_empty());
    assert!(filter.contains(keep));
}

#[test]
fn a_selected_map_instance_queues_its_type_for_object_tree_reveal() {
    let path = TreePath::parse("/atom/structure/table");
    let mut tree = ObjectTree::new();
    let selected_type = tree.register(&path, Location::default());
    let mut map = Map::new(Size { x: 1, y: 1, z: 1 });
    let tile = map.intern_tile(vec![Prefab::new(path)]);
    map.grid[0][0][0] = tile;
    let document = MapDocument::new(map, 1);
    let picked_instance = document.instance_ids_at(Coord::new(1, 1, 1))[0];
    let mut session = Session::new();
    session.state.environment = Some(std::sync::Arc::new(crate::environment::Environment::new(".", tree)));
    session.state.open_document(document);
    session.set_tool(Tool::Pick);
    session.pick_instance(Some(picked_instance));
    let mut state = ObjectTreePanel::new().expect("valid window key");

    state.reveal_picked_instance(&session);

    assert_eq!(state.selected, Some(selected_type));
    assert_eq!(state.reveal, Some(selected_type));
    assert_eq!(session.tool(), Tool::Pick);
}

#[test]
fn object_tree_rows_follow_expansion_and_render_a_clipped_search() {
    let _context = CONTEXT_LOCK.lock().unwrap();
    let mut tree = ObjectTree::new();
    let thing = tree.register(&TreePath::parse("/atom/structure/thing"), Location::default());
    tree.register(&TreePath::parse("/atom/movable/item"), Location::default());
    let atom = tree.roots().atom.expect("registered /atom root");
    let mut context = dear_imgui_rs::Context::create();
    context
        .font_atlas()
        .try_claim_legacy_renderer()
        .expect("legacy renderer font atlas should be available")
        .build();
    context.io_mut().set_display_size([128.0, 128.0]);
    context.io_mut().set_delta_time(1.0 / 60.0);
    let ui = context.frame();

    let expanded = ui
        .window("object-tree-expanded")
        .build(|| {
            let mut rows = Vec::new();
            collect_type_rows(
                ui,
                &tree,
                atom,
                None,
                true,
                &mut rows,
                &ObjectTreeRowOptions {
                    expand: true,
                    ..ObjectTreeRowOptions::default()
                },
            );
            rows
        })
        .expect("test window should be visible");
    let paths = expanded
        .iter()
        .filter_map(|row| tree.get(row.id))
        .map(|decl| decl.path.to_string())
        .collect::<Vec<_>>();
    let parents = expanded.iter().map(|row| row.parent).collect::<Vec<_>>();
    let last_siblings = expanded.iter().map(|row| row.last_sibling).collect::<Vec<_>>();

    assert_eq!(
        paths,
        [
            "/atom",
            "/atom/movable",
            "/atom/movable/item",
            "/atom/structure",
            "/atom/structure/thing"
        ]
    );
    assert_eq!(parents, [None, Some(0), Some(1), Some(0), Some(3)]);
    assert_eq!(last_siblings, [true, false, true, true, true]);

    let opened_by_default = ui
        .window("object-tree-default-open")
        .build(|| {
            let mut rows = Vec::new();
            collect_type_rows(ui, &tree, atom, None, true, &mut rows, &ObjectTreeRowOptions::default());
            rows
        })
        .expect("test window should be visible");
    let default_paths = opened_by_default
        .iter()
        .filter_map(|row| tree.get(row.id))
        .map(|decl| decl.path.to_string())
        .collect::<Vec<_>>();
    assert_eq!(default_paths, ["/atom", "/atom/movable", "/atom/structure"]);

    let collapsed = ui
        .window("object-tree-collapsed")
        .build(|| {
            let atom_storage_id = ui.get_id("/atom");
            ui.with_current_state_storage(|mut storage| storage.set_bool(atom_storage_id, false));
            let mut rows = Vec::new();
            collect_type_rows(ui, &tree, atom, None, true, &mut rows, &ObjectTreeRowOptions::default());
            rows
        })
        .expect("test window should be visible");
    assert_eq!(
        collapsed,
        [ObjectTreeRow {
            id: atom,
            parent: None,
            leaf: false,
            last_sibling: true
        }]
    );

    let revealed = ui
        .window("object-tree-revealed")
        .build(|| {
            let atom_storage_id = ui.get_id("/atom");
            ui.with_current_state_storage(|mut storage| storage.set_bool(atom_storage_id, false));
            let mut rows = Vec::new();
            collect_type_rows(
                ui,
                &tree,
                atom,
                None,
                true,
                &mut rows,
                &ObjectTreeRowOptions {
                    reveal: Some(thing),
                    ..ObjectTreeRowOptions::default()
                },
            );
            rows
        })
        .expect("test window should be visible");
    let revealed_paths = revealed
        .iter()
        .filter_map(|row| tree.get(row.id))
        .map(|decl| decl.path.to_string())
        .collect::<Vec<_>>();
    assert_eq!(
        revealed_paths,
        ["/atom", "/atom/movable", "/atom/structure", "/atom/structure/thing"]
    );

    let structure = tree.id_of(&TreePath::parse("/atom/structure")).unwrap();
    let filtered = ui
        .window("object-tree-filtered-reveal")
        .build(|| {
            let atom_storage_id = ui.get_id("/atom");
            ui.with_current_state_storage(|mut storage| storage.set_bool(atom_storage_id, false));
            let mut rows = Vec::new();
            collect_type_rows(
                ui,
                &tree,
                atom,
                None,
                true,
                &mut rows,
                &ObjectTreeRowOptions {
                    type_filter: ObjectTreeTypeFilter {
                        custom: Some(structure),
                        ..ObjectTreeTypeFilter::default()
                    },
                    reveal: Some(thing),
                    ..ObjectTreeRowOptions::default()
                },
            );
            rows
        })
        .expect("test window should be visible");
    assert_eq!(
        filtered,
        [ObjectTreeRow {
            id: atom,
            parent: None,
            leaf: false,
            last_sibling: true
        }]
    );

    let mut tree = ObjectTree::new();
    for index in 0..500 {
        tree.register(
            &TreePath::parse(&format!("/atom/common/type_{index}")),
            Location::default(),
        );
    }
    let mut session = Session::new();
    session.state.environment = Some(std::sync::Arc::new(crate::environment::Environment::new(".", tree)));
    let mut state = ObjectTreePanel::new().expect("valid window key");
    state.search = String::from("atom");
    state.filter_revision = u64::MAX;
    let mut settings = Settings::default();

    state.draw(ui, &mut session, &mut settings, false, &mut true);

    assert!(context.render_legacy().valid());
}
