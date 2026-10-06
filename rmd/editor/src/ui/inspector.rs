use core::{
    path::TreePath,
    types::{Identifier, ListEntry, Value},
    vars,
};
use std::collections::{HashMap, HashSet};

use dear_imgui_rs::{ChildFlags, DragFlags, StyleColor, TableFlags, TableSizingPolicy, Ui, WindowKey, WindowKeyError};
use dmi::metadata::Dir;
use dmm::{Coord, Prefab, parser::parse_value, writer::format_value};
use editor::{
    blame::{self, BlameCell},
    command::EditGroupId,
    document::{DocumentId, PrefabInstanceId, PrefabLocation, VarMutation},
    icons::materialdesignicons::{ICON_CIRCLE_SMALL, ICON_PIN, ICON_PIN_OUTLINE},
    visual,
};
use objtree::{ObjectTree, ResolvedVarType, TypeId, VarTypeKind};

use super::{
    common::{IDENTICAL_EDIT_COLOR, checkbox_width, focus_window_on_hover, text_wrapped_colored},
    git::commit_summary,
    search::draw_type_path_search,
};
use crate::{
    external_editor::SourceLocation,
    session::{DirectionalTypes, EditScope, Session},
    settings::Settings,
    transform::anchor_axis,
};

const DISPLAY_PROPERTIES: &[&str] = &[
    vars::NAME,
    vars::ICON,
    vars::ICON_STATE,
    vars::DIR,
    vars::PIXEL_X,
    vars::PIXEL_Y,
    vars::PIXEL_W,
    vars::PIXEL_Z,
    vars::PLANE,
    vars::LAYER,
    vars::COLOR,
    vars::ALPHA,
    vars::INVISIBILITY,
];
const MOVABLE_PROPERTIES: &[&str] = &[
    vars::PIXEL_X,
    vars::PIXEL_Y,
    vars::PIXEL_Z,
    vars::PIXEL_W,
    vars::STEP_X,
    vars::STEP_Y,
    "step_z",
    "step_w",
];
const DISPLAY_ROWS: &[&str] = &[
    vars::NAME,
    vars::ICON,
    vars::ICON_STATE,
    vars::DIR,
    vars::PLANE,
    vars::LAYER,
    vars::COLOR,
    vars::ALPHA,
    vars::INVISIBILITY,
];
const ADVANCED_OFFSETS: &[&str] = &[vars::PIXEL_W, vars::PIXEL_Z];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum TransformMode {
    #[default]
    Pixel,
    Step,
}

impl TransformMode {
    const ALL: [Self; 2] = [Self::Pixel, Self::Step];

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Pixel => "Pixel",
            Self::Step => "Step",
        }
    }

    pub(crate) const fn variables(self) -> (&'static str, &'static str) {
        match self {
            Self::Pixel => (vars::PIXEL_X, vars::PIXEL_Y),
            Self::Step => (vars::STEP_X, vars::STEP_Y),
        }
    }
}

#[derive(Default)]
pub struct InspectorState {
    selected: Option<PrefabInstanceId>,
    drafts: HashMap<Identifier, VariableDraft>,
    active_drag: Option<(Identifier, EditGroupId)>,
    transform_mode: TransformMode,
    scope: EditScope,
    filter: InspectorFilter,
}

#[derive(Default)]
struct InspectorFilter {
    query: String,
    modified_only: bool,
}

struct VariableDraft {
    text: String,
    committed: String,
    error: Option<String>,
    typed: TypedDraft,
    path_query: String,
    raw: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct TypedDraft {
    value: Value,
    text: String,
    entries: Vec<ListEntryDraft>,
}

#[derive(Debug, Clone, PartialEq)]
struct ListEntryDraft {
    key: String,
    value: String,
}

#[derive(Debug, Clone, PartialEq)]
struct InspectorVariable {
    name: Identifier,
    source: String,
    overridden: bool,
    value: Value,
    ty: Option<ResolvedVarType>,
    owner: Option<TypeId>,
}

#[derive(Debug, Clone, PartialEq)]
struct VariableGroup {
    owner: Option<TypeId>,
    path: String,
    source: Option<SourceLocation>,
}

#[derive(Debug, Clone)]
struct InspectorProperty {
    name: Identifier,
    value: Value,
    inherited: Value,
    editor_text: String,
    overridden: bool,
}

struct InspectorSnapshot {
    selected: PrefabInstanceId,
    groups: Vec<VariableGroup>,
    location: PrefabLocation,
    is_atom: bool,
    is_movable: bool,
    properties: HashMap<Identifier, InspectorProperty>,
    variables: Vec<InspectorVariable>,
    icon_states: Vec<(String, u32)>,
    declared_directions: Option<[bool; 8]>,
    directional_types: Option<DirectionalTypes>,
    icon_known: bool,
    icon_state_known: bool,
}

#[derive(Default)]
pub(crate) struct InspectorOutput {
    pub open_source: Option<SourceLocation>,
    pub find_similar: bool,
    pub copy_hash: Option<String>,
}

#[derive(Clone, Copy)]
enum TextPropertyKind {
    Text,
    Resource,
    NullableText,
}

impl InspectorState {
    pub(crate) const fn transform_mode(&self) -> TransformMode { self.transform_mode }

