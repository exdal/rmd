use super::*;
use ui::testing::{CONTEXT_LOCK, rectangle_context};

use base::types::Value;
use prefab::Prefab;

use crate::{
    frontend::UiState,
    session::fixtures::{issue_vars_session, unknown_paths_session},
};

fn expand_all(panel: &mut IssuesPanel, session: &Session) {
    for row in panel.rows(session) {
        if let Row::Group { document, key, .. } = row {
            panel.expanded.insert((document, key));
        }
    }
}

fn documents(rows: &[Row]) -> HashSet<DocumentId> {
    rows.iter()
        .map(|row| match row {
            Row::Group { document, .. } | Row::Tile { document, .. } | Row::Instance { document, .. } => *document,
        })
        .collect()
}

#[test]
fn jumping_reaches_the_exact_unknown_instance_and_stops_after_the_issue_is_gone() {
    let mut session = unknown_paths_session();
    let document = session.state.active().unwrap();
    let groups = session.map_issues(document);
    let group = &groups[0];
    let instance = group.instances[3];
    let target = apply(&mut session, Action::Jump(JumpTarget { document, instance })).unwrap();
    let mut state = UiState::new(false).unwrap();
    state.jump_to_instance(&mut session, target);
    assert_eq!(session.z(), 2);
    assert_eq!(session.picked_instance(), Some(instance));
    apply(
        &mut session,
        Action::Delete {
            document,
            path: group.path.clone(),
            instances: vec![instance],
        },
    );
    assert!(apply(&mut session, Action::Jump(JumpTarget { document, instance })).is_none());
    session.close_map(document);
    assert!(apply(&mut session, Action::Jump(target)).is_none());
}

#[test]
fn filtering_matches_map_names_and_paths_and_loading_clears_a_hidden_filter() {
    assert!(matches_filter("removed", "Station.dmm", "/obj/Removed_Table"));
    assert!(matches_filter("station", "Station.dmm", "/obj/Removed_Table"));
    assert!(!matches_filter("missing", "Station.dmm", "/obj/Removed_Table"));
    let session = unknown_paths_session();
    let mut panel = IssuesPanel::new().unwrap();
    panel.filter = "would hide the new map".into();
    panel.request(session.state.active());
    assert!(panel.filter.is_empty());
    assert!(panel.has_focus_request());
}

#[test]
fn rows_list_collapsed_groups_and_expand_into_tiles() {
    let session = unknown_paths_session();
    let document = session.state.active().unwrap();
    let groups = session.map_issues(document);
    let mut panel = IssuesPanel::new().unwrap();
    let rows = panel.rows(&session);
    assert_eq!(rows.len(), 1 + groups.len());
    assert!(rows.iter().all(|row| matches!(row, Row::Group { .. })));
    assert_eq!(
        rows[0],
        Row::Group {
            document,
            key: IssueKey::MissingBase,
            count: session.missing_base_tiles(document).len()
        }
    );

    panel
        .expanded
        .insert((document, IssueKey::UnknownPath(groups[0].path.clone())));
    let rows = panel.rows(&session);
    assert_eq!(rows.len(), 1 + groups.len() + groups[0].instances.len());
    assert!(matches!(&rows[2], Row::Tile { instance: Some(_), .. }));

    panel.filter = "removed_floor".into();
    assert_eq!(panel.rows(&session).len(), 1);
}

#[test]
fn current_level_only_drops_other_levels_and_hides_emptied_groups() {
    let mut session = unknown_paths_session();
    let document = session.state.active().unwrap();
    let level = session.state.document(document).unwrap().z;
    let mut panel = IssuesPanel::new().unwrap();
    expand_all(&mut panel, &session);
    let all = panel.rows(&session);
    panel.current_level_only = true;
    let filtered = panel.rows(&session);
    assert!(filtered.len() < all.len());
    assert!(filtered.iter().all(|row| match row {
        Row::Tile { coord, .. } | Row::Instance { coord, .. } => coord.z == level,
        Row::Group { .. } => true,
    }));

    let on_level = session
        .missing_base_tiles(document)
        .iter()
        .copied()
        .filter(|coord| coord.z == level)
        .collect::<Vec<_>>();
    assert!(session.fill_missing_bases(document, &on_level));
    assert!(panel.rows(&session).iter().all(|row| !matches!(
        row,
        Row::Group {
            key: IssueKey::MissingBase,
            ..
        }
    )));
    panel.current_level_only = false;
    assert!(panel.rows(&session).iter().any(|row| matches!(
        row,
        Row::Group {
            key: IssueKey::MissingBase,
            ..
        }
    )));
}

#[test]
fn panel_renders_live_results_repairs_and_the_empty_state() {
    let _guard = CONTEXT_LOCK.lock().unwrap();
    let mut context = rectangle_context();
    let mut session = unknown_paths_session();
    let document = session.state.active().unwrap();
    let mut panel = IssuesPanel::new().unwrap();
    panel.request(Some(document));
    expand_all(&mut panel, &session);
    let mut open = true;
    for _ in 0..3 {
        panel.draw(context.frame(), &mut session, &Settings::default(), &mut open);
        assert!(!panel.has_focus_request(), "a draw consumes the one-shot focus request");
        assert!(context.render_legacy().valid());
    }

    assert!(panel.visible());
    for group in session.map_issues(document).iter() {
        apply(
            &mut session,
            Action::Delete {
                document,
                path: group.path.clone(),
                instances: group.instances.clone(),
            },
        );
    }

    assert!(session.map_issues(document).is_empty());
    panel.draw(context.frame(), &mut session, &Settings::default(), &mut open);
    assert!(context.render_legacy().valid());
    assert!(
        open && panel.visible(),
        "repairing the last issue leaves the panel open"
    );
}

