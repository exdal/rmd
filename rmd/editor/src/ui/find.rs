use core::path::TreePath;

use dear_imgui_rs::{
    Condition,
    ListClipper,
    SelectableFlags,
    StyleColor,
    TableFlags,
    TableSizingPolicy,
    Ui,
    WindowKey,
    WindowKeyError,
};
use editor::{
    document::{DocumentId, PrefabInstanceId, Selection},
    search::{SearchQuery, resolve_instances},
};

use super::common::{centered_note, focus_window_on_hover, text_wrapped_colored};
use crate::{session::Session, settings::Settings};

const SEARCH_WINDOW_SIZE: [f32; 2] = [460.0, 360.0];

#[derive(Clone, Copy)]
pub(super) enum SimilarMatchKind {
    Type,
    Prefab,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct JumpTarget {
    pub(super) document: DocumentId,
    pub(super) instance: PrefabInstanceId,
}

struct ActiveSearch {
    document: DocumentId,
    query: SearchQuery,
    bounds: Option<Selection>,
    instances: Vec<PrefabInstanceId>,
    current: Option<PrefabInstanceId>,
}

enum Action {
    Search,
    Refresh,
    Delete(Vec<PrefabInstanceId>),
    Replace(Vec<PrefabInstanceId>),
    Jump(PrefabInstanceId),
    Step(bool),
}

pub(super) struct FindPanel {
    window: WindowKey,
    focus: bool,
    visible: bool,
    path: String,
    subtypes: bool,
    in_selection: bool,
    error: Option<&'static str>,
    search: Option<ActiveSearch>,
}

impl FindPanel {
    pub(super) fn new() -> Result<Self, WindowKeyError> {
        Ok(Self {
            window: WindowKey::new("search", "Search")?,
            focus: false,
            visible: false,
            path: String::new(),
            subtypes: false,
            in_selection: false,
            error: None,
            search: None,
        })
    }

    pub(super) const fn window(&self) -> &WindowKey { &self.window }

    pub(super) fn request(&mut self) { self.focus = true; }

    pub(super) const fn has_focus_request(&self) -> bool { self.focus }

    #[cfg(test)]
    pub(super) const fn visible(&self) -> bool { self.visible }

    pub(super) fn open_for(
        &mut self, session: &Session, document: DocumentId, instance: PrefabInstanceId, kind: SimilarMatchKind,
    ) {
        let Some((prefab, _)) = session
            .state
            .document(document)
            .and_then(|document| document.prefab_instance(instance))
        else {
            return;
        };
        let query = match kind {
            SimilarMatchKind::Prefab => SearchQuery::Prefab(prefab.clone()),
            SimilarMatchKind::Type => SearchQuery::Type {
                path: prefab.path.clone(),
                subtypes: false,
            },
        };
        self.path = prefab.path.to_string();
        self.subtypes = false;
        self.in_selection = false;
        self.error = None;
        self.run(session, document, query, None);
        self.request();
    }

    fn run(&mut self, session: &Session, document: DocumentId, query: SearchQuery, bounds: Option<Selection>) {
        self.search = Some(ActiveSearch {
            instances: session.find_instances(document, &query, bounds),
            document,
            query,
            bounds,
            current: None,
        });
    }

    fn search_typed_path(&mut self, session: &Session) {
        self.error = None;
        let text = self.path.trim();
        if text.is_empty() {
            return;
        }
        let path = TreePath::parse(text);
        if session.tree().is_some_and(|tree| tree.id_of(&path).is_none()) {
            self.error = Some("Unknown type");
            return;
        }
        let Some(document) = session.state.active() else {
            self.error = Some("No map is open");
            return;
        };
        let bounds = if self.in_selection {
            let Some(selection) = session.selection() else {
                self.error = Some("Nothing is block selected");
                return;
            };
            Some(selection)
        } else {
            None
        };

        let query = SearchQuery::Type {
            path,
            subtypes: self.subtypes,
        };
        self.run(session, document, query, bounds);
    }

    pub(super) fn step(&mut self, session: &Session, forward: bool) -> Option<JumpTarget> {
        let search = self.search.as_mut()?;
        let rows = resolve_instances(session.state.document(search.document)?, &search.instances);
        if rows.is_empty() {
            return None;
        }
        let position = search
            .current
            .and_then(|current| rows.iter().position(|(instance, _)| *instance == current));
        let next = match (position, forward) {
            (None, true) => 0,
            (None, false) => rows.len() - 1,
            (Some(position), true) => (position + 1) % rows.len(),
            (Some(position), false) => (position + rows.len() - 1) % rows.len(),
        };
        let instance = rows[next].0;
        search.current = Some(instance);

        Some(JumpTarget {
            document: search.document,
            instance,
        })
    }

