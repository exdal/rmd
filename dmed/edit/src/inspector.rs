use core::types::{Identifier, Value};
use std::collections::{HashMap, HashSet};

use dear_imgui_rs::{DragFlags, StyleColor, TableFlags, TableSizingPolicy, Ui};
use dmi::metadata::Dir;
use dmm::{Prefab, writer::format_value};
use editor::{
    command::EditGroupId,
    document::{PrefabInstanceId, PrefabLocation, VarMutation},
    visual,
};
use objtree::{ObjectTree, TypeId};

use crate::session::Session;

const DISPLAY_PROPERTIES: &[&str] = &[
    "name",
    "icon",
    "icon_state",
    "dir",
    "pixel_x",
    "pixel_y",
    "pixel_w",
    "pixel_z",
    "plane",
    "layer",
    "color",
    "alpha",
    "invisibility",
];
const MOVABLE_PROPERTIES: &[&str] = &[
    "pixel_x", "pixel_y", "pixel_z", "pixel_w", "step_x", "step_y", "step_z", "step_w",
];

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum TransformMode {
    #[default]
    Pixel,
    Step,
}

impl TransformMode {
    const ALL: [Self; 2] = [Self::Pixel, Self::Step];

    const fn label(self) -> &'static str {
        match self {
            Self::Pixel => "Pixel",
            Self::Step => "Step",
        }
    }
}

#[derive(Default)]
pub struct InspectorState {
    selected: Option<PrefabInstanceId>,
    drafts: HashMap<Identifier, VariableDraft>,
    active_drag: Option<(Identifier, EditGroupId)>,
    transform_mode: TransformMode,
}

struct VariableDraft {
    text: String,
    committed: String,
    error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InspectorVariable {
    name: Identifier,
    source: String,
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
    path: String,
    location: PrefabLocation,
    is_atom: bool,
    is_movable: bool,
    properties: HashMap<Identifier, InspectorProperty>,
    overrides: Vec<InspectorVariable>,
    defaults: Vec<InspectorVariable>,
    icon_states: Vec<(String, u32)>,
    icon_known: bool,
    icon_state_known: bool,
}

#[derive(Clone, Copy)]
enum TextPropertyKind {
    Text,
    Resource,
    NullableText,
}

impl InspectorState {
    pub fn draw(&mut self, ui: &Ui, session: &mut Session) {
        let Some(snapshot) = inspector_snapshot(session) else {
            self.clear();
            ui.text_disabled("No object selected");

            return;
        };

        self.sync(&snapshot);
        ui.text_wrapped(&snapshot.path);
        ui.text_disabled(format!(
            "Tile {}, {}, {}",
            snapshot.location.coord.x, snapshot.location.coord.y, snapshot.location.coord.z
        ));
        ui.separator();

        if snapshot.is_atom {
            self.draw_transform(ui, session, &snapshot);
            self.draw_display(ui, session, &snapshot);
        }

        draw_variable_section(
            ui,
            session,
            self,
            "inspector-other-overrides",
            "Other overrides",
            &snapshot.overrides,
        );
        draw_variable_section(
            ui,
            session,
            self,
            "inspector-other-defaults",
            "Other defaults",
            &snapshot.defaults,
        );
    }

