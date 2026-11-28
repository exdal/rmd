use base::types::{Identifier, Value};
use prefab::PrefabInstanceId;
use std::collections::{HashMap, HashSet};

use dear_imgui_rs::Ui;
use ui::{icons::materialdesignicons::ICON_CIRCLE_SMALL, widgets::checkbox_width};

use crate::{
    app::external_editor::SourceLocation,
    document::command::EditGroupId,
    git::{
        blame::{self, BlameCell},
        panel::commit_summary,
    },
    inspector::session::EditScope,
    maploader::preview::PREVIEW_ID_START,
    session::Session,
};

use super::{
    ADVANCED_OFFSETS, DISPLAY_ROWS, TransformMode,
    property::{draw_identical_banner, identical_frame_colors, property_table},
    snapshot::{
        InspectorProperty, InspectorSnapshot, InspectorVariable, VariableGroup, inspector_snapshot, list_entry_drafts,
        typed_value_text,
    },
    variable::{draw_pin, draw_variable_rows, toggle_pin},
};

#[derive(Default)]
pub struct InspectorState {
    picked: Option<PrefabInstanceId>,
    pub(super) drafts: HashMap<Identifier, VariableDraft>,
    pub(super) active_drag: Option<(Identifier, EditGroupId)>,
    pub(super) transform_mode: TransformMode,
    pub(super) scope: EditScope,
    pub(super) filter: InspectorFilter,
    pub(super) shows_all_directions: bool,
}

#[derive(Default)]
pub(super) struct InspectorFilter {
    pub(super) query: String,
    pub(super) modified_only: bool,
}