    pub(super) fn draw(&mut self, ui: &Ui, session: &mut Session, settings: &Settings) -> Option<JumpTarget> {
        if self
            .search
            .as_ref()
            .is_some_and(|search| session.state.document(search.document).is_none())
        {
            self.search = None;
        }

        let mut action = None;
        let focus = std::mem::take(&mut self.focus);
        self.visible = false;
        let Self {
            window,
            path,
            subtypes,
            in_selection,
            error,
            search,
            visible,
            ..
        } = self;

        ui.window(&*window)
            .size(SEARCH_WINDOW_SIZE, Condition::FirstUseEver)
            .focused(focus)
            .build(|| {
                *visible = true;
                if settings.focus_windows_on_hover {
                    focus_window_on_hover(ui);
                }

                ui.set_next_item_width(-f32::MIN_POSITIVE);
                if focus {
                    ui.set_keyboard_focus_here();
                }
                let entered = ui
                    .input_text("##search-path", path)
                    .hint("Type path, e.g. /obj/machinery/door")
                    .enter_returns_true(true)
                    .build();
                ui.checkbox("Subtypes", subtypes);
                ui.same_line();
                ui.checkbox("In selection", in_selection);
                ui.set_item_tooltip("Search inside selected area (selection tool)");
                ui.same_line();
                if ui.button("Search") || entered {
                    action = Some(Action::Search);
                }
                if let Some(error) = error {
                    text_wrapped_colored(ui, ui.style_color(StyleColor::TextDisabled), error);
                }
                ui.separator();

                let Some(search) = search.as_ref() else {
                    centered_note(ui, "Search a type, or right click an atom and search by type or prefab");
                    return;
                };
                let Some(document) = session.state.document(search.document) else {
                    return;
                };
                let rows = resolve_instances(document, &search.instances);

                let (target, kind) = match &search.query {
                    SearchQuery::Prefab(prefab) => (prefab.path.to_string(), "prefab"),
                    SearchQuery::Type { path, subtypes: false } => (path.to_string(), "type"),
                    SearchQuery::Type { path, subtypes: true } => (path.to_string(), "type or subtype"),
                };
                ui.text_wrapped(&target);
                let suffix = if rows.len() == 1 { "instance" } else { "instances" };
                let scope = if search.bounds.is_some() {
                    " in the selection"
                } else {
                    ""
                };
                text_wrapped_colored(
                    ui,
                    ui.style_color(StyleColor::TextDisabled),
                    &format!("{} matching {kind} {suffix}{scope} in {}", rows.len(), document.title()),
                );

                if ui.button("Refresh") {
                    action = Some(Action::Refresh);
                }
                ui.same_line();
                let all = rows.iter().map(|(instance, _)| *instance).collect::<Vec<_>>();
                if ui.with_disabled_if(rows.is_empty(), || ui.button("Delete all")) {
                    action = Some(Action::Delete(all.clone()));
                }
                ui.same_line();
                let brush = session.palette().map(|prefab| prefab.path.to_string());
                let replaceable = all
                    .iter()
                    .any(|instance| session.can_replace_instance(search.document, *instance));
                if ui.with_disabled_if(!replaceable, || ui.button("Replace all")) {
                    action = Some(Action::Replace(all.clone()));
                }
                ui.set_item_tooltip(brush.as_deref().map_or_else(
                    || String::from("Choose a type to replace with"),
                    |brush| format!("Replace with {brush}"),
                ));
                ui.same_line();
                if ui.with_disabled_if(rows.is_empty(), || {
                    ui.arrow_button("##previous", dear_imgui_rs::Direction::Left)
                }) {
                    action = Some(Action::Step(false));
                }
                ui.same_line();
                if ui.with_disabled_if(rows.is_empty(), || {
                    ui.arrow_button("##next", dear_imgui_rs::Direction::Right)
                }) {
                    action = Some(Action::Step(true));
                }
                ui.separator();

                if rows.is_empty() {
                    centered_note(ui, "No matching instances remain");
                    return;
                }

                ui.table("search-results")
                    .flags(TableFlags::BORDERS_INNER_V | TableFlags::RESIZABLE | TableFlags::ROW_BG)
                    .sizing_policy(TableSizingPolicy::StretchProp)
                    .column("Tile")
                    .weight(0.5)
                    .done()
                    .column("Action")
                    .weight(0.5)
                    .done()
                    .build(|ui| {
                        for index in ListClipper::new(rows.len()).begin(ui).iter() {
                            let (instance, location) = rows[index];
                            let row_id = instance.get().to_string();
                            let _id = ui.push_id(&row_id);

                            ui.table_next_row();
                            ui.table_next_column();
                            let coord = location.coord;
                            if ui
                                .selectable_config(format!("{}, {}, {}", coord.x, coord.y, coord.z))
                                .selected(search.current == Some(instance))
                                .flags(SelectableFlags::ALLOW_OVERLAP)
                                .build()
                            {
                                action = Some(Action::Jump(instance));
                            }
                            ui.table_next_column();
                            if ui.small_button("Jump to") {
                                action = Some(Action::Jump(instance));
                            }
                            ui.same_line();
                            if ui.small_button("Delete") {
                                action = Some(Action::Delete(vec![instance]));
                            }
                            ui.same_line();
                            let can_replace = session.can_replace_instance(search.document, instance);
                            if ui.with_disabled_if(!can_replace, || ui.small_button("Replace")) {
                                action = Some(Action::Replace(vec![instance]));
                            }
                        }
                    });
            });

        self.apply(session, action?)
    }