    fn draw_transform(&mut self, ui: &Ui, session: &mut Session, snapshot: &InspectorSnapshot) {
        let Some(section) = section(ui, "inspector-transform", "Transform", true) else {
            return;
        };
        if !snapshot.is_movable {
            self.transform_mode = TransformMode::Pixel;
        }

        property_table(ui, "inspector-transform-properties", |ui| {
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
            ui.table_next_column();
            ui.text_disabled("read-only");

            match self.transform_mode {
                TransformMode::Pixel => {
                    self.draw_int_property(ui, session, snapshot, "pixel_x", "Pixel X", None);
                    self.draw_int_property(ui, session, snapshot, "pixel_y", "Pixel Y", None);
                },
                TransformMode::Step => {
                    self.draw_int_property(ui, session, snapshot, "step_x", "Step X", None);
                    self.draw_int_property(ui, session, snapshot, "step_y", "Step Y", None);
                },
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
            ui.table_next_column();
            ui.text_disabled("Editor");
        });

        section.pop();
    }

    fn draw_display(&mut self, ui: &Ui, session: &mut Session, snapshot: &InspectorSnapshot) {
        let Some(display_section) = section(ui, "inspector-display", "Display", true) else {
            return;
        };

        property_table(ui, "inspector-display-properties", |ui| {
            self.draw_text_property(ui, session, snapshot, "name", "Name", TextPropertyKind::Text);
            self.draw_asset_property(
                ui,
                session,
                snapshot,
                "icon",
                "Icon",
                TextPropertyKind::Resource,
                (!snapshot.icon_known).then_some("The current DMI is not loaded"),
            );
            self.draw_asset_property(
                ui,
                session,
                snapshot,
                "icon_state",
                "Icon state",
                TextPropertyKind::Text,
                (!snapshot.icon_state_known).then_some("The current state is not present in the DMI"),
            );
            self.draw_direction_property(ui, session, snapshot);
            self.draw_float_property(ui, session, snapshot, "plane", "Plane");
            self.draw_float_property(ui, session, snapshot, "layer", "Layer");
            self.draw_color_property(ui, session, snapshot);
            self.draw_int_property(ui, session, snapshot, "alpha", "Alpha", Some((0, 255)));
            self.draw_int_property(ui, session, snapshot, "invisibility", "Invisibility", Some((0, 101)));
        });

        if let Some(advanced) = section(ui, "inspector-display-advanced", "Advanced offsets", false) {
            ui.text_wrapped("Pixel W/Z are map-format axes. In the current top-down view they add to Pixel X/Y.");
            property_table(ui, "inspector-display-advanced-properties", |ui| {
                self.draw_int_property(ui, session, snapshot, "pixel_w", "Pixel W", None);
                self.draw_int_property(ui, session, snapshot, "pixel_z", "Pixel Z", None);
            });
            advanced.pop();
        }

        display_section.pop();
    }

    fn draw_text_property(
        &mut self, ui: &Ui, session: &mut Session, snapshot: &InspectorSnapshot, name: &str, label: &str,
        kind: TextPropertyKind,
    ) {
        let Some(property) = snapshot.property(name) else {
            return;
        };
        if !text_kind_matches(kind, &property.value) {
            self.draw_raw_property(ui, session, property, label);

            return;
        }

        begin_property_row(ui, property, label);
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
            commit_value(session, property.name.clone(), value, None, draft);
        } else if commit {
            draft.error = None;
        }
        draw_draft_error(ui, draft);
        draw_reset(ui, session, property);
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_asset_property(
        &mut self, ui: &Ui, session: &mut Session, snapshot: &InspectorSnapshot, name: &str, label: &str,
        kind: TextPropertyKind, warning: Option<&str>,
    ) {
        let Some(property) = snapshot.property(name) else {
            return;
        };
        if !text_kind_matches(kind, &property.value) {
            self.draw_raw_property(ui, session, property, label);

            return;
        }

        begin_property_row(ui, property, label);
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
            commit_value(session, property.name.clone(), value, None, draft);
        } else if commit {
            draft.error = None;
        }

        if let Some(warning) = warning {
            ui.text_disabled(warning);
        }
        draw_draft_error(ui, draft);
        draw_reset(ui, session, property);
    }

    fn draw_int_property(
        &mut self, ui: &Ui, session: &mut Session, snapshot: &InspectorSnapshot, name: &str, label: &str,
        range: Option<(i32, i32)>,
    ) {
        let Some(property) = snapshot.property(name) else {
            return;
        };
        let Some(number) = property.value.as_num() else {
            self.draw_raw_property(ui, session, property, label);

            return;
        };

        begin_property_row(ui, property, label);
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
            let group = self.drag_group(&property.name);
            session.edit_selected_instance_vars(
                format!("change {}", property.name),
                &[VarMutation::Set(property.name.clone(), Value::Num(value as f32))],
                Some(group),
            );
        }
        if ui.is_item_deactivated_after_edit() {
            self.end_drag(&property.name);
        }
        draw_reset(ui, session, property);
    }

