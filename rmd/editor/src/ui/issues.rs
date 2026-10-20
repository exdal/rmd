use core::path::TreePath;
use std::collections::HashMap;

use dear_imgui_rs::{
    Direction, ListClipper, SelectableFlags, TableColumnFlags, TableFlags, TableSizingPolicy, Ui, WindowKey,
    WindowKeyError,
};
use editor::{
    document::{DocumentId, PrefabInstanceId},
    issues,
};

use super::{
    common::{
        RowSelection, button_width, centered_note, checkbox_width, focus_window_on_hover, overflow_scroll,
        same_line_if_fits, table_min_width,
    },
    find::JumpTarget,
    search::draw_type_path_search,
    welcome::codebase_relative,
};
use crate::{
    session::{MapIssueGroup, Session},
    settings::Settings,
};

#[derive(Default)]
struct Replacement {
    query: String,
    target: Option<TreePath>,
}

enum Action {
    Jump(JumpTarget),
    Step {
        document: DocumentId,
        path: TreePath,
        forward: bool,
    },
    Replace {
        document: DocumentId,
        path: TreePath,
        instances: Vec<PrefabInstanceId>,
        target: TreePath,
    },
    Delete {
        document: DocumentId,
        path: TreePath,
        instances: Vec<PrefabInstanceId>,
    },
}

#[derive(Default)]
pub(super) struct IssuesOutput {
    pub jump: Option<JumpTarget>,
    pub copy: Option<String>,
}

pub(super) struct IssuesPanel {
    window: WindowKey,
    focus_requested: bool,
    reveal: Option<DocumentId>,
    filter: String,
    all_maps: bool,
    current_level_only: bool,
    selection_document: Option<DocumentId>,
    selected_group: Option<(DocumentId, TreePath)>,
    rows: RowSelection<PrefabInstanceId>,
    current: Option<JumpTarget>,
    replacements: HashMap<(DocumentId, TreePath), Replacement>,
    visible: bool,
}

impl IssuesPanel {
    pub(super) fn new() -> Result<Self, WindowKeyError> {
        Ok(Self {
            window: WindowKey::new("map-issues", "Map Issues")?,
            focus_requested: false,
            reveal: None,
            filter: String::new(),
            all_maps: false,
            current_level_only: false,
            selection_document: None,
            selected_group: None,
            rows: RowSelection::default(),
            current: None,
            replacements: HashMap::new(),
            visible: false,
        })
    }

    pub(super) fn window(&self) -> &WindowKey {
        &self.window
    }

    pub(super) fn request(&mut self, document: Option<DocumentId>) {
        self.focus_requested = true;
        self.reveal = document;
        if document.is_some() {
            self.filter.clear();
            self.current_level_only = false;
        }
    }

    pub(super) fn has_focus_request(&self) -> bool {
        self.focus_requested
    }

    #[cfg(test)]
    pub(super) fn visible(&self) -> bool {
        self.visible
    }

    pub(super) fn draw(
        &mut self, ui: &Ui, session: &mut Session, settings: &Settings, open: &mut bool,
    ) -> IssuesOutput {
        let mut output = IssuesOutput::default();
        let is_focus_requested = std::mem::take(&mut self.focus_requested);
        let window = self.window.clone();
        self.visible = false;
        if !*open {
            return output;
        }

        self.replacements
            .retain(|(id, path), _| session.map_issues(*id).iter().any(|group| group.path == *path));
        let count = session
            .state
            .document_ids()
            .into_iter()
            .filter(|id| self.all_maps || Some(*id) == session.state.active())
            .map(|id| {
                session
                    .map_issues(id)
                    .iter()
                    .map(|group| group.instances.len())
                    .sum::<usize>()
            })
            .sum::<usize>();
        let title = if count == 0 {
            String::from("Map Issues")
        } else {
            format!("Map Issues ({count})")
        };
        let mut action = None;
        ui.window(window.label(title))
            .opened(open)
            .focused(is_focus_requested)
            .build(|| {
                self.visible = true;
                if settings.focus_windows_on_hover {
                    focus_window_on_hover(ui);
                }

                self.draw_contents(ui, session, &mut action, &mut output);
            });
        if let Some(action) = action {
            output.jump = self.apply(session, action);
        }

        output
    }

