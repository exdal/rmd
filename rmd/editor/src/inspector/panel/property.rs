use base::{
    coord::Coord,
    types::{Identifier, Value},
    vars,
};

use dear_imgui_rs::{ChildFlags, DragFlags, StyleColor, TableFlags, TableSizingPolicy, Ui};
use dmi::metadata::Dir;
use dmm::{parser::parse_value, writer::format_value};
use ui::{color::byte, widgets::text_wrapped_colored};

use crate::{
    document::{command::EditGroupId, instance::VarMutation},
    gizmo::{direction::direction_choices, transform::anchor_axis},
    inspector::session::EditScope,
    session::Session,
};

use super::{
    ADVANCED_OFFSETS, DISPLAY_ROWS, IDENTICAL_EDIT_COLOR, TransformMode,
    direction::{direction_label, selected_direction},
    snapshot::{InspectorProperty, InspectorSnapshot, display_source},
    state::{InspectorState, TypedDraft, VariableDraft, drag_group, end_drag},
};

#[derive(Clone, Copy)]
enum TextPropertyKind {
    Text,
    Resource,
    NullableText,
}

impl InspectorState {
    pub(super) fn draw_transform(&mut self, ui: &Ui, session: &mut Session, snapshot: &InspectorSnapshot) {
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

    pub(super) fn draw_display(&mut self, ui: &Ui, session: &mut Session, snapshot: &InspectorSnapshot) {
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
            snapshot.unknown_icon.as_ref().map(ToString::to_string).as_deref(),
        );
        self.draw_asset_property(
            ui,
            session,
            snapshot,
            vars::ICON_STATE,
            "Icon state",
            TextPropertyKind::Text,
            snapshot.unknown_icon_state.as_ref().map(ToString::to_string).as_deref(),
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
        let (directions, current) = if self.shows_all_directions {
            (Dir::ORDER.to_vec(), Dir::from_bits(bits))
        } else {
            (
                direction_choices(count, snapshot.declared_directions, snapshot.directional_types),
                selected_direction(bits, snapshot.directional_types),
            )
        };
        let preview = current
            .map(direction_label)
            .unwrap_or_else(|| format!("Custom ({number})"));

        let shows_all_directions = &mut self.shows_all_directions;
        begin_property_row_with(ui, session, self.scope, property, "Direction", |ui| {
            let label = if *shows_all_directions {
                "Show supported directions"
            } else {
                "Show all directions"
            };
            if ui.menu_item(label) {
                *shows_all_directions = !*shows_all_directions;
            }
        });
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
                    if snapshot.directional_types.is_some() && !self.shows_all_directions {
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
}

pub(super) fn draw_identical_banner(ui: &Ui, session: &Session) -> bool {
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
pub(super) fn identical_frame_colors(ui: &Ui) -> [dear_imgui_rs::ColorStackToken<'_>; 3] {
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

pub(super) fn property_table(ui: &Ui, id: &str, content: impl FnOnce(&Ui)) {
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

pub(super) fn draw_changed_text(ui: &Ui, text: &str, is_changed: bool) {
    let _color = is_changed.then(|| ui.push_style_color(StyleColor::Text, ui.style_color(StyleColor::PlotHistogram)));
    ui.text(text);
}

fn begin_property_row(ui: &Ui, session: &mut Session, scope: EditScope, property: &InspectorProperty, label: &str) {
    begin_property_row_with(ui, session, scope, property, label, |_| {});
}

fn begin_property_row_with(
    ui: &Ui, session: &mut Session, scope: EditScope, property: &InspectorProperty, label: &str, menu: impl FnOnce(&Ui),
) {
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
            menu(ui);
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

pub(super) fn draw_reset_item(
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

pub(super) fn commit_int_property(
    session: &mut Session, scope: EditScope, mode: TransformMode, name: &Identifier, value: i32, group: EditGroupId,
) {
    let label = format!("change {name}");
    let mut committed = value;
    let mut to_coord = None;

    // Each identical instance keeps its own tile, so their offsets are set as typed
    if scope == EditScope::Selected
        && session.is_single_pick()
        && let Some(axis) = transform_axis(mode, name)
        && let Some(transform) = session.picked_transform()
        && let Some(location) = session.picked_location()
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
            session.move_picked_instance(coord, label, &[mutation], Some(group));
        },
        None => {
            session.edit_instance_vars_in(scope, label, &[mutation], Some(group));
        },
    }
}

pub(super) fn commit_value(
    session: &mut Session, scope: EditScope, name: Identifier, value: Value, group: Option<EditGroupId>,
    draft: &mut VariableDraft,
) {
    if !is_writable(&value) {
        draft.error = Some(String::from(r#"Escape " as \" and a final \ as \\"#));

        return;
    }

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

// Text is stored as the body of the dmm string literal, so it must survive being written and read back
pub(super) fn is_writable(value: &Value) -> bool {
    match value {
        Value::Text(_) | Value::Resource(_) => parse_value(&format_value(value)).is_ok_and(|parsed| parsed == *value),
        _ => true,
    }
}

pub(super) fn draw_expression_input(
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

pub(super) fn draw_draft_error(ui: &Ui, draft: &VariableDraft) {
    if let Some(error) = &draft.error {
        let _style = ui.push_style_color(StyleColor::Text, [1.0, 0.35, 0.35, 1.0]);
        ui.text_wrapped(error);
    }
}

pub(super) fn format_color(color: [f32; 4]) -> String {
    let [red, green, blue, alpha] = color.map(byte);

    if alpha == u8::MAX {
        format!("#{red:02X}{green:02X}{blue:02X}")
    } else {
        format!("#{red:02X}{green:02X}{blue:02X}{alpha:02X}")
    }
}