#[test]
fn panel_follows_the_active_map_and_keeps_other_maps_available() {
    let mut session = unknown_paths_session();
    let first = session.state.active().unwrap();
    let map = session.state.document(first).unwrap().to_map();
    session.apply_map(crate::environment::worker::LoadedMap {
        path: "other-map.dmm".into(),
        map,
        z: 1,
        errors: Vec::new(),
        repo: None,
        conflict: None,
    });
    let second = session.state.active().unwrap();
    let mut panel = IssuesPanel::new().unwrap();
    assert_eq!(documents(&panel.rows(&session)), HashSet::from([second]));
    panel.all_maps = true;
    assert_eq!(documents(&panel.rows(&session)), HashSet::from([first, second]));
    session.close_map(first);
    assert_eq!(documents(&panel.rows(&session)), HashSet::from([second]));
}

fn tile_prefab(path: &str, vars: &[(&str, Value)]) -> Prefab {
    let mut prefab = Prefab::new(TreePath::parse(path));
    for (name, value) in vars {
        prefab.set_var((*name).into(), value.clone());
    }

    prefab
}

fn problem_session() -> Session {
    let bogus = [("bogus", Value::Num(1.0))];
    issue_vars_session(vec![
        vec![
            tile_prefab("/turf/open/floor", &[]),
            tile_prefab("/area/station", &[]),
            tile_prefab("/area/station/extra", &[]),
            tile_prefab("/obj/base", &bogus),
        ],
        vec![
            tile_prefab("/turf/open/floor", &[]),
            tile_prefab("/area/station", &[]),
            tile_prefab("/obj/gone", &[]),
            tile_prefab("/obj/base/child", &bogus),
        ],
    ])
}

fn instance_labels(panel: &IssuesPanel, session: &Session) -> Vec<String> {
    panel
        .rows(session)
        .into_iter()
        .filter_map(|row| match row {
            Row::Instance { label, .. } => Some(label),
            _ => None,
        })
        .collect()
}

#[test]
fn tile_problem_groups_are_ordered_and_rows_are_labeled_by_path() {
    let session = problem_session();
    let mut panel = IssuesPanel::new().unwrap();
    expand_all(&mut panel, &session);

    let labels = panel
        .rows(&session)
        .iter()
        .filter_map(|row| match row {
            Row::Group { key, .. } => Some(key.label()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(labels, ["Multiple areas on a tile", "/obj/gone", "Undefined var bogus"]);
    assert_eq!(
        instance_labels(&panel, &session),
        [
            "1, 1, 1 \u{2014} /area/station",
            "1, 1, 1 \u{2014} /area/station/extra",
            "1, 1, 1 \u{2014} /obj/base",
            "2, 1, 1 \u{2014} /obj/base/child",
        ]
    );

    panel.filter = "obj/base/child".into();
    assert_eq!(instance_labels(&panel, &session), ["2, 1, 1 \u{2014} /obj/base/child"]);
    panel.filter = "undefined var".into();
    assert_eq!(instance_labels(&panel, &session).len(), 2);
}

#[test]
fn deleting_an_area_and_removing_vars_are_single_undo_steps_that_respect_read_only() {
    let mut session = problem_session();
    let document = session.state.active().unwrap();
    let mut panel = IssuesPanel::new().unwrap();
    expand_all(&mut panel, &session);
    let area = session.multiple_area_instances(document)[1];

    apply(
        &mut session,
        Action::DeleteInstance {
            document,
            instance: area,
        },
    );
    assert!(session.multiple_area_instances(document).is_empty());
    assert!(session.undo());
    assert_eq!(session.multiple_area_instances(document).len(), 2);

    let group = &session.undefined_vars(document)[0];
    let (name, instances) = (group.name.clone(), group.instances.clone());
    apply(
        &mut session,
        Action::RemoveVar {
            document,
            instances: instances[..1].to_vec(),
            name: name.clone(),
        },
    );
    assert_eq!(session.undefined_vars(document)[0].instances.len(), 1);
    assert!(session.undo());
    apply(
        &mut session,
        Action::RemoveVar {
            document,
            instances,
            name,
        },
    );
    assert!(session.undefined_vars(document).is_empty());
    assert!(session.undo());
    assert_eq!(session.undefined_vars(document)[0].instances.len(), 2);

    session.state.document_mut(document).unwrap().set_read_only(true);
    assert!(!document_allows(&session, document, Coord::new(1, 1, 1)));
    apply(
        &mut session,
        Action::DeleteInstance {
            document,
            instance: area,
        },
    );
    assert_eq!(session.multiple_area_instances(document).len(), 2);
}

#[test]
fn tile_problem_rows_render() {
    let _guard = CONTEXT_LOCK.lock().unwrap();
    let mut context = rectangle_context();
    let mut session = problem_session();
    let mut panel = IssuesPanel::new().unwrap();
    panel.request(session.state.active());
    expand_all(&mut panel, &session);
    let mut open = true;
    for _ in 0..3 {
        panel.draw(context.frame(), &mut session, &Settings::default(), &mut open);
        assert!(context.render_legacy().valid());
    }

    assert!(panel.visible());
}