    fn draw_float_property(
        &mut self, ui: &Ui, session: &mut Session, snapshot: &InspectorSnapshot, name: &str, label: &str,
    ) {
        let Some(property) = snapshot.property(name) else {
            return;
        };
        let Some(mut value) = property.value.as_num() else {
            self.draw_raw_property(ui, session, property, label);

            return;
        };

        begin_property_row(ui, property, label);
        let _id = ui.push_id(name);
        ui.set_next_item_width(-1.0);
        if ui.drag_float("##value", &mut value) {
            let group = self.drag_group(&property.name);
            session.edit_selected_instance_vars(
                format!("change {}", property.name),
                &[VarMutation::Set(property.name.clone(), Value::Num(value))],
                Some(group),
            );
        }
        if ui.is_item_deactivated_after_edit() {
            self.end_drag(&property.name);
        }
        draw_reset(ui, session, property);
    }

    fn draw_direction_property(&mut self, ui: &Ui, session: &mut Session, snapshot: &InspectorSnapshot) {
        let Some(property) = snapshot.property("dir") else {
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
            .find(|(name, _)| name == &snapshot.text("icon_state"))
            .map_or(8, |(_, dirs)| match dirs {
                1 => 1,
                4 => 4,
                8 => 8,
                _ => 8,
            });
        let preview = Dir::from_bits(bits)
            .map(direction_label)
            .map(str::to_string)
            .unwrap_or_else(|| format!("Custom ({number})"));

        begin_property_row(ui, property, "Direction");
        let _id = ui.push_id("dir");
        ui.set_next_item_width(-1.0);
        if let Some(combo) = ui.begin_combo("##value", &preview) {
            for direction in Dir::ORDER.iter().take(count) {
                let value = direction.to_bits();
                if ui
                    .selectable_config(direction_label(*direction))
                    .selected(value == bits)
                    .build()
                {
                    session.edit_selected_instance_vars(
                        "set dir",
                        &[VarMutation::Set(property.name.clone(), Value::Num(value as f32))],
                        None,
                    );
                }
            }
            combo.end();
        }
        draw_reset(ui, session, property);
    }

    fn draw_color_property(&mut self, ui: &Ui, session: &mut Session, snapshot: &InspectorSnapshot) {
        let Some(property) = snapshot.property("color") else {
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

        begin_property_row(ui, property, "Color");
        let _id = ui.push_id("color");
        if let Some(mut color) = color {
            ui.set_next_item_width(-1.0);
            if ui.color_edit4("##picker", &mut color) {
                let group = self.drag_group(&property.name);
                session.edit_selected_instance_vars(
                    "change color",
                    &[VarMutation::Set(
                        property.name.clone(),
                        Value::Text(format_color(color)),
                    )],
                    Some(group),
                );
            }
            if ui.is_item_deactivated_after_edit() {
                self.end_drag(&property.name);
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
                commit_value(session, property.name.clone(), value, None, draft);
            } else if commit {
                draft.error = None;
            }
            draw_draft_error(ui, draft);
        }
        draw_reset(ui, session, property);
    }

    fn draw_raw_property(&mut self, ui: &Ui, session: &mut Session, property: &InspectorProperty, label: &str) {
        begin_property_row(ui, property, label);
        let _id = ui.push_id(property.name.as_str());
        if let Some(draft) = self.drafts.get_mut(&property.name) {
            draw_expression_input(ui, session, property.name.clone(), draft);
        }
        draw_reset(ui, session, property);
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

        let current = snapshot
            .properties
            .values()
            .map(|property| (property.name.clone(), property.editor_text.clone()))
            .chain(
                snapshot
                    .overrides
                    .iter()
                    .chain(&snapshot.defaults)
                    .map(|variable| (variable.name.clone(), variable.source.clone())),
            )
            .collect::<HashMap<_, _>>();
        self.drafts.retain(|name, _| current.contains_key(name));

        for (name, source) in current {
            match self.drafts.get_mut(&name) {
                Some(draft) if draft.committed != source && draft.text == draft.committed => {
                    draft.text.clone_from(&source);
                    draft.committed = source;
                    draft.error = None;
                },
                Some(_) => {},
                None => {
                    self.drafts.insert(
                        name,
                        VariableDraft {
                            text: source.clone(),
                            committed: source,
                            error: None,
                        },
                    );
                },
            }
        }
    }

    fn drag_group(&mut self, name: &Identifier) -> EditGroupId {
        if let Some((active, group)) = &self.active_drag
            && active == name
        {
            return *group;
        }

        let group = EditGroupId::new();
        self.active_drag = Some((name.clone(), group));

        group
    }

    fn end_drag(&mut self, name: &Identifier) {
        if self.active_drag.as_ref().is_some_and(|(active, _)| active == name) {
            self.active_drag = None;
        }
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

    let (overrides, defaults) = inspector_variables(tree, prefab, &special);
    let icon = properties
        .get(&Identifier::from("icon"))
        .and_then(|property| property.value.as_text())
        .unwrap_or_default()
        .to_string();
    let icon_state = properties
        .get(&Identifier::from("icon_state"))
        .and_then(|property| property.value.as_text())
        .unwrap_or_default()
        .to_string();
    let metadata = session.icon_metadata(&icon);
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
        path: prefab.path.to_string(),
        location,
        is_atom,
        is_movable,
        properties,
        overrides,
        defaults,
        icon_states,
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
        "dir" | "layer" => Value::Num(2.0),
        "alpha" => Value::Num(255.0),
        "pixel_x" | "pixel_y" | "pixel_w" | "pixel_z" | "plane" | "invisibility" | "step_x" | "step_y" => {
            Value::Num(0.0)
        },
        _ => Value::Null,
    }
}

fn property_editor_text(name: &str, value: &Value, source: &str) -> String {
    match (name, value) {
        ("name" | "icon_state" | "color", Value::Text(text)) | ("icon", Value::Resource(text)) => text.clone(),
        ("name" | "icon" | "icon_state" | "color", Value::Null) => String::new(),
        _ => source.to_string(),
    }
}

fn display_source(value: &Value) -> String {
    match value {
        Value::Unevaluated => String::from("<runtime>"),
        value => format_value(value),
    }
}

fn inspector_variables(
    tree: Option<&ObjectTree>, prefab: &Prefab, excluded: &HashSet<Identifier>,
) -> (Vec<InspectorVariable>, Vec<InspectorVariable>) {
    let overrides = prefab
        .vars
        .iter()
        .filter(|(name, _)| !excluded.contains(name))
        .map(|(name, variable)| InspectorVariable {
            name: name.clone(),
            source: variable
                .verbatim()
                .map_or_else(|| format_value(&variable.value), str::to_string),
        })
        .collect::<Vec<_>>();
    let overridden = prefab.vars.iter().map(|(name, _)| name.clone()).collect::<HashSet<_>>();
    let mut defaults = HashMap::<Identifier, Value>::new();

    if let Some(tree) = tree
        && let Some(id) = tree.id_of(&prefab.path)
    {
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

                defaults.entry(name.clone()).or_insert_with(|| variable.value.clone());
            }
        }
    }

