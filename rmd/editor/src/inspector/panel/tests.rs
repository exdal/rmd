use base::{
    location::Location,
    path::TreePath,
    types::{Identifier, Value, VarModifiers},
};

use dmm::Prefab;
use objtree::{ObjectTree, VarDecl};

use super::*;

fn two_tables() -> (Session, [PrefabInstanceId; 2]) {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/env");
    let mut session = Session::new();
    session.load_environment(&root.join("test.dme")).unwrap();
    let mut map = dmm::Map::new(dmm::Size { x: 3, y: 1, z: 1 });
    let key = map.intern_tile(vec![
        Prefab::new(TreePath::parse("/turf/open/floor")),
        Prefab::new(TreePath::parse("/obj/structure/table")),
    ]);
    map.grid[0][0].fill(key);
    session.apply_map(crate::environment::worker::LoadedMap {
        path: root.join("identical-test.dmm"),
        map,
        z: 1,
        errors: vec![],
        repo: None,
        conflict: None,
    });
    let document = session.state.active_document().unwrap();
    let tables = [1, 2].map(|x| document.instance_ids_at(Coord::new(x, 1, 1))[1]);
    session.pick_instance(Some(tables[0]));

    (session, tables)
}

fn pixel_and_tile(session: &Session, id: PrefabInstanceId) -> Option<(Option<Value>, Coord)> {
    let instance = session.state.active_document()?.prefab_instance(id)?;
    let prefab = instance.prefab();
    let location = instance.location;

    Some((prefab.var(&"pixel_x".into()).cloned(), location.coord))
}

#[test]
fn the_identical_banner_replaces_the_checkbox_while_the_mode_is_on() {
    let _guard = ui::testing::CONTEXT_LOCK.lock().unwrap();
    let mut context = ui::testing::rectangle_context();
    let (mut session, _) = two_tables();
    let mut settings = Settings::default();
    let mut panel = InspectorPanel::new().unwrap();
    panel.state.scope = EditScope::Identical;

    for _ in 0..2 {
        let ui = context.frame();
        panel.draw(ui, &mut session, &mut settings, false, &mut true);
        assert!(context.render_legacy().valid());
    }

    assert!(panel.edits_identical());
    assert_eq!(session.identical().len(), 3);
}

#[test]
fn a_filtered_panel_with_pins_draws_every_filter_state() {
    let _guard = ui::testing::CONTEXT_LOCK.lock().unwrap();
    let mut context = ui::testing::rectangle_context();
    let (mut session, _) = two_tables();
    let mut settings = Settings {
        pinned_vars: vec![String::from("desc"), String::from("missing")],
        ..Settings::default()
    };
    let mut panel = InspectorPanel::new().unwrap();

    for (query, modified_only) in [("", false), ("pixel", false), ("", true), ("nothing matches", true)] {
        panel.state.filter.query = String::from(query);
        panel.state.filter.modified_only = modified_only;
        let ui = context.frame();
        panel.draw(ui, &mut session, &mut settings, false, &mut true);
        assert!(context.render_legacy().valid());
    }

    assert_eq!(settings.pinned_vars, ["desc", "missing"]);
}

#[test]
fn identical_offsets_are_set_as_typed_instead_of_moving_to_another_tile() {
    let (mut session, tables) = two_tables();
    let pixel_x = Identifier::from("pixel_x");

    commit_int_property(
        &mut session,
        EditScope::Identical,
        TransformMode::Pixel,
        &pixel_x,
        40,
        EditGroupId::new(),
    );

    assert_eq!(
        tables.map(|id| pixel_and_tile(&session, id)),
        [
            Some((Some(Value::Num(40.0)), Coord::new(1, 1, 1))),
            Some((Some(Value::Num(40.0)), Coord::new(2, 1, 1)))
        ]
    );

    let (mut session, tables) = two_tables();
    commit_int_property(
        &mut session,
        EditScope::Selected,
        TransformMode::Pixel,
        &pixel_x,
        40,
        EditGroupId::new(),
    );
    assert_eq!(
        pixel_and_tile(&session, tables[0]),
        Some((Some(Value::Num(8.0)), Coord::new(2, 1, 1))),
        "a single object is re-anchored onto the tile it was dragged over"
    );
    assert_eq!(pixel_and_tile(&session, tables[1]), Some((None, Coord::new(2, 1, 1))));
}

