use base::{
    path::TreePath,
    types::{Identifier, ListEntry, Value, encode_string, encode_text_macros},
    vars,
};
use std::collections::{HashMap, HashSet};

use dmm::{error::MapErrorKind, parser::parse_value, writer::format_value};
use objtree::{ObjectTree, ResolvedVarType, TypeId};
use prefab::{Prefab, PrefabInstanceId, PrefabLocation};

use crate::{
    app::external_editor::SourceLocation, frame::visual, gizmo::direction::DirectionalTypes, session::Session,
};

use super::state::ListEntryDraft;

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

#[derive(Debug, Clone, PartialEq)]
pub(super) struct InspectorVariable {
    pub(super) name: Identifier,
    pub(super) source: String,
    pub(super) overridden: bool,
    pub(super) value: Value,
    pub(super) ty: Option<ResolvedVarType>,
    pub(super) owner: Option<TypeId>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct VariableGroup {
    pub(super) owner: Option<TypeId>,
    pub(super) path: String,
    pub(super) source: Option<SourceLocation>,
}

#[derive(Debug, Clone)]
pub(super) struct InspectorProperty {
    pub(super) name: Identifier,
    pub(super) value: Value,
    pub(super) inherited: Value,
    pub(super) editor_text: String,
    pub(super) overridden: bool,
}

pub(super) struct InspectorSnapshot {
    pub(super) picked: PrefabInstanceId,
    pub(super) groups: Vec<VariableGroup>,
    pub(super) location: PrefabLocation,
    pub(super) is_atom: bool,
    pub(super) is_movable: bool,
    pub(super) properties: HashMap<Identifier, InspectorProperty>,
    pub(super) variables: Vec<InspectorVariable>,
    pub(super) icon_states: Vec<(String, u32)>,
    pub(super) declared_directions: Option<[bool; 8]>,
    pub(super) directional_types: Option<DirectionalTypes>,
    pub(super) unknown_icon: Option<MapErrorKind>,
    pub(super) unknown_icon_state: Option<MapErrorKind>,
}

impl InspectorSnapshot {
    pub(super) fn property(&self, name: &str) -> Option<&InspectorProperty> {
        self.properties.get(&Identifier::from(name))
    }

    pub(super) fn text(&self, name: &str) -> String {
        self.property(name)
            .and_then(|property| property.value.as_text())
            .unwrap_or_default()
            .to_string()
    }
}

pub(super) fn inspector_snapshot(session: &Session) -> Option<InspectorSnapshot> {
    let picked = session.picked_instance()?;
    let location = session.picked_location()?;
    let prefab = session.picked_prefab()?;
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
    let directional_types = session.picked_directional_types();
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
        picked,
        groups: variable_groups(tree, type_id, &prefab.path, &variables, |id| session.type_source(id)),
        location,
        is_atom,
        is_movable,
        properties,
        variables,
        icon_states,
        declared_directions,
        directional_types,
        unknown_icon: (!icon.is_empty() && metadata.is_none()).then_some(MapErrorKind::UnknownIcon(icon)),
        unknown_icon_state: (!icon_state.is_empty()
            && metadata.is_none_or(|metadata| metadata.find(&icon_state).is_none()))
        .then_some(MapErrorKind::UnknownIconState(icon_state)),
    })
}

pub(super) fn property_snapshot(
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
        .map(spelling)
        .unwrap_or_else(|| builtin_default(name.as_str()));
    let value = match resolved {
        Some(resolved) if resolved.origin == visual::ValueOrigin::Instance => resolved.value.clone(),
        Some(resolved) => spelling(resolved.value.clone()),
        None => prefab.var(&name).cloned().unwrap_or_else(|| inherited.clone()),
    };
    let source = prefab
        .vars
        .iter()
        .find(|(candidate, _)| candidate == &name)
        .and_then(|(_, variable)| variable.spelling())
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

/// Tree defaults are decoded while dmm values keep their source spelling, and the inspector edits spellings
pub(super) fn spelling(value: Value) -> Value {
    match value {
        Value::Text(text) => Value::Text(encode_string(&text)),
        Value::List(entries) => Value::List(
            entries
                .into_iter()
                .map(|entry| ListEntry {
                    key: spelling(entry.key),
                    value: entry.value.map(spelling),
                })
                .collect(),
        ),
        value => value,
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
            encode_text_macros(text)
        },
        (vars::NAME | vars::ICON | vars::ICON_STATE | vars::COLOR, Value::Null) => String::new(),
        _ => source.to_string(),
    }
}

pub(super) fn display_source(value: &Value) -> String {
    match value {
        Value::Unevaluated => String::from("<runtime>"),
        value => encode_text_macros(&format_value(value)),
    }
}

pub(super) fn typed_value_text(value: &Value) -> String {
    match value {
        Value::Text(text) | Value::Resource(text) => text.clone(),
        _ => String::new(),
    }
}

pub(super) fn list_entry_drafts(value: &Value) -> Vec<ListEntryDraft> {
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

pub(super) fn parse_list_entries(entries: &[ListEntryDraft]) -> Result<Value, String> {
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

pub(super) fn list_is_constant(entries: &[ListEntry]) -> bool {
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

pub(super) fn inspector_variables(
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
                .spelling()
                .map_or_else(|| encode_text_macros(&format_value(&variable.value)), str::to_string),
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
                        spelling(variable.value.clone()),
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

pub(super) fn variable_groups(
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