    let mut defaults = defaults
        .into_iter()
        .map(|(name, value)| InspectorVariable {
            name,
            source: display_source(&value),
        })
        .collect::<Vec<_>>();
    defaults.sort_by(|left, right| left.name.as_str().cmp(right.name.as_str()));

    (overrides, defaults)
}

fn section<'ui>(ui: &'ui Ui, id: &str, title: &str, default_open: bool) -> Option<dear_imgui_rs::TreeNodeToken<'ui>> {
    ui.tree_node_config(id)
        .label(title)
        .default_open(default_open)
        .framed(true)
        .frame_padding(true)
        .span_avail_width(true)
        .push()
}

fn property_table(ui: &Ui, id: &str, content: impl FnOnce(&Ui)) {
    ui.table(id)
        .flags(TableFlags::BORDERS_INNER_V | TableFlags::RESIZABLE)
        .sizing_policy(TableSizingPolicy::StretchProp)
        .column("Property")
        .weight(0.32)
        .done()
        .column("Value")
        .weight(0.53)
        .done()
        .column("Override")
        .weight(0.15)
        .done()
        .build(content);
}

fn begin_property_row(ui: &Ui, property: &InspectorProperty, label: &str) {
    ui.table_next_row();
    ui.table_next_column();
    ui.align_text_to_frame_padding();
    if property.overridden {
        ui.text(format!("{label} *"));
    } else {
        ui.text(label);
    }
    ui.table_next_column();
}

