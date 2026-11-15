use base::path::TreePath;
use std::collections::HashSet;

use dear_imgui_rs::{
    Condition, ListClipper, SelectableFlags, TableColumnFlags, TableFlags, TableSizingPolicy, Ui, WindowKey,
    WindowKeyError,
};
use dmm::Coord;
use objtree::ObjectTree;
use ui::widgets::{
    button_width, centered_note, checkbox_width, focus_window_on_hover, overflow_scroll, same_line_if_fits,
    table_min_width,
};

use crate::{
    document::{DocumentId, PrefabInstanceId},
    frontend::welcome::codebase_relative,
    issues,
    object_tree::panel::type_search::draw_type_path_search,
    search::panel::JumpTarget,
    session::Session,
    settings::Settings,
};

enum Action {
    Jump(JumpTarget),
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
    FillBases {
        document: DocumentId,
        tiles: Vec<Coord>,
    },
}

#[derive(Default)]
pub(crate) struct IssuesOutput {
    pub jump: Option<JumpTarget>,
    pub tile: Option<(DocumentId, Coord)>,
    pub copy: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum IssueKey {
    MissingBase,
    UnknownPath(TreePath),
}

impl IssueKey {
    fn label(&self) -> String {
        match self {
            Self::MissingBase => String::from("Missing turf or area"),
            Self::UnknownPath(path) => path.to_string(),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Row {
    Group {
        document: DocumentId,
        key: IssueKey,
        count: usize,
    },
    Tile {
        document: DocumentId,
        coord: Coord,
        instance: Option<PrefabInstanceId>,
    },
}

pub(crate) struct IssuesPanel {
    window: WindowKey,
    focus_requested: bool,
    filter: String,
    all_maps: bool,
    current_level_only: bool,
    expanded: HashSet<(DocumentId, IssueKey)>,
    replace_query: String,
    visible: bool,
}

impl IssuesPanel {
    pub(crate) fn new() -> Result<Self, WindowKeyError> {
        Ok(Self {
            window: WindowKey::new("map-issues", "Map Issues")?,
            focus_requested: false,
            filter: String::new(),
            all_maps: false,
            current_level_only: false,
            expanded: HashSet::new(),
            replace_query: String::new(),
            visible: false,
        })
    }

    pub(crate) fn window(&self) -> &WindowKey {
        &self.window
    }

    pub(crate) fn request(&mut self, document: Option<DocumentId>) {
        self.focus_requested = true;
        if document.is_some() {
            self.filter.clear();
            self.current_level_only = false;
        }
    }

    pub(crate) fn has_focus_request(&self) -> bool {
        self.focus_requested
    }

    #[cfg(test)]
    pub(crate) fn visible(&self) -> bool {
        self.visible
    }

    pub(crate) fn draw(
        &mut self, ui: &Ui, session: &mut Session, settings: &Settings, open: &mut bool,
    ) -> IssuesOutput {
        let mut output = IssuesOutput::default();
        let is_focus_requested = std::mem::take(&mut self.focus_requested);
        let window = self.window.clone();
        self.visible = false;
        if !*open {
            return output;
        }

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
                    + session.missing_base_tiles(id).len()
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
            output.jump = apply(session, action);
        }

        output
    }

    fn draw_contents(&mut self, ui: &Ui, session: &Session, action: &mut Option<Action>, output: &mut IssuesOutput) {
        let Some(tree) = session.tree() else {
            centered_note(ui, "Load a codebase to check map paths");
            return;
        };

        let active = session.state.active();
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
            .hint("Filter issues")
            .build();
        ui.checkbox("All open maps", &mut self.all_maps);
        same_line_if_fits(ui, checkbox_width(ui, "Current level only"));
        ui.checkbox("Current level only", &mut self.current_level_only);
        if active.is_none() && !self.all_maps {
            centered_note(ui, "No map selected");
            return;
        }

        let rows = self.rows(session);
        if rows.is_empty() {
            let is_filtered = !self.filter.trim().is_empty() || self.current_level_only;
            centered_note(ui, if is_filtered { "No matching issues" } else { "No issues" });
            return;
        }

        let count_width = ui.calc_text_size("00000")[0];
        let spacing = ui.clone_style().item_spacing()[0];
        let actions_width = button_width(ui, "Replace…") + button_width(ui, "Delete all") + spacing;
        let (scroll, inner_width) = overflow_scroll(
            ui,
            table_min_width(ui, &[count_width, actions_width], if self.all_maps { 2 } else { 1 }),
        );
        let mut table = ui
            .table("issues")
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
            .column("Issue")
            .flags(TableColumnFlags::NO_HIDE)
            .weight(2.0)
            .done()
            .column("#")
            .width(count_width)
            .done();
        if self.all_maps {
            table = table.column("Map").weight(1.0).done();
        }

        table
            .column("Action")
            .width(actions_width)
            .flags(TableColumnFlags::NO_HIDE)
            .done()
            .build(|ui| {
                for index in ListClipper::new(rows.len()).begin(ui).iter() {
                    let _row = ui.push_id(index);
                    ui.table_next_row();
                    ui.table_next_column();
                    match &rows[index] {
                        Row::Group { document, key, count } => {
                            self.draw_group(ui, session, tree, *document, key, *count, action, output);
                        },
                        Row::Tile {
                            document,
                            coord,
                            instance,
                        } => self.draw_tile(ui, session, tree, *document, *coord, *instance, action, output),
                    }
                }
            });
    }

    fn rows(&self, session: &Session) -> Vec<Row> {
        let active = session.state.active();
        let query = self.filter.trim().to_ascii_lowercase();
        let mut maps = session
            .state
            .document_ids()
            .into_iter()
            .filter(|id| self.all_maps || Some(*id) == active)
            .filter_map(|id| session.state.document(id).map(|document| (id, document.title())))
            .collect::<Vec<_>>();
        maps.sort_by(|left, right| left.1.cmp(&right.1).then_with(|| left.0.cmp(&right.0)));

        let mut rows = Vec::new();
        for (document, title) in maps {
            let groups = session.map_issues(document);
            let keys = std::iter::once(IssueKey::MissingBase)
                .chain(groups.iter().map(|group| IssueKey::UnknownPath(group.path.clone())));
            for key in keys {
                let tiles = self.tiles(session, document, &key);
                if tiles.is_empty() || !matches_filter(&query, &title, &key.label()) {
                    continue;
                }

                let is_expanded = self.expanded.contains(&(document, key.clone()));
                rows.push(Row::Group {
                    document,
                    key,
                    count: tiles.len(),
                });
                if is_expanded {
                    rows.extend(tiles.into_iter().map(|(coord, instance)| Row::Tile {
                        document,
                        coord,
                        instance,
                    }));
                }
            }
        }

        rows
    }

    fn tiles(&self, session: &Session, id: DocumentId, key: &IssueKey) -> Vec<(Coord, Option<PrefabInstanceId>)> {
        let Some(document) = session.state.document(id) else {
            return Vec::new();
        };

        let mut tiles: Vec<_> = match key {
            IssueKey::MissingBase => session
                .missing_base_tiles(id)
                .iter()
                .map(|coord| (*coord, None))
                .collect(),
            IssueKey::UnknownPath(path) => session
                .map_issues(id)
                .iter()
                .filter(|group| group.path == *path)
                .flat_map(|group| &group.instances)
                .filter_map(|instance| {
                    let location = document.instance_location(*instance)?;
                    Some((location.coord, Some(*instance)))
                })
                .collect(),
        };
        if self.current_level_only {
            tiles.retain(|(coord, _)| coord.z == document.z);
        }

        tiles
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_group(
        &mut self, ui: &Ui, session: &Session, tree: &ObjectTree, document: DocumentId, key: &IssueKey, count: usize,
        action: &mut Option<Action>, output: &mut IssuesOutput,
    ) {
        let slot = (document, key.clone());
        let is_open = self.expanded.contains(&slot);
        let is_now_open = ui
            .tree_node_config("group")
            .label(key.label())
            .opened(is_open, Condition::Always)
            .no_tree_push_on_open(true)
            .span_avail_width(true)
            .push()
            .is_some();
        if is_now_open != is_open {
            if is_now_open {
                self.expanded.insert(slot);
            } else {
                self.expanded.remove(&slot);
            }
        }

        if let IssueKey::UnknownPath(path) = key
            && let Some(_menu) = ui.begin_popup_context_item()
            && ui.menu_item("Copy path")
        {
            output.copy = Some(path.to_string());
        }

        ui.table_next_column();
        ui.text(count.to_string());
        if self.all_maps {
            ui.table_next_column();
            ui.text(session.state.document(document).unwrap().title());
        }

        ui.table_next_column();
        let tiles = self.tiles(session, document, key);
        match key {
            IssueKey::MissingBase => {
                let is_fillable = session.world().is_some_and(|world| world.base_tile.is_some());
                if ui.with_disabled_if(!is_fillable, || ui.small_button("Fill all")) {
                    *action = Some(Action::FillBases {
                        document,
                        tiles: tiles.into_iter().map(|(coord, _)| coord).collect(),
                    });
                }

                ui.set_item_tooltip("Add world.turf and world.area where they are missing");
            },
            IssueKey::UnknownPath(path) => {
                let instances = tiles
                    .into_iter()
                    .filter_map(|(_, instance)| instance)
                    .collect::<Vec<_>>();
                let instances = session.editable_issue_instances(document, path, &instances);
                let is_classifiable = issues::has_replacement_kind(tree, path);
                let target = replace_button(
                    ui,
                    tree,
                    &mut self.replace_query,
                    path,
                    instances.is_empty() || !is_classifiable,
                );
                if !is_classifiable {
                    ui.set_item_tooltip("Cannot determine whether this path is an object, turf, or area");
                }

                ui.same_line();
                if ui.with_disabled_if(instances.is_empty(), || ui.small_button("Delete all")) {
                    *action = Some(Action::Delete {
                        document,
                        path: path.clone(),
                        instances: instances.clone(),
                    });
                }

                if let Some(target) = target {
                    *action = Some(Action::Replace {
                        document,
                        path: path.clone(),
                        instances,
                        target,
                    });
                }
            },
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_tile(
        &mut self, ui: &Ui, session: &Session, tree: &ObjectTree, document: DocumentId, coord: Coord,
        instance: Option<PrefabInstanceId>, action: &mut Option<Action>, output: &mut IssuesOutput,
    ) {
        let jump = |action: &mut Option<Action>, output: &mut IssuesOutput| match instance {
            Some(instance) => *action = Some(Action::Jump(JumpTarget { document, instance })),
            None => output.tile = Some((document, coord)),
        };
        ui.indent();
        let is_clicked = ui
            .selectable_config(format!("{}, {}, {}", coord.x, coord.y, coord.z))
            .flags(SelectableFlags::SPAN_ALL_COLUMNS | SelectableFlags::ALLOW_OVERLAP)
            .build();
        ui.unindent();
        if is_clicked {
            jump(action, output);
        }

        if let Some(_menu) = ui.begin_popup_context_item() {
            if ui.menu_item("Jump to tile") {
                jump(action, output);
            }

            if ui.menu_item("Copy coordinates") {
                output.copy = Some(format!("{}, {}, {}", coord.x, coord.y, coord.z));
            }
        }

        ui.table_next_column();
        ui.table_next_column();
        if self.all_maps {
            ui.table_next_column();
        }

        let Some(map) = session.state.document(document) else {
            return;
        };

        let Some(instance) = instance else {
            let is_fillable =
                map.allows_edit_at(coord) && session.world().is_some_and(|world| world.base_tile.is_some());
            if ui.with_disabled_if(!is_fillable, || ui.small_button("Fill")) {
                *action = Some(Action::FillBases {
                    document,
                    tiles: vec![coord],
                });
            }

            return;
        };

        let Some(path) = map.prefab_instance(instance).map(|found| found.prefab().path.clone()) else {
            return;
        };

        let instances = session.editable_issue_instances(document, &path, &[instance]);
        let target = replace_button(
            ui,
            tree,
            &mut self.replace_query,
            &path,
            instances.is_empty() || !issues::has_replacement_kind(tree, &path),
        );
        ui.same_line();
        if ui.with_disabled_if(instances.is_empty(), || ui.small_button("Delete")) {
            *action = Some(Action::Delete {
                document,
                path: path.clone(),
                instances: instances.clone(),
            });
        }

        if let Some(target) = target {
            *action = Some(Action::Replace {
                document,
                path,
                instances,
                target,
            });
        }
    }
}

fn apply(session: &mut Session, action: Action) -> Option<JumpTarget> {
    match action {
        Action::Jump(target) => {
            session
                .state
                .document(target.document)?
                .instance_location(target.instance)?;
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
        Action::FillBases { document, tiles } => {
            session.fill_missing_bases(document, &tiles);
            None
        },
    }
}

fn replace_button(
    ui: &Ui, tree: &ObjectTree, query: &mut String, source: &TreePath, is_disabled: bool,
) -> Option<TreePath> {
    if ui.with_disabled_if(is_disabled, || ui.small_button("Replace…")) {
        query.clear();
        ui.open_popup("replace");
    }

    let _popup = ui.begin_popup("replace")?;
    let target = draw_type_path_search(
        ui,
        Some(tree),
        query,
        "##replacement-path",
        50,
        |tree, path| issues::can_replace(tree, source, path),
        |_| true,
    );
    if target.is_some() {
        ui.close_current_popup();
    }

    target
}

fn matches_filter(query: &str, title: &str, label: &str) -> bool {
    title.to_ascii_lowercase().contains(query) || label.to_ascii_lowercase().contains(query)
}

#[cfg(test)]
mod tests;
