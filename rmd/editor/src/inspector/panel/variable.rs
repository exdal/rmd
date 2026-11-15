use base::types::{Identifier, Value};

use dear_imgui_rs::Ui;
use objtree::{ObjectTree, ResolvedVarType, VarTypeKind};
use ui::icons::materialdesignicons::{ICON_PIN, ICON_PIN_OUTLINE};

use crate::{
    document::command::EditGroupId, inspector::session::EditScope,
    object_tree::panel::type_search::draw_type_path_search, session::Session,
};

use super::{
    property::{commit_value, draw_changed_text, draw_draft_error, draw_expression_input, draw_reset_item},
    snapshot::{InspectorVariable, list_is_constant, parse_list_entries},
    state::{EditorMode, InspectorState, ListEntryDraft, TypedDraft, VariableDraft, drag_group, end_drag},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum VariableWidget {
    Number,
    Boolean,
    Text,
    Resource,
    Path,
    List,
    Raw,
}

pub(super) fn editor_mode(widget: VariableWidget, value: &Value, draft: &VariableDraft) -> EditorMode {
    if widget == VariableWidget::Raw {
        return EditorMode::Expression;
    }

    draft.mode.unwrap_or(match (widget, value) {
        (VariableWidget::Text, _) | (_, Value::Null) => EditorMode::Expression,
        _ => EditorMode::Typed,
    })
}

pub(super) fn variable_widget(variable: &InspectorVariable) -> VariableWidget {
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

pub(super) fn draw_variable_rows(
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
    if editor_mode(widget, &variable.value, draft) == EditorMode::Expression {
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

    let widget = variable_widget(variable);
    if widget != VariableWidget::Raw {
        let (label, toggled) = match editor_mode(widget, &variable.value, draft) {
            EditorMode::Expression => ("Use typed editor", EditorMode::Typed),
            EditorMode::Typed => ("Edit as expression", EditorMode::Expression),
        };
        if ui.menu_item(label) {
            draft.mode = Some(toggled);
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

pub(super) fn type_label(tree: Option<&ObjectTree>, ty: Option<&ResolvedVarType>) -> String {
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