#[test]
fn direction_choices_prefer_directional_subtypes_over_dmi_slots() {
    let mut supported = [false; 8];
    supported[1] = true;
    supported[2] = true;
    supported[7] = true;
    let types = DirectionalTypes {
        supported,
        current: Some(Dir::North),
    };

    assert_eq!(
        direction_choices(8, None, Some(types)),
        [Dir::North, Dir::East, Dir::Northwest]
    );
    assert_eq!(selected_direction(Dir::South.to_bits(), Some(types)), Some(Dir::North));
    assert_eq!(direction_choices(4, None, None), Dir::ORDER[..4]);

    let mut cardinals = [false; 8];
    cardinals[..4].fill(true);
    assert_eq!(direction_choices(1, Some(cardinals), None), Dir::ORDER[..4]);
    assert_eq!(
        direction_choices(1, Some(cardinals), Some(types)),
        [Dir::North, Dir::East, Dir::Northwest]
    );
    assert_eq!(selected_direction(Dir::South.to_bits(), None), Some(Dir::South));
}

#[test]
fn inspector_separates_special_properties_and_closest_editable_defaults() {
    let mut tree = ObjectTree::new();
    let parent = tree.register(&TreePath::parse("/obj"), Location::default());
    let child = tree.register(&TreePath::parse("/obj/item"), Location::default());

    add_var(&mut tree, parent, "inherited", Value::Num(1.0), VarModifiers::default());
    add_var(&mut tree, parent, "closest", Value::Num(1.0), VarModifiers::default());
    add_var(
        &mut tree,
        parent,
        "parent_only",
        Value::Text("parent".into()),
        VarModifiers::default(),
    );
    add_var(&mut tree, child, "inherited", Value::Num(2.0), VarModifiers::default());
    add_var(&mut tree, child, "closest", Value::Num(2.0), VarModifiers::default());
    for name in ["inherited", "closest"] {
        tree.get_mut(child)
            .unwrap()
            .vars
            .get_mut(&Identifier::from(name))
            .unwrap()
            .declared = false;
    }

    add_var(&mut tree, child, "runtime", Value::Unevaluated, VarModifiers::default());
    add_var(
        &mut tree,
        child,
        "temporary",
        Value::Num(3.0),
        VarModifiers {
            is_tmp: true,
            ..Default::default()
        },
    );

    let mut prefab = Prefab::new(TreePath::parse("/obj/item"));
    prefab.set_var("color".into(), Value::Text("#ff0000".into()));
    prefab.set_var("inherited".into(), Value::Num(4.0));
    let excluded = HashSet::from([Identifier::from("color")]);
    let variables = inspector_variables(Some(&tree), &prefab, &excluded);

    assert_eq!(
        variables,
        [
            InspectorVariable {
                name: "closest".into(),
                source: String::from("2"),
                overridden: false,
                value: Value::Num(2.0),
                ty: None,
                owner: Some(parent),
            },
            InspectorVariable {
                name: "inherited".into(),
                source: String::from("4"),
                overridden: true,
                value: Value::Num(4.0),
                ty: None,
                owner: Some(parent),
            },
            InspectorVariable {
                name: "parent_only".into(),
                source: String::from("\"parent\""),
                overridden: false,
                value: Value::Text("parent".into()),
                ty: None,
                owner: Some(parent),
            },
            InspectorVariable {
                name: "runtime".into(),
                source: String::from("<runtime>"),
                overridden: false,
                value: Value::Unevaluated,
                ty: None,
                owner: Some(child),
            },
        ],
    );
}