    fn select_group(&mut self, group: Option<(DocumentId, TreePath)>) {
        if self.selected_group != group {
            self.rows.clear();
            self.current = None;
            self.selected_group = group;
        }
    }

    fn draw_contents(&mut self, ui: &Ui, session: &Session, action: &mut Option<Action>, output: &mut IssuesOutput) {
        let active = session.state.active();
        if !self.all_maps && self.selection_document != active {
            self.select_group(None);
        }

        self.selection_document = active;
        if session.tree().is_none() {
            centered_note(ui, "Load a codebase to check map paths");
            return;
        }

        if self.all_maps {
            ui.text_disabled("All open maps");
        } else if let Some(document) = active.and_then(|id| session.state.document(id)) {
            ui.text_disabled(document.path.as_ref().map_or_else(
                || document.title(),
                |path| {
                    session
                        .codebase_dir()
                        .map_or_else(|| path.display().to_string(), |base| codebase_relative(base, path))
                },
            ));
        }

        ui.set_next_item_width(-f32::MIN_POSITIVE);
        ui.input_text("##issues-filter", &mut self.filter)
            .hint("Filter unknown paths")
            .build();
        ui.checkbox("All open maps", &mut self.all_maps);
        same_line_if_fits(ui, checkbox_width(ui, "Current level only"));
        ui.checkbox("Current level only", &mut self.current_level_only);
        if active.is_none() && !self.all_maps {
            centered_note(ui, "No map selected");
            return;
        }

        let mut maps = session
            .state
            .document_ids()
            .into_iter()
            .filter(|id| self.all_maps || Some(*id) == active)
            .map(|id| (id, session.state.document(id).unwrap().title(), session.map_issues(id)))
            .collect::<Vec<_>>();
        maps.sort_by(|left, right| left.1.cmp(&right.1).then_with(|| left.0.cmp(&right.0)));
        let query = self.filter.trim().to_ascii_lowercase();
        let groups = maps
            .iter()
            .flat_map(|(id, title, groups)| groups.iter().map(move |group| (*id, title, group)))
            .filter(|(id, title, group)| {
                matches_filter(&query, title, &group.path)
                    && (!self.current_level_only
                        || group.instances.iter().any(|instance| {
                            session
                                .state
                                .document(*id)
                                .unwrap()
                                .instance_location(*instance)
                                .is_some_and(|location| location.coord.z == session.state.document(*id).unwrap().z)
                        }))
            })
            .collect::<Vec<_>>();
        let reveal = self.reveal.take();
        if let Some((id, _, group)) = reveal.and_then(|reveal| groups.iter().find(|(id, ..)| *id == reveal)) {
            self.select_group(Some((*id, group.path.clone())));
        } else if !groups
            .iter()
            .any(|(id, _, group)| self.selected_group.as_ref() == Some(&(*id, group.path.clone())))
        {
            self.select_group(groups.first().map(|(id, _, group)| (*id, group.path.clone())));
        }

        if groups.is_empty() {
            self.select_group(None);
            let is_any_issue = maps.iter().any(|(_, _, groups)| !groups.is_empty());
            centered_note(
                ui,
                if is_any_issue {
                    "No matching issues"
                } else {
                    "No unknown type paths"
                },
            );
            return;
        }

        ui.text_disabled(format!("{} unknown paths", groups.len()));
        let height = (ui.content_region_avail()[1] * 0.3).max(ui.frame_height_with_spacing() * 3.0);
        let count_width = ui.calc_text_size("Instances")[0];
        let (scroll, inner_width) = overflow_scroll(
            ui,
            table_min_width(ui, &[count_width], if self.all_maps { 2 } else { 1 }),
        );
        let mut table = ui
            .table("issue-paths")
            .flags(
                TableFlags::ROW_BG
                    | TableFlags::BORDERS_INNER_V
                    | TableFlags::BORDERS_OUTER
                    | TableFlags::RESIZABLE
                    | TableFlags::HIDEABLE
                    | TableFlags::SCROLL_Y
                    | scroll,
            )
            .sizing_policy(TableSizingPolicy::StretchProp)
            .outer_size([0.0, height])
            .inner_width(inner_width)
            .freeze(1, 1)
            .headers(true)
            .column("Unknown path")
            .flags(TableColumnFlags::NO_HIDE)
            .weight(2.0)
            .done()
            .column("Instances")
            .width(count_width)
            .done();
        if self.all_maps {
            table = table.column("Map").weight(1.0).done();
        }

        let mut selected = None;
        table.build(|ui| {
            for index in ListClipper::new(groups.len()).begin(ui).iter() {
                let (id, title, group) = groups[index];
                let _row = ui.push_id(index);
                ui.table_next_row();
                ui.table_next_column();
                if ui
                    .selectable_config(group.path.to_string())
                    .selected(self.selected_group.as_ref() == Some(&(id, group.path.clone())))
                    .flags(SelectableFlags::SPAN_ALL_COLUMNS)
                    .build()
                {
                    selected = Some((id, group.path.clone()));
                }

                ui.set_item_tooltip(group.path.to_string());
                ui.table_next_column();
                ui.text(group.instances.len().to_string());
                if self.all_maps {
                    ui.table_next_column();
                    ui.text(title);
                }
            }
        });
        if let Some(selected) = selected {
            self.select_group(Some(selected));
        }

        let Some((id, _, group)) = groups
            .iter()
            .find(|(id, _, group)| self.selected_group.as_ref() == Some(&(*id, group.path.clone())))
        else {
            return;
        };
        ui.separator();
        let replacement = self.replacements.entry((*id, group.path.clone())).or_default();
        draw_occurrences(
            ui,
            session,
            *id,
            group,
            replacement,
            &mut self.rows,
            self.current_level_only,
            action,
            output,
        );
    }