    pub fn draw(&mut self, ui: &Ui, session: &mut Session, pins: &mut Vec<String>) -> InspectorOutput {
        let Some(snapshot) = inspector_snapshot(session) else {
            self.clear();
            ui.text_disabled("No object selected");

            return InspectorOutput::default();
        };

        self.sync(&snapshot);
        ui.text_disabled(format!(
            "Tile {}, {}, {}",
            snapshot.location.coord.x, snapshot.location.coord.y, snapshot.location.coord.z
        ));
        let find_similar = ui.text_link("Find similar...");
        ui.separator();
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

        let mut copy_hash = None;
        if let Some(id) = session.state.active()
            && session.git_state(id).is_some_and(|git| git.show_blame)
        {
            ui.separator();
            ui.text("Blame");
            enum Detail {
                Commit(u32, editor::git::CommitInfo),
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
                        copy_hash = Some(commit.hash);
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
        let (pinned, variables) = split_pinned(&snapshot.variables, pins);
        let is_top_shown = self.shows_properties(&snapshot)
            || pinned
                .iter()
                .any(|variable| self.filter.shows(variable.name.as_str(), variable.overridden));
        if is_top_shown {
            property_table(ui, "inspector-properties", |ui| {
                draw_variable_rows(ui, session, self, pins, &pinned);
                if snapshot.is_atom {
                    self.draw_transform(ui, session, &snapshot);
                    self.draw_display(ui, session, &snapshot);
                }
            });
        }

        let mut open_source = None;
        let mut is_any_shown = is_top_shown;
        for (index, group) in snapshot.groups.iter().enumerate() {
            let rows = variables
                .iter()
                .copied()
                .filter(|variable| {
                    variable.owner == group.owner && self.filter.shows(variable.name.as_str(), variable.overridden)
                })
                .collect::<Vec<_>>();
            if index > 0 && rows.is_empty() {
                continue;
            }

            if index > 0 || is_top_shown {
                ui.separator();
            }

            match &group.source {
                Some(source) if ui.text_link(&group.path) => open_source = Some(source.clone()),
                Some(_) => {},
                None => ui.text_wrapped(&group.path),
            }

            if !rows.is_empty() {
                property_table(ui, "inspector-properties", |ui| {
                    draw_variable_rows(ui, session, self, pins, &rows);
                });
                is_any_shown = true;
            }
        }

        if !is_any_shown {
            ui.text_disabled("No matching variables");
        }

        InspectorOutput {
            open_source,
            find_similar,
            copy_hash,
        }
    }

    fn draw_transform(&mut self, ui: &Ui, session: &mut Session, snapshot: &InspectorSnapshot) {
        let (x, y) = self.transform_mode.variables();
        if !self.shows_any(snapshot, &[x, y]) {
            return;
        }

        let filtering = self.filter.is_active();

        if !filtering {
            ui.table_next_row();
            ui.table_next_column();
            ui.align_text_to_frame_padding();
            ui.text("Tile");
            ui.table_next_column();
            ui.align_text_to_frame_padding();
            ui.text(format!(
                "X {}   Y {}   Z {}",
                snapshot.location.coord.x, snapshot.location.coord.y, snapshot.location.coord.z
            ));
        }

        match self.transform_mode {
            TransformMode::Pixel => {
                self.draw_int_property(ui, session, snapshot, vars::PIXEL_X, "Pixel X", None);
                self.draw_int_property(ui, session, snapshot, vars::PIXEL_Y, "Pixel Y", None);
            },
            TransformMode::Step => {
                self.draw_int_property(ui, session, snapshot, vars::STEP_X, "Step X", None);
                self.draw_int_property(ui, session, snapshot, vars::STEP_Y, "Step Y", None);
            },
        }

        if filtering {
            return;
        }

        ui.table_next_row();
        ui.table_next_column();
        ui.align_text_to_frame_padding();
        ui.text("Mode");
        ui.table_next_column();
        ui.set_next_item_width(-1.0);
        if let Some(combo) = ui.begin_combo("##transform-mode", self.transform_mode.label()) {
            for mode in TransformMode::ALL {
                if ui
                    .selectable_config(mode.label())
                    .selected(self.transform_mode == mode)
                    .disabled(mode == TransformMode::Step && !snapshot.is_movable)
                    .build()
                {
                    self.transform_mode = mode;
                }
            }

            combo.end();
        }
    }

    fn draw_display(&mut self, ui: &Ui, session: &mut Session, snapshot: &InspectorSnapshot) {
        let advanced_shown = self.shows_any(snapshot, ADVANCED_OFFSETS);
        if !advanced_shown && !self.shows_any(snapshot, DISPLAY_ROWS) {
            return;
        }

        self.draw_text_property(ui, session, snapshot, vars::NAME, "Name", TextPropertyKind::Text);
        self.draw_asset_property(
            ui,
            session,
            snapshot,
            vars::ICON,
            "Icon",
            TextPropertyKind::Resource,
            (!snapshot.icon_known).then_some("The current DMI is not loaded"),
        );
        self.draw_asset_property(
            ui,
            session,
            snapshot,
            vars::ICON_STATE,
            "Icon state",
            TextPropertyKind::Text,
            (!snapshot.icon_state_known).then_some("The current state is not present in the DMI"),
        );
        self.draw_direction_property(ui, session, snapshot);
        self.draw_float_property(ui, session, snapshot, vars::PLANE, "Plane");
        self.draw_float_property(ui, session, snapshot, vars::LAYER, "Layer");
        self.draw_color_property(ui, session, snapshot);
        self.draw_int_property(ui, session, snapshot, vars::ALPHA, "Alpha", Some((0, 255)));
        self.draw_int_property(
            ui,
            session,
            snapshot,
            vars::INVISIBILITY,
            "Invisibility",
            Some((0, 101)),
        );

        self.draw_int_property(ui, session, snapshot, vars::PIXEL_W, "Pixel W", None);
        self.draw_int_property(ui, session, snapshot, vars::PIXEL_Z, "Pixel Z", None);
    }

    fn draw_text_property(
        &mut self, ui: &Ui, session: &mut Session, snapshot: &InspectorSnapshot, name: &str, label: &str,
        kind: TextPropertyKind,
    ) {
        let Some(property) = self.shown(snapshot, name) else {
            return;
        };
        if !text_kind_matches(kind, &property.value) {
            self.draw_raw_property(ui, session, property, label);

            return;
        }

        begin_property_row(ui, session, self.scope, property, label);
        let _id = ui.push_id(name);
        let Some(draft) = self.drafts.get_mut(&property.name) else {
            return;
        };
        ui.set_next_item_width(-1.0);
        let enter = ui
            .input_text("##value", &mut draft.text)
            .enter_returns_true(true)
            .build();
        let commit = enter || ui.is_item_deactivated_after_edit();

        if commit && draft.text != draft.committed {
            let value = text_value(kind, &draft.text);
            commit_value(session, self.scope, property.name.clone(), value, None, draft);
        } else if commit {
            draft.error = None;
        }
        draw_draft_error(ui, draft);
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_asset_property(
        &mut self, ui: &Ui, session: &mut Session, snapshot: &InspectorSnapshot, name: &str, label: &str,
        kind: TextPropertyKind, warning: Option<&str>,
    ) {
        let Some(property) = self.shown(snapshot, name) else {
            return;
        };
        if !text_kind_matches(kind, &property.value) {
            self.draw_raw_property(ui, session, property, label);

            return;
        }

        begin_property_row(ui, session, self.scope, property, label);
        let _id = ui.push_id(name);
        let Some(draft) = self.drafts.get_mut(&property.name) else {
            return;
        };
        ui.set_next_item_width(-1.0);
        let enter = ui
            .input_text("##value", &mut draft.text)
            .enter_returns_true(true)
            .build();
        let commit = enter || ui.is_item_deactivated_after_edit();
        if commit && draft.text != draft.committed {
            let value = text_value(kind, &draft.text);
            commit_value(session, self.scope, property.name.clone(), value, None, draft);
        } else if commit {
            draft.error = None;
        }

        if let Some(warning) = warning {
            ui.text_disabled(warning);
        }
        draw_draft_error(ui, draft);
    }

    fn draw_int_property(
        &mut self, ui: &Ui, session: &mut Session, snapshot: &InspectorSnapshot, name: &str, label: &str,
        range: Option<(i32, i32)>,
    ) {
        let Some(property) = self.shown(snapshot, name) else {
            return;
        };
        let Some(number) = property.value.as_num() else {
            self.draw_raw_property(ui, session, property, label);

            return;
        };

        begin_property_row(ui, session, self.scope, property, label);
        let _id = ui.push_id(name);
        ui.set_next_item_width(-1.0);
        let mut value = number as i32;
        let changed = match range {
            Some((min, max)) => ui
                .drag_int_config("##value")
                .range(min, max)
                .flags(DragFlags::ALWAYS_CLAMP)
                .build(ui, &mut value),
            None => ui.drag_int("##value", &mut value),
        };
        if changed {
            let group = drag_group(&mut self.active_drag, &property.name);
            commit_int_property(session, self.scope, self.transform_mode, &property.name, value, group);
        }
        if ui.is_item_deactivated_after_edit() {
            end_drag(&mut self.active_drag, &property.name);
        }
    }

    fn draw_float_property(
        &mut self, ui: &Ui, session: &mut Session, snapshot: &InspectorSnapshot, name: &str, label: &str,
    ) {
        let Some(property) = self.shown(snapshot, name) else {
            return;
        };
        let Some(mut value) = property.value.as_num() else {
            self.draw_raw_property(ui, session, property, label);

            return;
        };

        begin_property_row(ui, session, self.scope, property, label);
        let _id = ui.push_id(name);
        ui.set_next_item_width(-1.0);
        if ui.drag_float("##value", &mut value) {
            let group = drag_group(&mut self.active_drag, &property.name);
            session.edit_instance_vars_in(
                self.scope,
                format!("change {}", property.name),
                &[VarMutation::Set(property.name.clone(), Value::Num(value))],
                Some(group),
            );
        }
        if ui.is_item_deactivated_after_edit() {
            end_drag(&mut self.active_drag, &property.name);
        }
    }

    fn draw_direction_property(&mut self, ui: &Ui, session: &mut Session, snapshot: &InspectorSnapshot) {
        let Some(property) = self.shown(snapshot, vars::DIR) else {
            return;
        };
        let Some(number) = property.value.as_num() else {
            self.draw_raw_property(ui, session, property, "Direction");

            return;
        };

        let bits = number as u32;
        let count = snapshot
            .icon_states
            .iter()
            .find(|(name, _)| name == &snapshot.text(vars::ICON_STATE))
            .map_or(8, |(_, dirs)| match dirs {
                1 => 1,
                4 => 4,
                8 => 8,
                _ => 8,
            });
        let directions = direction_choices(count, snapshot.declared_directions, snapshot.directional_types);
        let current = selected_direction(bits, snapshot.directional_types);
        let preview = current
            .map(direction_label)
            .map(str::to_string)
            .unwrap_or_else(|| format!("Custom ({number})"));

        begin_property_row(ui, session, self.scope, property, "Direction");
        let _id = ui.push_id(vars::DIR);
        ui.set_next_item_width(-1.0);
        if let Some(combo) = ui.begin_combo("##value", &preview) {
            for direction in directions {
                let value = direction.to_bits();
                if ui
                    .selectable_config(direction_label(direction))
                    .selected(current == Some(direction))
                    .build()
                {
                    if snapshot.directional_types.is_some() {
                        session.set_directional_type_in(self.scope, direction, None);
                    } else {
                        session.edit_instance_vars_in(
                            self.scope,
                            "set dir",
                            &[VarMutation::Set(property.name.clone(), Value::Num(value as f32))],
                            None,
                        );
                    }
                }
            }
            combo.end();
        }
    }

    fn draw_color_property(&mut self, ui: &Ui, session: &mut Session, snapshot: &InspectorSnapshot) {
        let Some(property) = self.shown(snapshot, vars::COLOR) else {
            return;
        };
        let color = match &property.value {
            Value::Null => Some([1.0; 4]),
            Value::Text(text) => render::color::parse(text),
            _ => {
                self.draw_raw_property(ui, session, property, "Color");

                return;
            },
        };

        begin_property_row(ui, session, self.scope, property, "Color");
        let _id = ui.push_id(vars::COLOR);
        if let Some(mut color) = color {
            ui.set_next_item_width(-1.0);
            if ui.color_edit4("##picker", &mut color) {
                let group = drag_group(&mut self.active_drag, &property.name);
                session.edit_instance_vars_in(
                    self.scope,
                    "change color",
                    &[VarMutation::Set(
                        property.name.clone(),
                        Value::Text(format_color(color)),
                    )],
                    Some(group),
                );
            }
            if ui.is_item_deactivated_after_edit() {
                end_drag(&mut self.active_drag, &property.name);
            }
        } else {
            ui.text_disabled("Color picker unavailable for this value");
        }

        if let Some(draft) = self.drafts.get_mut(&property.name) {
            ui.set_next_item_width(-1.0);
            let enter = ui
                .input_text("##text", &mut draft.text)
                .enter_returns_true(true)
                .build();
            let commit = enter || ui.is_item_deactivated_after_edit();
            if commit && draft.text != draft.committed {
                let value = text_value(TextPropertyKind::NullableText, &draft.text);
                commit_value(session, self.scope, property.name.clone(), value, None, draft);
            } else if commit {
                draft.error = None;
            }
            draw_draft_error(ui, draft);
        }
    }

    fn draw_raw_property(&mut self, ui: &Ui, session: &mut Session, property: &InspectorProperty, label: &str) {
        begin_property_row(ui, session, self.scope, property, label);
        let _id = ui.push_id(property.name.as_str());
        if let Some(draft) = self.drafts.get_mut(&property.name) {
            draw_expression_input(ui, session, self.scope, property.name.clone(), draft);
        }
    }

    fn shown<'a>(&self, snapshot: &'a InspectorSnapshot, name: &str) -> Option<&'a InspectorProperty> {
        snapshot
            .property(name)
            .filter(|property| self.filter.shows(name, property.overridden))
    }

    fn shows_any(&self, snapshot: &InspectorSnapshot, names: &[&str]) -> bool {
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
        self.selected = None;
        self.drafts.clear();
        self.active_drag = None;
    }

    fn sync(&mut self, snapshot: &InspectorSnapshot) {
        if self.selected != Some(snapshot.selected) {
            self.drafts.clear();
            self.active_drag = None;
            self.selected = Some(snapshot.selected);
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
    fn new(source: &str, value: &Value) -> Self {
        Self {
            text: source.to_string(),
            committed: source.to_string(),
            error: None,
            typed: TypedDraft::new(value),
            path_query: String::new(),
            raw: false,
        }
    }

    fn sync(&mut self, source: &str, value: &Value) {
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
    fn new(value: &Value) -> Self {
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

fn drag_group(active_drag: &mut Option<(Identifier, EditGroupId)>, name: &Identifier) -> EditGroupId {
    if let Some((active, group)) = active_drag
        && active == name
    {
        return *group;
    }

    let group = EditGroupId::new();
    *active_drag = Some((name.clone(), group));

    group
}

fn end_drag(active_drag: &mut Option<(Identifier, EditGroupId)>, name: &Identifier) {
    if active_drag.as_ref().is_some_and(|(active, _)| active == name) {
        *active_drag = None;
    }
}

impl InspectorSnapshot {
    fn property(&self, name: &str) -> Option<&InspectorProperty> { self.properties.get(&Identifier::from(name)) }

    fn text(&self, name: &str) -> String {
        self.property(name)
            .and_then(|property| property.value.as_text())
            .unwrap_or_default()
            .to_string()
    }
}

impl InspectorFilter {
    fn is_active(&self) -> bool { self.modified_only || !self.query.trim().is_empty() }

    fn shows(&self, name: &str, overridden: bool) -> bool {
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

type SplitVariables<'a> = (Vec<&'a InspectorVariable>, Vec<&'a InspectorVariable>);

fn split_pinned<'a>(variables: &'a [InspectorVariable], pins: &[String]) -> SplitVariables<'a> {
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

fn inspector_snapshot(session: &Session) -> Option<InspectorSnapshot> {
    let selected = session.selected_instance()?;
    let location = session.selected_location()?;
    let prefab = session.selected_prefab()?;
    let tree = session.tree();
    let type_id = tree.and_then(|tree| tree.id_of(&prefab.path));
    let is_atom = matches!((tree, type_id), (Some(tree), Some(id)) if tree.roots().atom.is_some_and(|atom| tree.is_subtype_of(id, atom)));
    let is_movable = matches!((tree, type_id), (Some(tree), Some(id)) if tree.roots().movable.is_some_and(|movable| tree.is_subtype_of(id, movable)));
    let mut special = HashSet::new();
    let mut properties = HashMap::new();

    if is_atom {
        for name in DISPLAY_PROPERTIES {
            special.insert(Identifier::from(*name));
            let property = property_snapshot(tree, type_id, prefab, name);
            properties.insert(property.name.clone(), property);
        }
    }
    if is_movable {
        for name in MOVABLE_PROPERTIES {
            special.insert(Identifier::from(*name));
            let property = property_snapshot(tree, type_id, prefab, name);
            properties.insert(property.name.clone(), property);
        }
    }

    let variables = inspector_variables(tree, prefab, &special);
    let icon = properties
        .get(&Identifier::from(vars::ICON))
        .and_then(|property| property.value.as_text())
        .unwrap_or_default()
        .to_string();
    let icon_state = properties
        .get(&Identifier::from(vars::ICON_STATE))
        .and_then(|property| property.value.as_text())
        .unwrap_or_default()
        .to_string();
    let metadata = session.icon_metadata(&icon);
    let directional_types = session.selected_directional_types();
    let declared_directions = session.declared_directions(&prefab.path);
    let icon_states = metadata
        .map(|metadata| {
            metadata
                .states
                .iter()
                .map(|state| (state.name.clone(), state.dirs))
                .collect()
        })
        .unwrap_or_default();

    Some(InspectorSnapshot {
        selected,
        groups: variable_groups(tree, type_id, &prefab.path, &variables, |id| session.type_source(id)),
        location,
        is_atom,
        is_movable,
        properties,
        variables,
        icon_states,
        declared_directions,
        directional_types,
        icon_known: icon.is_empty() || metadata.is_some(),
        icon_state_known: icon_state.is_empty()
            || metadata.is_some_and(|metadata| metadata.find(&icon_state).is_some()),
    })
}

fn property_snapshot(
    tree: Option<&ObjectTree>, type_id: Option<TypeId>, prefab: &Prefab, name: &str,
) -> InspectorProperty {
    let name = Identifier::from(name);
    let resolved = match (tree, type_id) {
        (Some(tree), Some(id)) => visual::resolve_value(tree, id, prefab, &name),
        _ => None,
    };
    let overridden = resolved.is_some_and(|resolved| resolved.origin == visual::ValueOrigin::Instance)
        || resolved.is_none() && prefab.var(&name).is_some();
    let inherited = resolved
        .and_then(|resolved| resolved.inherited.cloned())
        .unwrap_or_else(|| builtin_default(name.as_str()));
    let value = resolved
        .map(|resolved| resolved.value.clone())
        .or_else(|| prefab.var(&name).cloned())
        .unwrap_or_else(|| inherited.clone());
    let source = prefab
        .vars
        .iter()
        .find(|(candidate, _)| candidate == &name)
        .and_then(|(_, variable)| variable.verbatim())
        .map(str::to_string)
        .unwrap_or_else(|| display_source(&value));
    let editor_text = property_editor_text(name.as_str(), &value, &source);

    InspectorProperty {
        name,
        value,
        inherited,
        editor_text,
        overridden,
    }
}

fn builtin_default(name: &str) -> Value {
    match name {
        vars::DIR | vars::LAYER => Value::Num(2.0),
        vars::ALPHA => Value::Num(255.0),
        vars::PIXEL_X
        | vars::PIXEL_Y
        | vars::PIXEL_W
        | vars::PIXEL_Z
        | vars::PLANE
        | vars::INVISIBILITY
        | vars::STEP_X
        | vars::STEP_Y => Value::Num(0.0),
        _ => Value::Null,
    }
}

fn property_editor_text(name: &str, value: &Value, source: &str) -> String {
    match (name, value) {
        (vars::NAME | vars::ICON_STATE | vars::COLOR, Value::Text(text)) | (vars::ICON, Value::Resource(text)) => {
            text.clone()
        },
        (vars::NAME | vars::ICON | vars::ICON_STATE | vars::COLOR, Value::Null) => String::new(),
        _ => source.to_string(),
    }
}

fn display_source(value: &Value) -> String {
    match value {
        Value::Unevaluated => String::from("<runtime>"),
        value => format_value(value),
    }
}

fn typed_value_text(value: &Value) -> String {
    match value {
        Value::Text(text) | Value::Resource(text) => text.clone(),
        _ => String::new(),
    }
}

fn list_entry_drafts(value: &Value) -> Vec<ListEntryDraft> {
    match value {
        Value::List(entries) => entries
            .iter()
            .map(|entry| ListEntryDraft {
                key: format_value(&entry.key),
                value: entry.value.as_ref().map_or_else(String::new, format_value),
            })
            .collect::<Vec<_>>(),
        _ => Vec::new(),
    }
}

fn parse_list_entries(entries: &[ListEntryDraft]) -> Result<Value, String> {
    let entries = entries
        .iter()
        .map(|entry| {
            let key = parse_value(&entry.key).map_err(|error| error.kind.to_string())?;
            let value = (!entry.value.trim().is_empty())
                .then(|| parse_value(&entry.value).map_err(|error| error.kind.to_string()))
                .transpose()?;
            Ok(ListEntry { key, value })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(Value::List(entries))
}

fn list_is_constant(entries: &[ListEntry]) -> bool {
    fn constant(value: &Value) -> bool {
        match value {
            Value::Unevaluated => false,
            Value::List(entries) => list_is_constant(entries),
            _ => true,
        }
    }

    entries
        .iter()
        .all(|entry| constant(&entry.key) && entry.value.as_ref().is_none_or(constant))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VariableWidget {
    Number,
    Boolean,
    Text,
    Resource,
    Path,
    List,
    Raw,
}

fn variable_widget(variable: &InspectorVariable) -> VariableWidget {
    // `list("a" = newlist(/obj/item))`
    if matches!(variable.value, Value::List(_)) && variable.source.contains("newlist(") {
        return VariableWidget::Raw;
    }

    let Some(ty) = variable.ty.as_ref().filter(|ty| !ty.kinds.is_empty()) else {
        return match &variable.value {
            Value::Num(_) => VariableWidget::Number,
            Value::Text(_) => VariableWidget::Text,
            Value::Resource(_) => VariableWidget::Resource,
            Value::Path(_) => VariableWidget::Path,
            Value::List(entries) if list_is_constant(entries) => VariableWidget::List,
            Value::List(_) | Value::Null | Value::Unevaluated => VariableWidget::Raw,
        };
    };

    let kind = match &variable.value {
        Value::Num(0.0 | 1.0) if ty.kinds.contains(&VarTypeKind::Bool) => Some(VarTypeKind::Bool),
        Value::Num(_) => Some(VarTypeKind::Number),
        Value::Text(_) => Some(VarTypeKind::Text),
        Value::Resource(_) => Some(VarTypeKind::Resource),
        Value::Path(_) => ty
            .kinds
            .iter()
            .copied()
            .find(|kind| matches!(kind, VarTypeKind::Path | VarTypeKind::Object(_))),
        Value::List(entries) if list_is_constant(entries) => Some(VarTypeKind::List),
        Value::List(_) => None,
        Value::Null if ty.nullable => ty.single(),
        Value::Null | Value::Unevaluated => None,
    };
    let Some(kind) = kind.filter(|kind| ty.kinds.contains(kind)) else {
        return VariableWidget::Raw;
    };

    match kind {
        VarTypeKind::Bool if matches!(variable.value, Value::Num(_)) => VariableWidget::Boolean,
        VarTypeKind::Number if matches!(variable.value, Value::Num(_)) => VariableWidget::Number,
        VarTypeKind::Text => VariableWidget::Text,
        VarTypeKind::Resource => VariableWidget::Resource,
        VarTypeKind::Path | VarTypeKind::Object(_) => VariableWidget::Path,
        VarTypeKind::List => VariableWidget::List,
        _ => VariableWidget::Raw,
    }
}

fn inspector_variables(
    tree: Option<&ObjectTree>, prefab: &Prefab, excluded: &HashSet<Identifier>,
) -> Vec<InspectorVariable> {
    let declaration = tree.and_then(|tree| Some((tree, tree.id_of(&prefab.path)?)));
    let mut variables = prefab
        .vars
        .iter()
        .filter(|(name, _)| !excluded.contains(name))
        .map(|(name, variable)| InspectorVariable {
            name: name.clone(),
            source: variable
                .verbatim()
                .map_or_else(|| format_value(&variable.value), str::to_string),
            overridden: true,
            value: variable.value.clone(),
            ty: declaration.and_then(|(tree, id)| tree.resolved_var_type(id, name).cloned()),
            owner: declaration.and_then(|(tree, id)| declaring_type(tree, id, name)),
        })
        .collect::<Vec<_>>();
    let overridden = prefab.vars.iter().map(|(name, _)| name.clone()).collect::<HashSet<_>>();
    let mut defaults = HashMap::<Identifier, (Value, Option<ResolvedVarType>, Option<TypeId>)>::new();

    if let Some((tree, id)) = declaration {
        for declaration in tree.ancestors(id) {
            for (name, variable) in &declaration.vars {
                let modifiers = variable.modifiers;
                if excluded.contains(name)
                    || overridden.contains(name)
                    || modifiers.is_const
                    || modifiers.is_global
                    || modifiers.is_static
                    || modifiers.is_tmp
                {
                    continue;
                }

                defaults.entry(name.clone()).or_insert_with(|| {
                    (
                        variable.value.clone(),
                        variable.resolved_type.clone(),
                        declaring_type(tree, id, name),
                    )
                });
            }
        }
    }

    variables.extend(
        defaults
            .into_iter()
            .map(|(name, (value, ty, owner))| InspectorVariable {
                name,
                source: display_source(&value),
                overridden: false,
                value,
                ty,
                owner,
            }),
    );
    variables.sort_by(|left, right| left.name.as_str().cmp(right.name.as_str()));

    variables
}

fn declaring_type(tree: &ObjectTree, id: TypeId, name: &Identifier) -> Option<TypeId> {
    let mut farthest = None;
    for decl in tree.ancestors(id) {
        let Some(variable) = decl.vars.get(name) else {
            continue;
        };

        if variable.declared {
            return Some(decl.id);
        }

        farthest = Some(decl.id);
    }

    farthest
}

fn variable_groups(
    tree: Option<&ObjectTree>, type_id: Option<TypeId>, path: &TreePath, variables: &[InspectorVariable],
    source: impl Fn(TypeId) -> Option<SourceLocation>,
) -> Vec<VariableGroup> {
    let mut groups = Vec::new();
    if let (Some(tree), Some(id)) = (tree, type_id) {
        groups.extend(
            tree.ancestors(id)
                .filter(|decl| decl.id == id || variables.iter().any(|variable| variable.owner == Some(decl.id)))
                .map(|decl| VariableGroup {
                    owner: Some(decl.id),
                    path: decl.path.to_string(),
                    source: source(decl.id),
                }),
        );
    }

    if groups.is_empty() || variables.iter().any(|variable| variable.owner.is_none()) {
        groups.push(VariableGroup {
            owner: None,
            path: if groups.is_empty() {
                path.to_string()
            } else {
                String::from("Undeclared")
            },
            source: None,
        });
    }

    groups
}

fn draw_identical_banner(ui: &Ui, session: &Session) -> bool {
    let count = session.identical().len();
    let other_levels = session.state.active_document().map_or(0, |document| {
        session
            .identical()
            .iter()
            .filter_map(|id| document.instance_location(*id))
            .filter(|location| location.coord.z != document.z)
            .count()
    });
    let [r, g, b, _] = IDENTICAL_EDIT_COLOR;
    let _background = ui.push_style_color(StyleColor::ChildBg, [r, g, b, 0.14]);
    let _border = ui.push_style_color(StyleColor::Border, IDENTICAL_EDIT_COLOR);
    let mut stop = false;
    ui.child_window("identical-edit-banner")
        .child_flags(ChildFlags::BORDERS | ChildFlags::AUTO_RESIZE_Y)
        .build(ui, || {
            let noun = if count == 1 { "object" } else { "objects" };
            text_wrapped_colored(
                ui,
                IDENTICAL_EDIT_COLOR,
                &format!("Editing all {count} identical {noun}"),
            );
            let reach = match other_levels {
                0 => String::from("Every change below applies to each one, outlined on the map."),
                other => format!("Every change below applies to each one, {other} of them on other levels."),
            };
            text_wrapped_colored(ui, ui.style_color(StyleColor::TextDisabled), &reach);
            stop = ui.button("Edit only this one");
        });

    stop
}

/// Tints every value field while an edit reaches more than the selected object
fn identical_frame_colors(ui: &Ui) -> [dear_imgui_rs::ColorStackToken<'_>; 3] {
    let tint = |base: StyleColor, strength: f32| {
        let base = ui.style_color(base);
        let mut mixed = [0.0; 4];
        for channel in 0..3 {
            mixed[channel] = base[channel] + (IDENTICAL_EDIT_COLOR[channel] - base[channel]) * strength;
        }
        mixed[3] = base[3].max(0.6);

        mixed
    };

    let frame = ui.push_style_color(StyleColor::FrameBg, tint(StyleColor::FrameBg, 0.25));
    let hovered = ui.push_style_color(StyleColor::FrameBgHovered, tint(StyleColor::FrameBgHovered, 0.35));
    let active = ui.push_style_color(StyleColor::FrameBgActive, tint(StyleColor::FrameBgActive, 0.45));

    // Arrays drop front to back, and the pushes have to pop in reverse
    [active, hovered, frame]
}

fn property_table(ui: &Ui, id: &str, content: impl FnOnce(&Ui)) {
    ui.table(id)
        .flags(TableFlags::BORDERS_INNER_V | TableFlags::RESIZABLE)
        .sizing_policy(TableSizingPolicy::StretchProp)
        .column("Property")
        .weight(0.4)
        .done()
        .column("Value")
        .weight(0.6)
        .done()
        .build(content);
}

fn draw_changed_text(ui: &Ui, text: &str, is_changed: bool) {
    let _color = is_changed.then(|| ui.push_style_color(StyleColor::Text, ui.style_color(StyleColor::PlotHistogram)));
    ui.text(text);
}

fn begin_property_row(ui: &Ui, session: &mut Session, scope: EditScope, property: &InspectorProperty, label: &str) {
    ui.table_next_row();
    ui.table_next_column();
    ui.align_text_to_frame_padding();
    draw_changed_text(ui, label, property.overridden);
    let tooltip = ADVANCED_OFFSETS
        .contains(&property.name.as_str())
        .then_some("Pixel W/Z are map-format axes. In the current top-down view they add to Pixel X/Y.")
        .into_iter()
        .chain(property.overridden.then_some("Right-click to reset"))
        .collect::<Vec<_>>()
        .join("\n");
    if !tooltip.is_empty() {
        ui.set_item_tooltip(tooltip);
    }

    {
        let _id = ui.push_id(property.name.as_str());
        if let Some(_popup) = ui.begin_popup_context_item_with_label(Some("property-menu")) {
            draw_reset_item(ui, session, scope, &property.name, property.overridden, || {
                format!(
                    "Remove the map override and restore {}",
                    display_source(&property.inherited)
                )
            });
        }
    }

    ui.table_next_column();
}

fn draw_reset_item(
    ui: &Ui, session: &mut Session, scope: EditScope, name: &Identifier, is_changed: bool,
    hint: impl FnOnce() -> String,
) {
    if ui.menu_item_enabled_selected_no_shortcut("Reset", false, is_changed) {
        session.edit_instance_vars_in(
            scope,
            format!("reset {name}"),
            &[VarMutation::Remove(name.clone())],
            None,
        );
    }

    if is_changed {
        ui.set_item_tooltip(hint());
    }
}

fn text_kind_matches(kind: TextPropertyKind, value: &Value) -> bool {
    match kind {
        TextPropertyKind::Text | TextPropertyKind::NullableText => matches!(value, Value::Text(_) | Value::Null),
        TextPropertyKind::Resource => matches!(value, Value::Resource(_) | Value::Null),
    }
}

fn text_value(kind: TextPropertyKind, text: &str) -> Value {
    match kind {
        TextPropertyKind::Text => Value::Text(text.to_string()),
        TextPropertyKind::Resource if text.trim().is_empty() => Value::Null,
        TextPropertyKind::Resource => Value::Resource(text.to_string()),
        TextPropertyKind::NullableText if text.trim().is_empty() => Value::Null,
        TextPropertyKind::NullableText => Value::Text(text.to_string()),
    }
}

fn transform_axis(mode: TransformMode, name: &Identifier) -> Option<usize> {
    let (x, y) = mode.variables();

    if name.as_str() == x {
        Some(0)
    } else if name.as_str() == y {
        Some(1)
    } else {
        None
    }
}

fn commit_int_property(
    session: &mut Session, scope: EditScope, mode: TransformMode, name: &Identifier, value: i32, group: EditGroupId,
) {
    let label = format!("change {name}");
    let mut committed = value;
    let mut to_coord = None;

    // Each identical instance keeps its own tile, so their offsets are set as typed
    if scope == EditScope::Selected
        && let Some(axis) = transform_axis(mode, name)
        && let Some(transform) = session.selected_transform()
        && let Some(location) = session.selected_location()
    {
        let current = match mode {
            TransformMode::Pixel => transform.pixel[axis],
            TransformMode::Step => transform.step[axis],
        };
        let anchor = if axis == 0 {
            transform.sprite.x
        } else {
            transform.sprite.y
        };
        let (origin, limit) = if axis == 0 {
            (location.coord.x, session.map().map_or(1, |map| map.size().x.max(1)))
        } else {
            (location.coord.y, session.map().map_or(1, |map| map.size().y.max(1)))
        };
        let (target, adjust) = anchor_axis(
            anchor + (value - current) as f32,
            origin,
            session.options.tile_size,
            limit,
        );
        committed += adjust;
        if target != origin {
            to_coord = Some(if axis == 0 {
                Coord::new(target, location.coord.y, location.coord.z)
            } else {
                Coord::new(location.coord.x, target, location.coord.z)
            });
        }
    }

    let mutation = VarMutation::Set(name.clone(), Value::Num(committed as f32));
    match to_coord {
        Some(coord) => {
            session.move_selected_instance(coord, label, &[mutation], Some(group));
        },
        None => {
            session.edit_instance_vars_in(scope, label, &[mutation], Some(group));
        },
    }
}

fn commit_value(
    session: &mut Session, scope: EditScope, name: Identifier, value: Value, group: Option<EditGroupId>,
    draft: &mut VariableDraft,
) {
    let typed = TypedDraft::new(&value);
    if session
        .edit_instance_vars_in(scope, format!("set {name}"), &[VarMutation::Set(name, value)], group)
        .is_some()
    {
        draft.committed.clone_from(&draft.text);
        draft.typed = typed;
        draft.error = None;
    } else {
        draft.error = Some(String::from("Selected object is no longer available"));
    }
}

fn draw_expression_input(
    ui: &Ui, session: &mut Session, scope: EditScope, name: Identifier, draft: &mut VariableDraft,
) {
    ui.set_next_item_width(-1.0);
    let enter = ui
        .input_text("##value", &mut draft.text)
        .enter_returns_true(true)
        .build();
    let commit = enter || ui.is_item_deactivated_after_edit();

    if commit && draft.text != draft.committed {
        match parse_value(&draft.text) {
            Ok(value) => {
                let canonical = format_value(&value);
                let label = format!("set {name}");
                if session
                    .edit_instance_vars_in(scope, label, &[VarMutation::Set(name, value)], None)
                    .is_some()
                {
                    draft.text.clone_from(&canonical);
                    draft.committed = canonical;
                    draft.error = None;
                } else {
                    draft.error = Some(String::from("Selected object is no longer available"));
                }
            },
            Err(error) => draft.error = Some(error.kind.to_string()),
        }
    } else if commit {
        draft.error = None;
    }
    draw_draft_error(ui, draft);
}

fn draw_draft_error(ui: &Ui, draft: &VariableDraft) {
    if let Some(error) = &draft.error {
        let _style = ui.push_style_color(StyleColor::Text, [1.0, 0.35, 0.35, 1.0]);
        ui.text_wrapped(error);
    }
}

fn direction_label(direction: Dir) -> &'static str {
    match direction {
        Dir::South => "South",
        Dir::North => "North",
        Dir::East => "East",
        Dir::West => "West",
        Dir::Southeast => "Southeast",
        Dir::Southwest => "Southwest",
        Dir::Northeast => "Northeast",
        Dir::Northwest => "Northwest",
    }
}

fn direction_choices(
    count: usize, declared: Option<[bool; 8]>, directional_types: Option<DirectionalTypes>,
) -> Vec<Dir> {
    if let Some(supported) = directional_types.map(|types| types.supported).or(declared) {
        return Dir::ORDER
            .into_iter()
            .zip(supported)
            .filter_map(|(direction, supported)| supported.then_some(direction))
            .collect();
    }

    Dir::ORDER.into_iter().take(count).collect()
}

fn selected_direction(bits: u32, directional_types: Option<DirectionalTypes>) -> Option<Dir> {
    directional_types
        .and_then(|types| types.current)
        .or_else(|| Dir::from_bits(bits))
}

fn format_color(color: [f32; 4]) -> String {
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    let [red, green, blue, alpha] = color.map(channel);

    if alpha == u8::MAX {
        format!("#{red:02X}{green:02X}{blue:02X}")
    } else {
        format!("#{red:02X}{green:02X}{blue:02X}{alpha:02X}")
    }
}

fn draw_variable_rows(
    ui: &Ui, session: &mut Session, state: &mut InspectorState, pins: &mut Vec<String>,
    variables: &[&InspectorVariable],
) {
    for variable in variables {
        if !state.filter.shows(variable.name.as_str(), variable.overridden) {
            continue;
        }

        let Some(draft) = state.drafts.get_mut(&variable.name) else {
            continue;
        };

        let _id = ui.push_id(variable.name.as_str());

        ui.table_next_row();
        ui.table_next_column();
        ui.align_text_to_frame_padding();
        if draw_pin(ui, pins.iter().any(|pin| pin == variable.name.as_str())) {
            toggle_pin(pins, variable.name.as_str());
        }

        ui.same_line();
        draw_changed_text(ui, variable.name.as_str(), variable.overridden);
        ui.set_item_tooltip(format!(
            "{}\nRight-click for more",
            type_label(session.tree(), variable.ty.as_ref())
        ));
        draw_variable_menu(ui, session, state.scope, variable, draft);
        ui.table_next_column();
        draw_variable_input(ui, session, state.scope, &mut state.active_drag, variable, draft);
    }
}

fn draw_variable_input(
    ui: &Ui, session: &mut Session, scope: EditScope, active_drag: &mut Option<(Identifier, EditGroupId)>,
    variable: &InspectorVariable, draft: &mut VariableDraft,
) {
    let widget = variable_widget(variable);
    if widget == VariableWidget::Raw || draft.raw {
        draw_expression_input(ui, session, scope, variable.name.clone(), draft);
        return;
    }

    match widget {
        VariableWidget::Number => {
            let Value::Num(mut number) = variable.value else {
                return;
            };

            ui.set_next_item_width(-1.0);
            if ui.drag_float("##value", &mut number) {
                let group = drag_group(active_drag, &variable.name);
                commit_value(
                    session,
                    scope,
                    variable.name.clone(),
                    Value::Num(number),
                    Some(group),
                    draft,
                );
            }

            if ui.is_item_deactivated_after_edit() {
                end_drag(active_drag, &variable.name);
            }
        },
        VariableWidget::Boolean => {
            let Value::Num(number) = variable.value else {
                return;
            };

            let mut is_checked = number != 0.0;
            if ui.checkbox("##value", &mut is_checked) {
                commit_value(
                    session,
                    scope,
                    variable.name.clone(),
                    Value::Num(f32::from(u8::from(is_checked))),
                    None,
                    draft,
                );
            }
        },
        VariableWidget::Text | VariableWidget::Resource => {
            ui.set_next_item_width(-1.0);
            let is_entered = ui
                .input_text("##value", &mut draft.typed.text)
                .enter_returns_true(true)
                .build();
            if is_entered || ui.is_item_deactivated_after_edit() {
                let value = match widget {
                    VariableWidget::Text => Value::Text(draft.typed.text.clone()),
                    _ => Value::Resource(draft.typed.text.clone()),
                };
                if value != variable.value {
                    commit_value(session, scope, variable.name.clone(), value, None, draft);
                }
            }
        },
        VariableWidget::Path => {
            let root = variable.ty.as_ref().and_then(|ty| {
                ty.kinds.iter().find_map(|kind| match kind {
                    VarTypeKind::Object(id) => Some(*id),
                    _ => None,
                })
            });
            if let Value::Path(path) = &variable.value
                && let Some(root) = root
                && !session
                    .tree()
                    .and_then(|tree| tree.id_of(path).map(|id| tree.is_subtype_of(id, root)))
                    .unwrap_or(false)
            {
                draw_expression_input(ui, session, scope, variable.name.clone(), draft);
                return;
            }

            let current = match &variable.value {
                Value::Path(path) => Some(path.to_string()),
                _ => None,
            };
            ui.set_next_item_width(-1.0);
            if let Some(combo) = ui.begin_combo("##value", current.as_deref().unwrap_or("<null>")) {
                if ui.is_window_appearing() {
                    draft.path_query = current.unwrap_or_default();
                }

                let picked = draw_type_path_search(
                    ui,
                    session.tree(),
                    &mut draft.path_query,
                    "##path-search",
                    50,
                    |tree, path| {
                        root.is_none_or(|root| tree.id_of(path).is_some_and(|id| tree.is_subtype_of(id, root)))
                    },
                    |_| true,
                );
                if let Some(path) = picked
                    && Value::Path(path.clone()) != variable.value
                {
                    commit_value(session, scope, variable.name.clone(), Value::Path(path), None, draft);
                }

                combo.end();
            }
        },
        VariableWidget::List => draw_list_input(ui, session, scope, variable, draft),
        VariableWidget::Raw => unreachable!(),
    }

    draw_draft_error(ui, draft);
}

fn draw_variable_menu(
    ui: &Ui, session: &mut Session, scope: EditScope, variable: &InspectorVariable, draft: &mut VariableDraft,
) {
    let Some(_popup) = ui.begin_popup_context_item_with_label(Some("variable-menu")) else {
        return;
    };

    if variable_widget(variable) != VariableWidget::Raw {
        let label = if draft.raw {
            "Use typed editor"
        } else {
            "Edit as expression"
        };
        if ui.menu_item(label) {
            draft.raw = !draft.raw;
        }
    }

    let is_nullable = variable.ty.as_ref().is_some_and(|ty| ty.nullable) && !matches!(variable.value, Value::Null);
    if ui.menu_item_enabled_selected_no_shortcut("Set null", false, is_nullable) {
        commit_value(session, scope, variable.name.clone(), Value::Null, None, draft);
    }

    draw_reset_item(ui, session, scope, &variable.name, variable.overridden, || {
        String::from("Remove the map override")
    });
}

fn type_label(tree: Option<&ObjectTree>, ty: Option<&ResolvedVarType>) -> String {
    let Some(ty) = ty.filter(|ty| !ty.kinds.is_empty()) else {
        return String::from("unknown");
    };

    let mut parts = ty
        .kinds
        .iter()
        .map(|kind| match kind {
            VarTypeKind::Number => String::from("number"),
            VarTypeKind::Bool => String::from("bool"),
            VarTypeKind::Text => String::from("text"),
            VarTypeKind::Resource => String::from("resource"),
            VarTypeKind::Path => String::from("path"),
            VarTypeKind::List => String::from("list"),
            VarTypeKind::Object(id) => tree
                .and_then(|tree| tree.get(*id))
                .map_or_else(|| String::from("object"), |decl| decl.path.to_string()),
        })
        .collect::<Vec<_>>();
    if ty.nullable {
        parts.push(String::from("null"));
    }

    parts.join(" | ")
}

fn draw_list_input(
    ui: &Ui, session: &mut Session, scope: EditScope, variable: &InspectorVariable, draft: &mut VariableDraft,
) {
    let Value::List(entries) = &variable.value else {
        if ui.small_button("Create list") {
            commit_value(
                session,
                scope,
                variable.name.clone(),
                Value::List(Vec::new()),
                None,
                draft,
            );
        }

        return;
    };

    let Some(tree) = ui
        .tree_node_config("##list")
        .label(format!("List ({} entries)", entries.len()))
        .push()
    else {
        return;
    };

    let mut is_changed = false;
    let mut remove = None;
    for (index, entry) in draft.typed.entries.iter_mut().enumerate() {
        let _id = ui.push_id(index);
        ui.set_next_item_width(ui.content_region_avail_width() * 0.5);
        let is_entered = ui.input_text("##key", &mut entry.key).enter_returns_true(true).build();
        is_changed |= is_entered || ui.is_item_deactivated_after_edit();
        if let Some(_popup) = ui.begin_popup_context_item_with_label(Some("entry-menu"))
            && ui.menu_item("Remove")
        {
            remove = Some(index);
        }

        ui.same_line();
        ui.set_next_item_width(-1.0);
        let is_entered = ui
            .input_text("##entry-value", &mut entry.value)
            .hint("no value")
            .enter_returns_true(true)
            .build();
        is_changed |= is_entered || ui.is_item_deactivated_after_edit();
    }

    if let Some(index) = remove {
        draft.typed.entries.remove(index);
        is_changed = true;
    }

    if ui.small_button("Add entry") {
        draft.typed.entries.push(ListEntryDraft {
            key: String::from("null"),
            value: String::new(),
        });
        is_changed = true;
    }

    ui.same_line();
    {
        let _disabled = ui.begin_disabled_with_cond(draft.typed.entries.is_empty());
        if ui.small_button("Remove entry") {
            draft.typed.entries.pop();
            is_changed = true;
        }
    }

    if is_changed {
        match parse_list_entries(&draft.typed.entries) {
            Ok(value) if value == variable.value => {
                draft.typed = TypedDraft::new(&variable.value);
                draft.error = None;
            },
            Ok(value) => commit_value(session, scope, variable.name.clone(), value, None, draft),
            Err(error) => draft.error = Some(error),
        }
    }

    tree.pop();
}

fn draw_pin(ui: &Ui, pinned: bool) -> bool {
    if pinned {
        ui.text(ICON_PIN.to_string());
    } else {
        ui.text_disabled(ICON_PIN_OUTLINE.to_string());
    }

    let clicked = ui.is_item_clicked();
    ui.set_item_tooltip(if pinned { "Unpin" } else { "Pin to the top" });

    clicked
}

fn toggle_pin(pins: &mut Vec<String>, name: &str) {
    if let Some(index) = pins.iter().position(|pin| pin == name) {
        pins.remove(index);
    } else {
        pins.push(name.to_string());
    }
}

pub(super) struct InspectorPanel {
    window: WindowKey,
    state: InspectorState,
}

#[derive(Default)]
pub(super) struct InspectorPanelOutput {
    pub(super) open_source: Option<SourceLocation>,
    pub(super) find_similar: Option<(DocumentId, PrefabInstanceId)>,
    pub(super) copy_hash: Option<String>,
    pub(super) docked: bool,
}

impl InspectorPanel {
    pub(super) fn new() -> Result<Self, WindowKeyError> {
        Ok(Self {
            window: WindowKey::new("inspector", "Inspector")?,
            state: InspectorState::default(),
        })
    }

    pub(super) const fn window(&self) -> &WindowKey { &self.window }

    pub(super) const fn transform_mode(&self) -> TransformMode { self.state.transform_mode() }

    pub(super) fn edits_identical(&self) -> bool { self.state.scope == EditScope::Identical }

    pub(super) fn draw(
        &mut self, ui: &Ui, session: &mut Session, settings: &mut Settings, focus: bool, open: &mut bool,
    ) -> InspectorPanelOutput {
        if !*open {
            return InspectorPanelOutput::default();
        }

        let mut output = InspectorOutput::default();
        let mut docked = false;
        ui.window(&self.window).opened(open).focused(focus).build(|| {
            docked = ui.is_window_docked();
            if settings.focus_windows_on_hover {
                focus_window_on_hover(ui);
            }
            output = self.state.draw(ui, session, &mut settings.pinned_vars);
        });
        let find_similar = output
            .find_similar
            .then(|| session.state.active_document())
            .flatten()
            .and_then(|document| Some((document.id(), document.selected_instance()?)));

        InspectorPanelOutput {
            open_source: output.open_source,
            find_similar,
            copy_hash: output.copy_hash,
            docked,
        }
    }
}

#[cfg(test)]
mod tests {
    use core::{
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
        session.apply_map(crate::loader::LoadedMap {
            path: root.join("identical-test.dmm"),
            map,
            z: 1,
            errors: vec![],
            repo: None,
            conflict: None,
        });
        let document = session.state.active_document().unwrap();
        let tables = [1, 2].map(|x| document.instance_ids_at(Coord::new(x, 1, 1))[1]);
        session.select_instance(Some(tables[0]));

        (session, tables)
    }

    fn pixel_and_tile(session: &Session, id: PrefabInstanceId) -> (Option<Value>, Coord) {
        let (prefab, location) = session.state.active_document().unwrap().prefab_instance(id).unwrap();

        (prefab.var(&"pixel_x".into()).cloned(), location.coord)
    }

    #[test]
    fn the_identical_banner_replaces_the_checkbox_while_the_mode_is_on() {
        let _guard = crate::ui::IMGUI_CONTEXT.lock().unwrap();
        let mut context = crate::ui::fixtures::rectangle_context();
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
        let _guard = crate::ui::IMGUI_CONTEXT.lock().unwrap();
        let mut context = crate::ui::fixtures::rectangle_context();
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
                (Some(Value::Num(40.0)), Coord::new(1, 1, 1)),
                (Some(Value::Num(40.0)), Coord::new(2, 1, 1))
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
            (Some(Value::Num(8.0)), Coord::new(2, 1, 1)),
            "a single object is re-anchored onto the tile it was dragged over"
        );
        assert_eq!(pixel_and_tile(&session, tables[1]), (None, Coord::new(2, 1, 1)));
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
            },
        );
    }
}