#[test]
fn builtin_groups_are_plain_text_while_project_groups_link_to_source() {
    let (session, _) = two_tables();
    let snapshot = inspector_snapshot(&session).expect("selected table snapshot");
    let table = snapshot
        .groups
        .iter()
        .find(|group| group.path == "/obj/structure/table")
        .expect("project type group");
    let source = table.source.as_ref().expect("project source link");
    assert!(source.path.ends_with("test.dm"));

    for path in ["/atom", "/atom/movable", "/datum"] {
        let group = snapshot
            .groups
            .iter()
            .find(|group| group.path == path)
            .expect("builtin type group");
        assert!(group.source.is_none(), "{path} must render as plain text");
    }
}

#[test]
fn variables_group_under_their_declaring_types_from_child_to_parent() {
    let mut tree = ObjectTree::new();
    let parent = tree.register(&TreePath::parse("/obj"), Location::default());
    let child = tree.register(&TreePath::parse("/obj/item"), Location::default());
    let selected = tree.register(&TreePath::parse("/obj/item/sub"), Location::default());
    add_var(&mut tree, parent, "density", Value::Num(1.0), VarModifiers::default());
    add_var(&mut tree, selected, "density", Value::Num(0.0), VarModifiers::default());
    tree.get_mut(selected)
        .unwrap()
        .vars
        .get_mut(&Identifier::from("density"))
        .unwrap()
        .declared = false;

    let mut prefab = Prefab::new(TreePath::parse("/obj/item/sub"));
    prefab.set_var("missing".into(), Value::Num(1.0));
    let variables = inspector_variables(Some(&tree), &prefab, &HashSet::new());
    let owner = |name: &str| {
        variables
            .iter()
            .find(|variable| variable.name.as_str() == name)
            .unwrap()
            .owner
    };
    assert_eq!(owner("density"), Some(parent));
    assert_eq!(owner("missing"), None);

    let groups = variable_groups(Some(&tree), Some(selected), &prefab.path, &variables, |_| None);
    let summary = groups
        .iter()
        .map(|group| (group.owner, group.path.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        summary,
        [
            (Some(selected), "/obj/item/sub"),
            (Some(parent), "/obj"),
            (None, "Undeclared"),
        ]
    );
    assert!(!summary.iter().any(|(owner, _)| *owner == Some(child)));

    let groups = variable_groups(None, None, &prefab.path, &[], |_| None);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].path, "/obj/item/sub");
}

#[test]
fn editing_a_default_does_not_move_its_row() {
    let mut tree = ObjectTree::new();
    let id = tree.register(&TreePath::parse("/obj/item"), Location::default());
    for name in ["alpha_var", "beta_var", "gamma_var"] {
        add_var(&mut tree, id, name, Value::Num(1.0), VarModifiers::default());
    }

    let mut prefab = Prefab::new(TreePath::parse("/obj/item"));
    let names = |variables: Vec<InspectorVariable>| {
        variables
            .into_iter()
            .map(|variable| variable.name.as_str().to_string())
            .collect::<Vec<_>>()
    };
    let before = names(inspector_variables(Some(&tree), &prefab, &HashSet::new()));

    prefab.set_var("gamma_var".into(), Value::Num(9.0));
    let edited = inspector_variables(Some(&tree), &prefab, &HashSet::new());
    assert!(edited.iter().any(|variable| variable.overridden));
    assert_eq!(names(edited), before);

    prefab.set_var("alpha_var".into(), Value::Num(9.0));
    assert_eq!(
        names(inspector_variables(Some(&tree), &prefab, &HashSet::new())),
        before
    );
}

#[test]
fn the_filter_matches_var_names_and_can_hide_inherited_values() {
    let mut filter = InspectorFilter::default();
    assert!(!filter.is_active());
    assert!(filter.shows("icon_state", false));

    filter.query = String::from(" Icon State ");
    assert!(filter.is_active());
    assert!(filter.shows("icon_state", false), "spaces stand in for underscores");
    assert!(!filter.shows("icon", false));

    filter.query.clear();
    filter.modified_only = true;
    assert!(filter.is_active());
    assert!(filter.shows("name", true));
    assert!(!filter.shows("name", false));
}