    fn apply(&mut self, session: &mut Session, action: Action) -> Option<JumpTarget> {
        match action {
            Action::Jump(target) => {
                session
                    .state
                    .document(target.document)?
                    .instance_location(target.instance)?;
                self.current = Some(target);
                self.rows.select(target.instance);
                Some(target)
            },
            Action::Step {
                document,
                path,
                forward,
            } => {
                let groups = session.map_issues(document);
                let group = groups.iter().find(|group| group.path == path)?;
                let document_state = session.state.document(document)?;
                let instances = group
                    .instances
                    .iter()
                    .copied()
                    .filter(|instance| {
                        !self.current_level_only
                            || document_state
                                .instance_location(*instance)
                                .is_some_and(|location| location.coord.z == document_state.z)
                    })
                    .collect::<Vec<_>>();
                if instances.is_empty() {
                    return None;
                }

                let current = self
                    .current
                    .filter(|target| target.document == document)
                    .and_then(|target| instances.iter().position(|id| *id == target.instance));
                let next = match (current, forward) {
                    (None, true) => 0,
                    (None, false) => instances.len() - 1,
                    (Some(index), true) => (index + 1) % instances.len(),
                    (Some(index), false) => (index + instances.len() - 1) % instances.len(),
                };
                let target = JumpTarget {
                    document,
                    instance: instances[next],
                };
                self.current = Some(target);
                self.rows.select(target.instance);
                Some(target)
            },
            Action::Replace {
                document,
                path,
                instances,
                target,
            } => {
                session.replace_issue_instances(document, &path, &instances, &target);
                None
            },
            Action::Delete {
                document,
                path,
                instances,
            } => {
                session.delete_issue_instances(document, &path, &instances);
                None
            },
        }
    }
}

fn matches_filter(query: &str, title: &str, path: &TreePath) -> bool {
    title.to_ascii_lowercase().contains(query) || path.to_string().to_ascii_lowercase().contains(query)
}