fn draw_reset(ui: &Ui, session: &mut Session, property: &InspectorProperty) {
    ui.table_next_column();
    if property.overridden {
        if ui.small_button("Reset") {
            session.edit_selected_instance_vars(
                format!("reset {}", property.name),
                &[VarMutation::Remove(property.name.clone())],
                None,
            );
        }
        ui.set_item_tooltip(format!(
            "Remove the map override and restore {}",
            display_source(&property.inherited)
        ));
    } else {
        ui.text_disabled("Inherited");
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

fn commit_value(
    session: &mut Session, name: Identifier, value: Value, group: Option<EditGroupId>, draft: &mut VariableDraft,
) {
    if session
        .edit_selected_instance_vars(format!("set {name}"), &[VarMutation::Set(name, value)], group)
        .is_some()
    {
        draft.committed.clone_from(&draft.text);
        draft.error = None;
    } else {
        draft.error = Some(String::from("Selected object is no longer available"));
    }
}

fn draw_expression_input(ui: &Ui, session: &mut Session, name: Identifier, draft: &mut VariableDraft) {
    ui.set_next_item_width(-1.0);
    let enter = ui
        .input_text("##value", &mut draft.text)
        .enter_returns_true(true)
        .build();
    let commit = enter || ui.is_item_deactivated_after_edit();

    if commit && draft.text != draft.committed {
        match dmm::parser::parse_value(&draft.text) {
            Ok(value) => {
                let canonical = format_value(&value);
                if session.set_selected_instance_var(name, value).is_some() {
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

fn format_color(color: [f32; 4]) -> String {
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    let [red, green, blue, alpha] = color.map(channel);

    if alpha == u8::MAX {
        format!("#{red:02X}{green:02X}{blue:02X}")
    } else {
        format!("#{red:02X}{green:02X}{blue:02X}{alpha:02X}")
    }
}

fn draw_variable_section(
    ui: &Ui, session: &mut Session, state: &mut InspectorState, id: &str, title: &str, variables: &[InspectorVariable],
) {
    let Some(section) = section(ui, id, title, true) else {
        return;
    };

    if variables.is_empty() {
        ui.text_disabled("None");
        section.pop();

        return;
    }

    ui.table(format!("{id}-properties"))
        .flags(TableFlags::BORDERS_INNER_V | TableFlags::RESIZABLE)
        .sizing_policy(TableSizingPolicy::StretchProp)
        .column("Variable")
        .weight(0.4)
        .done()
        .column("Value")
        .weight(0.6)
        .done()
        .build(|ui| {
            for variable in variables {
                let Some(draft) = state.drafts.get_mut(&variable.name) else {
                    continue;
                };
                let _id = ui.push_id(variable.name.as_str());

                ui.table_next_row();
                ui.table_next_column();
                ui.align_text_to_frame_padding();
                ui.text(variable.name.as_str());
                ui.table_next_column();
                draw_expression_input(ui, session, variable.name.clone(), draft);
            }
        });

    section.pop();
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
        let (overrides, defaults) = inspector_variables(Some(&tree), &prefab, &excluded);

        assert_eq!(
            overrides,
            [InspectorVariable {
                name: "inherited".into(),
                source: String::from("4"),
            }],
        );
        assert_eq!(
            defaults,
            [
                InspectorVariable {
                    name: "closest".into(),
                    source: String::from("2"),
                },
                InspectorVariable {
                    name: "parent_only".into(),
                    source: String::from("\"parent\""),
                },
                InspectorVariable {
                    name: "runtime".into(),
                    source: String::from("<runtime>"),
                },
            ],
        );
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

    fn add_var(tree: &mut ObjectTree, id: TypeId, name: &str, value: Value, modifiers: VarModifiers) {
        let name = Identifier::from(name);
        tree.get_mut(id).unwrap().vars.insert(
            name.clone(),
            VarDecl {
                name,
                declared_type: None,
                modifiers,
                value,
                location: Location::default(),
            },
        );
    }
}