#[test]
fn pinned_variables_leave_the_list_in_pin_order() {
    let variable = |name: &str, overridden| InspectorVariable {
        name: name.into(),
        source: String::new(),
        overridden,
        value: Value::Null,
        ty: None,
        owner: None,
    };
    let variables = [
        variable("anchored", false),
        variable("density", false),
        variable("dir", true),
        variable("req_access", true),
    ];
    let pins = [
        String::from("density"),
        String::from("missing"),
        String::from("req_access"),
    ];

    let (pinned, rest) = split_pinned(&variables, &pins);
    let names = |variables: Vec<&InspectorVariable>| {
        variables
            .into_iter()
            .map(|variable| variable.name.as_str().to_string())
            .collect::<Vec<_>>()
    };

    assert_eq!(names(pinned), ["density", "req_access"]);
    assert_eq!(names(rest), ["anchored", "dir"]);
}

#[test]
fn special_property_snapshots_preserve_override_source_and_fallback_defaults() {
    let mut prefab = Prefab::new(TreePath::parse("/obj/item"));
    prefab.set_var("pixel_x".into(), Value::Num(4.0));

    let pixel = property_snapshot(None, None, &prefab, "pixel_x");
    let alpha = property_snapshot(None, None, &prefab, "alpha");

    assert!(pixel.overridden);
    assert_eq!(pixel.value, Value::Num(4.0));
    assert_eq!(pixel.editor_text, "4");
    assert!(!alpha.overridden);
    assert_eq!(alpha.value, Value::Num(255.0));
}

#[test]
fn color_picker_values_use_compact_hex() {
    assert_eq!(format_color([1.0, 0.0, 0.5, 1.0]), "#FF0080");
    assert_eq!(format_color([0.0, 1.0, 0.0, 0.5]), "#00FF0080");
}

#[test]
fn ordinary_variable_widgets_follow_types_and_preserve_incompatible_values() {
    let variable = |name: &str, value, kind| InspectorVariable {
        name: name.into(),
        source: String::new(),
        overridden: true,
        value,
        ty: Some(ResolvedVarType {
            kinds: vec![kind],
            nullable: true,
        }),
        owner: None,
    };
    assert_eq!(
        variable_widget(&variable("density", Value::Num(1.0), VarTypeKind::Bool)),
        VariableWidget::Boolean
    );
    assert_eq!(
        variable_widget(&variable("density", Value::Num(2.0), VarTypeKind::Bool)),
        VariableWidget::Raw
    );
    assert_eq!(
        variable_widget(&variable("density", Value::Num(1.0), VarTypeKind::Number)),
        VariableWidget::Number
    );
    assert_eq!(
        variable_widget(&variable("damage", Value::Num(1.0), VarTypeKind::Number)),
        VariableWidget::Number
    );
    assert_eq!(
        variable_widget(&variable("title", Value::Null, VarTypeKind::Text)),
        VariableWidget::Text
    );
    assert_eq!(
        variable_widget(&variable("title", Value::Num(5.0), VarTypeKind::Text)),
        VariableWidget::Raw
    );
    assert_eq!(
        variable_widget(&variable("items", Value::List(Vec::new()), VarTypeKind::List)),
        VariableWidget::List
    );
    let mut newlist = variable("items", Value::List(Vec::new()), VarTypeKind::List);
    newlist.source = String::from("newlist(/obj/item)");
    assert_eq!(variable_widget(&newlist), VariableWidget::Raw);
    newlist.source = String::from("list(\"a\" = newlist(/obj/item))");
    assert_eq!(variable_widget(&newlist), VariableWidget::Raw);
    assert_eq!(
        variable_widget(&variable("locked", Value::Null, VarTypeKind::Bool)),
        VariableWidget::Raw
    );
}

#[test]
fn text_and_null_values_open_as_expressions() {
    let mut draft = VariableDraft::new("", &Value::Null);
    let text = Value::Text("pod".into());
    assert_eq!(editor_mode(VariableWidget::Text, &text, &draft), EditorMode::Expression);
    assert_eq!(
        editor_mode(VariableWidget::Number, &Value::Null, &draft),
        EditorMode::Expression
    );
    assert_eq!(
        editor_mode(VariableWidget::Number, &Value::Num(1.0), &draft),
        EditorMode::Typed
    );

    draft.mode = Some(EditorMode::Typed);
    assert_eq!(editor_mode(VariableWidget::Text, &text, &draft), EditorMode::Typed);
    assert_eq!(
        editor_mode(VariableWidget::Raw, &Value::Null, &draft),
        EditorMode::Expression
    );
}