#[allow(clippy::too_many_arguments)]
fn draw_occurrences(
    ui: &Ui, session: &Session, document: DocumentId, group: &MapIssueGroup, replacement: &mut Replacement,
    selection: &mut RowSelection<PrefabInstanceId>, level_only: bool, action: &mut Option<Action>,
    output: &mut IssuesOutput,
) {
    let map = session.state.document(document).unwrap();
    let rows = group
        .instances
        .iter()
        .filter_map(|instance| map.instance_location(*instance).map(|location| (*instance, location)))
        .filter(|(_, location)| !level_only || location.coord.z == map.z)
        .collect::<Vec<_>>();
    let order = rows.iter().map(|(instance, _)| *instance).collect::<Vec<_>>();
    let selected = selection.among(order.iter().copied());
    let (targets, suffix) = if selected.is_empty() {
        (order.clone(), String::from(" for all"))
    } else {
        let suffix = format!(" ({} selected)", selected.len());
        (selected, suffix)
    };
    let editable = session.editable_issue_instances(document, &group.path, &targets);
    ui.text_wrapped(group.path.to_string());
    if editable.len() != targets.len() {
        ui.text_disabled(format!(
            "{} of {} editable (read-only map or area focus)",
            editable.len(),
            targets.len()
        ));
    }

    if ui.button("Copy path") {
        output.copy = Some(group.path.to_string());
    }

    same_line_if_fits(ui, ui.frame_height() * 2.0 + ui.clone_style().item_spacing()[0]);
    if ui.arrow_button("##previous", Direction::Left) {
        *action = Some(Action::Step {
            document,
            path: group.path.clone(),
            forward: false,
        });
    }

    ui.set_item_tooltip("Previous occurrence");
    ui.same_line();
    if ui.arrow_button("##next", Direction::Right) {
        *action = Some(Action::Step {
            document,
            path: group.path.clone(),
            forward: true,
        });
    }

    ui.set_item_tooltip("Next occurrence");
    let tree = session.tree().unwrap();
    let is_classifiable = issues::has_replacement_kind(tree, &group.path);
    ui.set_next_item_width(-f32::MIN_POSITIVE);
    let disabled = ui.begin_disabled_with_cond(!is_classifiable || editable.is_empty());
    let preview = replacement
        .target
        .as_ref()
        .map_or_else(|| String::from("Choose replacement..."), ToString::to_string);
    if let Some(_combo) = ui.begin_combo("##issue-replacement", preview)
        && let Some(target) = draw_type_path_search(
            ui,
            Some(tree),
            &mut replacement.query,
            "##replacement-path",
            50,
            |tree, path| issues::can_replace(tree, &group.path, path),
            |_| true,
        )
    {
        replacement.target = Some(target);
        ui.close_current_popup();
    }

    drop(disabled);
    if !is_classifiable {
        ui.text_wrapped("Cannot determine whether this path is an object, turf, or area. Replacement is unavailable.");
    }

    let target = replacement
        .target
        .as_ref()
        .filter(|target| issues::can_replace(tree, &group.path, target));
    let replace = format!("Replace{suffix}");
    let delete = format!("Delete{suffix}");
    if ui.with_disabled_if(editable.is_empty() || target.is_none(), || ui.button(&replace)) {
        *action = Some(Action::Replace {
            document,
            path: group.path.clone(),
            instances: editable.clone(),
            target: target.unwrap().clone(),
        });
    }

    ui.set_item_tooltip("Change the type path and keep each instance's overrides");
    same_line_if_fits(ui, button_width(ui, &delete));
    if ui.with_disabled_if(editable.is_empty(), || ui.button(&delete)) {
        *action = Some(Action::Delete {
            document,
            path: group.path.clone(),
            instances: editable.clone(),
        });
    }

    let tile_width = ui.calc_text_size("000, 000, 00")[0];
    let buttons_width = button_width(ui, "Replace") + button_width(ui, "Delete") + ui.clone_style().item_spacing()[0];
    let (scroll, inner_width) = overflow_scroll(ui, table_min_width(ui, &[tile_width, buttons_width], 0));
    let all_editable = session.editable_issue_instances(document, &group.path, &order);
    ui.table("issue-occurrences")
        .flags(
            TableFlags::ROW_BG
                | TableFlags::BORDERS_INNER_V
                | TableFlags::BORDERS_OUTER
                | TableFlags::RESIZABLE
                | TableFlags::HIDEABLE
                | TableFlags::SCROLL_Y
                | scroll,
        )
        .sizing_policy(TableSizingPolicy::StretchProp)
        .inner_width(inner_width)
        .freeze(1, 1)
        .headers(true)
        .column("Tile")
        .weight(1.0)
        .flags(TableColumnFlags::NO_HIDE)
        .done()
        .column("##actions")
        .width(buttons_width)
        .flags(TableColumnFlags::NO_HIDE)
        .done()
        .build(|ui| {
            for index in ListClipper::new(rows.len()).begin(ui).iter() {
                let (instance, location) = rows[index];
                let _instance = ui.push_id(index);
                ui.table_next_row();
                ui.table_next_column();
                let coord = location.coord;
                if ui
                    .selectable_config(format!("{}, {}, {}", coord.x, coord.y, coord.z))
                    .selected(selection.contains(&instance))
                    .flags(SelectableFlags::SPAN_ALL_COLUMNS | SelectableFlags::ALLOW_OVERLAP)
                    .build()
                    && selection.click(ui, &order, index)
                {
                    *action = Some(Action::Jump(JumpTarget { document, instance }));
                }

                if let Some(_menu) = ui.begin_popup_context_item() {
                    if ui.menu_item("Jump to tile") {
                        *action = Some(Action::Jump(JumpTarget { document, instance }));
                    }

                    if ui.menu_item("Copy coordinates") {
                        output.copy = Some(format!("{}, {}, {}", coord.x, coord.y, coord.z));
                    }

                    if ui.menu_item("Copy path") {
                        output.copy = Some(group.path.to_string());
                    }
                }

                ui.table_next_column();
                let is_editable = all_editable.binary_search(&instance).is_ok();
                if ui.with_disabled_if(!is_editable || target.is_none(), || ui.small_button("Replace")) {
                    *action = Some(Action::Replace {
                        document,
                        path: group.path.clone(),
                        instances: vec![instance],
                        target: target.unwrap().clone(),
                    });
                }

                ui.same_line();
                if ui.with_disabled_if(!is_editable, || ui.small_button("Delete")) {
                    *action = Some(Action::Delete {
                        document,
                        path: group.path.clone(),
                        instances: vec![instance],
                    });
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        session::fixtures::unknown_paths_session,
        ui::{IMGUI_CONTEXT, UiState, fixtures::rectangle_context},
    };

    #[test]
    fn occurrence_navigation_wraps_and_jumps_to_the_exact_unknown_instance() {
        let mut session = unknown_paths_session();
        let document = session.state.active().unwrap();
        let groups = session.map_issues(document);
        let group = &groups[0];
        let mut panel = IssuesPanel::new().unwrap();
        let visited = (0..5)
            .map(|_| {
                panel
                    .apply(
                        &mut session,
                        Action::Step {
                            document,
                            path: group.path.clone(),
                            forward: true,
                        },
                    )
                    .unwrap()
                    .instance
            })
            .collect::<Vec<_>>();
        assert_eq!(
            visited,
            [
                group.instances[0],
                group.instances[1],
                group.instances[2],
                group.instances[3],
                group.instances[0]
            ]
        );
        let target = panel
            .apply(
                &mut session,
                Action::Step {
                    document,
                    path: group.path.clone(),
                    forward: false,
                },
            )
            .unwrap();
        assert_eq!(target.instance, group.instances[3]);
        let mut state = UiState::new(false).unwrap();
        state.jump_to_instance(&mut session, target);
        assert_eq!(session.z(), 2);
        assert_eq!(session.selected_instance(), Some(target.instance));
        panel.apply(
            &mut session,
            Action::Delete {
                document,
                path: group.path.clone(),
                instances: vec![target.instance],
            },
        );
        assert_eq!(
            panel
                .apply(
                    &mut session,
                    Action::Step {
                        document,
                        path: group.path.clone(),
                        forward: true
                    }
                )
                .unwrap()
                .instance,
            group.instances[0]
        );
        session.close_map(document);
        assert!(
            panel
                .apply(
                    &mut session,
                    Action::Step {
                        document,
                        path: group.path.clone(),
                        forward: true
                    }
                )
                .is_none()
        );
    }

    #[test]
    fn filtering_matches_map_names_and_paths_and_loading_clears_a_hidden_filter() {
        let path = TreePath::parse("/obj/Removed_Table");
        assert!(matches_filter("removed", "Station.dmm", &path));
        assert!(matches_filter("station", "Station.dmm", &path));
        assert!(!matches_filter("missing", "Station.dmm", &path));
        let session = unknown_paths_session();
        let mut panel = IssuesPanel::new().unwrap();
        panel.filter = "would hide the new map".into();
        panel.request(session.state.active());
        assert!(panel.filter.is_empty());
        assert!(panel.has_focus_request());
    }

    #[test]
    fn panel_renders_live_results_repairs_and_the_empty_state() {
        let _guard = IMGUI_CONTEXT.lock().unwrap();
        let mut context = rectangle_context();
        let mut session = unknown_paths_session();
        let document = session.state.active().unwrap();
        let mut panel = IssuesPanel::new().unwrap();
        panel.request(Some(document));
        let mut open = true;
        for _ in 0..3 {
            panel.draw(context.frame(), &mut session, &Settings::default(), &mut open);
            assert!(!panel.has_focus_request(), "a draw consumes the one-shot focus request");
            assert!(context.render_legacy().valid());
        }

        assert!(panel.visible());
        for group in session.map_issues(document).iter() {
            panel.apply(
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
        let _guard = IMGUI_CONTEXT.lock().unwrap();
        let mut context = rectangle_context();
        let mut session = unknown_paths_session();
        let first = session.state.active().unwrap();
        let map = session.state.document(first).unwrap().to_map();
        session.apply_map(crate::loader::LoadedMap {
            path: "other-map.dmm".into(),
            map,
            z: 1,
            errors: Vec::new(),
            repo: None,
            conflict: None,
        });
        let second = session.state.active().unwrap();
        let mut panel = IssuesPanel::new().unwrap();
        let mut open = true;
        panel.draw(context.frame(), &mut session, &Settings::default(), &mut open);
        assert!(context.render_legacy().valid());
        assert_eq!(panel.selected_group.as_ref().unwrap().0, second);
        panel.all_maps = true;
        panel.select_group(Some((first, session.map_issues(first)[0].path.clone())));
        panel.draw(context.frame(), &mut session, &Settings::default(), &mut open);
        assert!(context.render_legacy().valid());
        assert_eq!(panel.selected_group.as_ref().unwrap().0, first);
        session.close_map(first);
        panel.draw(context.frame(), &mut session, &Settings::default(), &mut open);
        assert!(context.render_legacy().valid());
        assert_eq!(panel.selected_group.as_ref().unwrap().0, second);
    }

    #[test]
    fn occurrence_navigation_respects_the_current_level_filter() {
        let mut session = unknown_paths_session();
        let document = session.state.active().unwrap();
        let groups = session.map_issues(document);
        let group = &groups[0];
        let mut panel = IssuesPanel::new().unwrap();
        panel.current_level_only = true;
        let visited = (0..3)
            .map(|_| {
                panel
                    .apply(
                        &mut session,
                        Action::Step {
                            document,
                            path: group.path.clone(),
                            forward: true,
                        },
                    )
                    .unwrap()
                    .instance
            })
            .collect::<Vec<_>>();
        assert_eq!(visited, [group.instances[0], group.instances[1], group.instances[0]]);
    }
}