pub(super) struct VariableDraft {
    pub(super) text: String,
    pub(super) committed: String,
    pub(super) error: Option<String>,
    pub(super) typed: TypedDraft,
    pub(super) path_query: String,
    pub(super) mode: Option<EditorMode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EditorMode {
    Typed,
    Expression,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct TypedDraft {
    value: Value,
    pub(super) text: String,
    pub(super) entries: Vec<ListEntryDraft>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct ListEntryDraft {
    pub(super) key: String,
    pub(super) value: String,
}

#[derive(Default)]
pub(crate) struct InspectorOutput {
    pub open_source: Option<SourceLocation>,
    pub find_similar: bool,
    pub copy: Option<String>,
}

impl InspectorState {
    pub(crate) const fn transform_mode(&self) -> TransformMode {
        self.transform_mode
    }

    pub fn draw(
        &mut self, ui: &Ui, session: &mut Session, pinned_vars: &mut Vec<String>, pinned_types: &mut Vec<String>,
    ) -> InspectorOutput {
        let Some(snapshot) = inspector_snapshot(session) else {
            self.clear();
            ui.text_disabled("No object selected");

            return InspectorOutput::default();
        };

        self.sync(&snapshot);
        let find_similar = ui.text_link("Find similar...");
        let is_preview = snapshot.picked.get() >= PREVIEW_ID_START;
        let is_stash = session.is_stash_active();
        let is_outside_focus = session
            .state
            .active_document()
            .is_some_and(|document| !document.is_within_focus(snapshot.location.coord));
        if is_preview {
            ui.text_disabled("Module preview, read only");
        } else if is_stash {
            ui.text_disabled("Stash, read only");
        } else if is_outside_focus {
            ui.text_disabled("Outside the focused area, read only");
        }

        ui.separator();
        let _disabled = ui.begin_disabled_with_cond(is_preview || is_stash || is_outside_focus);
        session.refresh_identical();
        match self.scope {
            EditScope::Selected => {
                let mut identical = false;
                if ui.checkbox(
                    format!("Edit all {} identical", session.identical().len()),
                    &mut identical,
                ) {
                    self.scope = EditScope::Identical;
                }
                ui.set_item_tooltip("Changes apply to every placement of this exact object on the map");
            },
            EditScope::Identical => {
                if draw_identical_banner(ui, session) {
                    self.scope = EditScope::Selected;
                }
            },
        }

        let mut copy = None;
        if let Some(id) = session.state.active()
            && session.git_state(id).is_some_and(|git| git.show_blame)
        {
            ui.separator();
            ui.text("Blame");
            enum Detail {
                Commit(u32, crate::git::repo::CommitInfo),
                Boundary,
                Note(&'static str),
            }
            let detail = session
                .blame_at(id, snapshot.location.coord)
                .map(|(cell, changed)| match cell {
                    BlameCell::Commit(index, commit) if !changed => Detail::Commit(index, commit.clone()),
                    BlameCell::Boundary if !changed => Detail::Boundary,
                    _ => Detail::Note(blame::pending_note(cell, changed).unwrap_or_default()),
                });
            match detail {
                Some(Detail::Commit(index, commit)) => {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |time| time.as_secs() as i64);
                    ui.text(format!(
                        "{} {ICON_CIRCLE_SMALL} {} {ICON_CIRCLE_SMALL} {}",
                        commit.short,
                        commit.author,
                        blame::relative_time(now, commit.time)
                    ));
                    let web = session.git_state(id).and_then(|git| git.web.clone());
                    commit_summary(ui, &commit, web.as_ref(), ui.content_region_avail_width());
                    if ui.small_button("Pin commit tiles") {
                        session.pin_blame(id, index);
                    }
                    ui.same_line();
                    if ui.small_button("Copy hash") {
                        copy = Some(commit.hash);
                    }
                },
                Some(Detail::Boundary) => {
                    if let Some(label) = session
                        .git_state(id)
                        .and_then(|git| git.blame.as_ref())
                        .map(|blame| blame.result.boundary_label())
                    {
                        ui.text_disabled(label);
                    }
                },
                Some(Detail::Note(note)) => ui.text_disabled(note),
                None => ui.text_disabled("Run blame from the Git menu"),
            }

            if session.blame_stale(id) {
                ui.text_disabled("Stale result");
            }
        }

        ui.separator();
        self.filter.draw(ui);

        let _identical_frames = (self.scope == EditScope::Identical).then(|| identical_frame_colors(ui));
        let (pinned, variables) = split_pinned(&snapshot.variables, pinned_vars);
        let mut open_source = None;
        let mut is_drawn = false;
        let mut is_any_shown = false;
        for section in inspector_sections(&snapshot.groups, pinned_types) {
            match section {
                InspectorSection::PinnedVariables => {
                    let is_shown = pinned
                        .iter()
                        .any(|variable| self.filter.shows(variable.name.as_str(), variable.overridden));
                    if !is_shown {
                        continue;
                    }

                    if is_drawn {
                        ui.separator();
                    }

                    property_table(ui, "inspector-pinned", |ui| {
                        draw_variable_rows(ui, session, self, pinned_vars, &pinned);
                    });
                    is_drawn = true;
                    is_any_shown = true;
                },
                InspectorSection::Properties => {
                    if !self.shows_properties(&snapshot) {
                        continue;
                    }

                    if is_drawn {
                        ui.separator();
                    }

                    property_table(ui, "inspector-properties", |ui| {
                        self.draw_transform(ui, session, &snapshot);
                        self.draw_display(ui, session, &snapshot);
                    });
                    is_drawn = true;
                    is_any_shown = true;
                },
                InspectorSection::Group(index, group) => {
                    let rows = variables
                        .iter()
                        .copied()
                        .filter(|variable| {
                            variable.owner == group.owner
                                && self.filter.shows(variable.name.as_str(), variable.overridden)
                        })
                        .collect::<Vec<_>>();
                    if index > 0 && rows.is_empty() {
                        continue;
                    }

                    if is_drawn {
                        ui.separator();
                    }

                    is_drawn = true;
                    let is_pinnable = group.owner.is_some();
                    let is_pinned = pinned_types.contains(&group.path);
                    if is_pinnable {
                        ui.align_text_to_frame_padding();
                        if draw_pin(ui, is_pinned) {
                            toggle_pin(pinned_types, &group.path);
                        }

                        ui.same_line();
                    }

                    match &group.source {
                        Some(source) if ui.text_link(&group.path) => open_source = Some(source.clone()),
                        Some(_) => {},
                        None => ui.text_wrapped(&group.path),
                    }

                    if let Some(_popup) = ui.begin_popup_context_item_with_label(Some(&group.path)) {
                        if is_pinnable && ui.menu_item(if is_pinned { "Unpin type" } else { "Pin type to the top" }) {
                            toggle_pin(pinned_types, &group.path);
                        }

                        if ui.menu_item("Copy type path") {
                            copy = Some(group.path.clone());
                        }
                    }

                    if !rows.is_empty() {
                        property_table(ui, "inspector-properties", |ui| {
                            draw_variable_rows(ui, session, self, pinned_vars, &rows);
                        });
                        is_any_shown = true;
                    }
                },
            }
        }

        if !is_any_shown {
            ui.text_disabled("No matching variables");
        }

        InspectorOutput {
            open_source,
            find_similar,
            copy,
        }
    }

    pub(super) fn shown<'a>(&self, snapshot: &'a InspectorSnapshot, name: &str) -> Option<&'a InspectorProperty> {
        snapshot
            .property(name)
            .filter(|property| self.filter.shows(name, property.overridden))
    }

    pub(super) fn shows_any(&self, snapshot: &InspectorSnapshot, names: &[&str]) -> bool {
        names.iter().any(|name| self.shown(snapshot, name).is_some())
    }

    fn shows_properties(&self, snapshot: &InspectorSnapshot) -> bool {
        let (x, y) = self.transform_mode.variables();

        snapshot.is_atom
            && (self.shows_any(snapshot, &[x, y])
                || self.shows_any(snapshot, DISPLAY_ROWS)
                || self.shows_any(snapshot, ADVANCED_OFFSETS))
    }

    fn clear(&mut self) {
        self.picked = None;
        self.drafts.clear();
        self.active_drag = None;
        self.shows_all_directions = false;
    }

    fn sync(&mut self, snapshot: &InspectorSnapshot) {
        if self.picked != Some(snapshot.picked) {
            self.drafts.clear();
            self.active_drag = None;
            self.shows_all_directions = false;
            self.picked = Some(snapshot.picked);
        }

        if !snapshot.is_movable {
            self.transform_mode = TransformMode::Pixel;
        }

        let current = snapshot
            .properties
            .values()
            .map(|property| (&property.name, &property.editor_text, &property.value))
            .chain(
                snapshot
                    .variables
                    .iter()
                    .map(|variable| (&variable.name, &variable.source, &variable.value)),
            );
        let names = current.clone().map(|(name, ..)| name).collect::<HashSet<_>>();
        self.drafts.retain(|name, _| names.contains(name));

        for (name, source, value) in current {
            match self.drafts.get_mut(name) {
                Some(draft) => draft.sync(source, value),
                None => {
                    self.drafts.insert(name.clone(), VariableDraft::new(source, value));
                },
            }
        }
    }
}

impl VariableDraft {
    pub(super) fn new(source: &str, value: &Value) -> Self {
        Self {
            text: source.to_string(),
            committed: source.to_string(),
            error: None,
            typed: TypedDraft::new(value),
            path_query: String::new(),
            mode: None,
        }
    }