#[test]
fn typed_drafts_follow_outside_edits_unless_they_are_being_edited() {
    let mut draft = VariableDraft::new("\"old\"", &Value::Text("old".into()));
    draft.text = String::from("\"unsent\"");
    draft.sync("\"undo\"", &Value::Text("undo".into()));
    assert_eq!(draft.typed.text, "undo");
    assert_eq!(draft.text, "\"unsent\"");

    draft.typed.text = String::from("typing");
    draft.sync("\"remote\"", &Value::Text("remote".into()));
    assert_eq!(draft.typed.text, "typing");
}

#[test]
fn unknown_variable_widgets_follow_the_value() {
    let variable = |value, ty| InspectorVariable {
        name: "custom_materials".into(),
        source: String::new(),
        overridden: true,
        value,
        ty,
        owner: None,
    };
    let unknown = || {
        Some(ResolvedVarType {
            kinds: Vec::new(),
            nullable: true,
        })
    };
    let list = Value::List(vec![ListEntry {
        key: Value::Path(TreePath::parse("/datum/material/plasma")),
        value: Some(Value::Num(100000.0)),
    }]);
    let runtime_list = Value::List(vec![ListEntry {
        key: Value::Unevaluated,
        value: None,
    }]);

    assert_eq!(variable_widget(&variable(list.clone(), None)), VariableWidget::List);
    assert_eq!(variable_widget(&variable(list, unknown())), VariableWidget::List);
    assert_eq!(
        variable_widget(&variable(Value::Num(1.0), None)),
        VariableWidget::Number
    );
    assert_eq!(
        variable_widget(&variable(Value::Text("a".into()), unknown())),
        VariableWidget::Text
    );
    assert_eq!(variable_widget(&variable(Value::Null, unknown())), VariableWidget::Raw);
    assert_eq!(variable_widget(&variable(runtime_list, None)), VariableWidget::Raw);
}

#[test]
fn list_entry_edits_keep_optional_keys_and_nested_values() {
    let entries = [
        ListEntryDraft {
            key: String::from("1"),
            value: String::new(),
        },
        ListEntryDraft {
            key: String::from("\"key\""),
            value: String::from("list(2)"),
        },
    ];
    let value = parse_list_entries(&entries).unwrap();
    assert_eq!(parse_value(&format_value(&value)).unwrap(), value);
    let drafts = list_entry_drafts(&value);
    assert_eq!(drafts[0].value, "");
    assert_eq!(drafts[1].value, "list(2)");
}

#[test]
fn list_entry_edits_reject_bare_word_values() {
    let entries = [ListEntryDraft {
        key: String::from("\"key\""),
        value: String::from("weirdpodthing1"),
    }];
    assert!(parse_list_entries(&entries).is_err());
}

#[test]
fn type_labels_join_kinds_and_nullability() {
    let ty = |kinds, nullable| ResolvedVarType { kinds, nullable };
    assert_eq!(type_label(None, None), "unknown");
    assert_eq!(type_label(None, Some(&ty(Vec::new(), true))), "unknown");
    assert_eq!(
        type_label(None, Some(&ty(vec![VarTypeKind::Number, VarTypeKind::Text], true))),
        "number | text | null"
    );
    assert_eq!(type_label(None, Some(&ty(vec![VarTypeKind::List], false))), "list");
}

fn add_var(tree: &mut ObjectTree, id: TypeId, name: &str, value: Value, modifiers: VarModifiers) {
    let name = Identifier::from(name);
    tree.get_mut(id).unwrap().vars.insert(
        name.clone(),
        VarDecl {
            name,
            declared_type: None,
            modifiers,
            value,
            declared: true,
            location: Location::default(),
            resolved_type: None,
            shared: None,
        },
    );
}