    fn apply(&mut self, session: &mut Session, action: Action) -> Option<JumpTarget> {
        match action {
            Action::Search => self.search_typed_path(session),
            Action::Refresh => {
                let search = self.search.take()?;
                self.run(session, search.document, search.query, search.bounds);
            },
            Action::Delete(instances) => {
                session.delete_instances(self.search.as_ref()?.document, &instances);
            },
            Action::Replace(instances) => {
                session.replace_instances(self.search.as_ref()?.document, &instances);
            },
            Action::Jump(instance) => {
                let search = self.search.as_mut()?;
                search.current = Some(instance);

                return Some(JumpTarget {
                    document: search.document,
                    instance,
                });
            },
            Action::Step(forward) => return self.step(session, forward),
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use core::path::TreePath;

    use dmm::{Coord, Map, Prefab, Size};
    use editor::document::MapDocument;

    use super::*;
    use crate::ui::{MapViewState, UiState};

    fn map() -> Map {
        let table = Prefab::new(TreePath::parse("/obj/table"));
        let chair = Prefab::new(TreePath::parse("/obj/chair"));
        let mut map = Map::new(Size { x: 2, y: 1, z: 2 });
        let first = map.intern_tile(vec![table.clone(), chair]);
        let second = map.intern_tile(vec![table]);
        map.grid[0][0] = vec![first, second];
        map.grid[1][0] = vec![second, first];

        map
    }

    #[test]
    fn context_search_uses_the_clicked_instance_instead_of_inspector_selection() {
        let document = MapDocument::new(map(), 1);
        let selected = document.instance_ids_at(Coord::new(1, 1, 1))[0];
        let clicked = document.instance_ids_at(Coord::new(1, 1, 1))[1];
        let other_match = document.instance_ids_at(Coord::new(2, 1, 2))[1];
        let mut session = Session::new();
        let document_id = session.state.open_document(document);
        session.select_instance(Some(selected));
        let mut find = FindPanel::new().expect("valid window key");

        find.open_for(&session, document_id, clicked, SimilarMatchKind::Prefab);

        let search = find.search.as_ref().expect("a search ran");
        assert!(find.has_focus_request());
        assert_eq!(search.document, document_id);
        assert_eq!(search.instances, [clicked, other_match]);
        assert_eq!(session.selected_instance(), Some(selected));
    }

    #[test]
    fn stepping_wraps_around_the_matches_that_remain() {
        let document = MapDocument::new(map(), 1);
        let first = document.instance_ids_at(Coord::new(1, 1, 1))[0];
        let mut session = Session::new();
        let document_id = session.state.open_document(document);
        let mut find = FindPanel::new().expect("valid window key");
        find.open_for(&session, document_id, first, SimilarMatchKind::Type);
        let matches = find.search.as_ref().unwrap().instances.clone();
        assert_eq!(matches.len(), 4);

        let visited = (0..5)
            .map(|_| find.step(&session, true).unwrap().instance)
            .collect::<Vec<_>>();
        assert_eq!(visited, [matches[0], matches[1], matches[2], matches[3], matches[0]]);
        assert_eq!(find.step(&session, false).unwrap().instance, matches[3]);

        assert!(session.delete_instances(document_id, &[matches[2]]));
        assert_eq!(find.step(&session, false).unwrap().instance, matches[1]);
    }

    #[test]
    fn jumping_to_an_instance_selects_its_level_and_centers_the_map_view() {
        let document = MapDocument::new(map(), 1);
        let instance = document.instance_ids_at(Coord::new(1, 1, 2))[0];
        let mut session = Session::new();
        let document_id = session.state.open_document(document);
        let mut state = UiState::new(false).expect("valid window keys");
        let mut view = MapViewState::new(document_id).expect("valid map view key");
        view.camera.camera.zoom = 2.5;
        state.map_views.insert(document_id, view);

        state.jump_to_instance(
            &mut session,
            JumpTarget {
                document: document_id,
                instance,
            },
        );

        assert_eq!(session.state.active(), Some(document_id));
        assert_eq!(session.z(), 2);
        assert_eq!(session.selected_instance(), Some(instance));
        let view = state.map_views.get(&document_id).unwrap();
        assert_eq!((view.camera.camera.x, view.camera.camera.y), (16.0, 16.0));
        assert_eq!(view.camera.camera.zoom, 2.5);
        assert!(view.focus);
        assert!(!view.refit);
    }
}