    pub(super) fn sync(&mut self, source: &str, value: &Value) {
        if self.committed != source && self.text == self.committed {
            self.text = source.to_string();
            self.committed = source.to_string();
            self.error = None;
        }

        if self.typed.value != *value && !self.typed.is_edited() {
            self.typed = TypedDraft::new(value);
        }
    }
}

impl TypedDraft {
    pub(super) fn new(value: &Value) -> Self {
        Self {
            value: value.clone(),
            text: typed_value_text(value),
            entries: list_entry_drafts(value),
        }
    }

    fn is_edited(&self) -> bool {
        self.text != typed_value_text(&self.value) || self.entries != list_entry_drafts(&self.value)
    }
}

pub(super) fn drag_group(active_drag: &mut Option<(Identifier, EditGroupId)>, name: &Identifier) -> EditGroupId {
    if let Some((active, group)) = active_drag
        && active == name
    {
        return *group;
    }

    let group = EditGroupId::new();
    *active_drag = Some((name.clone(), group));

    group
}

pub(super) fn end_drag(active_drag: &mut Option<(Identifier, EditGroupId)>, name: &Identifier) {
    if active_drag.as_ref().is_some_and(|(active, _)| active == name) {
        *active_drag = None;
    }
}

impl InspectorFilter {
    pub(super) fn is_active(&self) -> bool {
        self.modified_only || !self.query.trim().is_empty()
    }

    pub(super) fn shows(&self, name: &str, overridden: bool) -> bool {
        let query = self.query.trim().to_ascii_lowercase().replace(' ', "_");

        (overridden || !self.modified_only) && name.to_ascii_lowercase().contains(&query)
    }

    fn draw(&mut self, ui: &Ui) {
        let label = "Modified only";
        let spacing = ui.clone_style().item_spacing()[0];
        ui.set_next_item_width((ui.content_region_avail_width() - checkbox_width(ui, label) - spacing).max(1.0));
        ui.input_text("##inspector-filter", &mut self.query)
            .hint("Filter variables")
            .build();
        ui.same_line();
        ui.checkbox(label, &mut self.modified_only);
    }
}

#[derive(Debug, PartialEq)]
pub(super) enum InspectorSection<'a> {
    PinnedVariables,
    Properties,
    Group(usize, &'a VariableGroup),
}

pub(super) fn inspector_sections<'a>(
    groups: &'a [VariableGroup], pinned_types: &[String],
) -> Vec<InspectorSection<'a>> {
    let is_pinned = |group: &VariableGroup| group.owner.is_some() && pinned_types.contains(&group.path);
    let pinned = pinned_types.iter().filter_map(|pin| {
        groups
            .iter()
            .enumerate()
            .find(|(_, group)| is_pinned(group) && &group.path == pin)
    });
    let rest = groups.iter().enumerate().filter(|(_, group)| !is_pinned(group));

    pinned
        .map(|(index, group)| InspectorSection::Group(index, group))
        .chain([InspectorSection::PinnedVariables, InspectorSection::Properties])
        .chain(rest.map(|(index, group)| InspectorSection::Group(index, group)))
        .collect()
}

type SplitVariables<'a> = (Vec<&'a InspectorVariable>, Vec<&'a InspectorVariable>);

pub(super) fn split_pinned<'a>(variables: &'a [InspectorVariable], pins: &[String]) -> SplitVariables<'a> {
    let pinned = pins
        .iter()
        .filter_map(|pin| variables.iter().find(|variable| variable.name.as_str() == pin))
        .collect();
    let unpinned = variables
        .iter()
        .filter(|variable| !pins.iter().any(|pin| pin == variable.name.as_str()))
        .collect();

    (pinned, unpinned)
}
