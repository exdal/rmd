use core::types::{Identifier, Value};

use dmm::Prefab;
use objtree::{ObjectTree, TypeId};

#[derive(Debug, Clone, PartialEq)]
pub struct Appearance {
    pub name: Option<String>,
    pub icon: Option<String>,
    pub icon_state: Option<String>,
    pub dir: u32,
    pub layer: f32,
    pub plane: f32,
    pub pixel_x: i32,
    pub pixel_y: i32,
    pub pixel_w: i32,
    pub pixel_z: i32,
    pub step_x: i32,
    pub step_y: i32,
    pub color: Option<String>,
    pub alpha: u8,
    pub invisibility: i32,
    pub appearance_flags: u32,
    pub lighting: vm::AppearanceLighting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueOrigin {
    Instance,
    Type,
}

#[derive(Debug, Clone, Copy)]
pub struct ResolvedValue<'a> {
    pub value: &'a Value,
    pub inherited: Option<&'a Value>,
    pub origin: ValueOrigin,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            name: None,
            icon: None,
            icon_state: None,
            // SOUTH
            dir: 2,
            layer: 2.0,
            plane: 0.0,
            pixel_x: 0,
            pixel_y: 0,
            pixel_w: 0,
            pixel_z: 0,
            step_x: 0,
            step_y: 0,
            color: None,
            alpha: 255,
            invisibility: 0,
            appearance_flags: 0,
            lighting: vm::AppearanceLighting::Normal,
        }
    }
}

/// Instance vars override inherited vars
pub fn resolve(tree: &ObjectTree, prefab: &Prefab) -> Appearance {
    let Some(id) = tree.id_of(&prefab.path) else {
        return Appearance::default();
    };

    resolve_id(tree, id, prefab)
}

/// Resolve one placed-prefab variable using the same instance-before-type
/// precedence as appearance rendering.
pub fn resolve_value<'a>(
    tree: &'a ObjectTree, id: TypeId, prefab: &'a Prefab, name: &Identifier,
) -> Option<ResolvedValue<'a>> {
    let inherited = tree.var_inherited(id, name).map(|var| &var.value);

    match prefab.var(name) {
        Some(value) => Some(ResolvedValue {
            value,
            inherited,
            origin: ValueOrigin::Instance,
        }),
        None => inherited.map(|value| ResolvedValue {
            value,
            inherited,
            origin: ValueOrigin::Type,
        }),
    }
}

/// [`resolve`] for a caller that already looked the prefab's type up
pub fn resolve_id(tree: &ObjectTree, id: TypeId, prefab: &Prefab) -> Appearance {
    let mut appearance = Appearance::default();

    let get = |name: &str| -> Option<Value> {
        let key = Identifier::from(name);

        resolve_value(tree, id, prefab, &key).map(|resolved| resolved.value.clone())
    };

    appearance.name = get("name").and_then(|v| v.as_text().map(str::to_string));
    appearance.icon = get("icon").and_then(|v| v.as_text().map(str::to_string));
    appearance.icon_state = get("icon_state").and_then(|v| v.as_text().map(str::to_string));

    if let Some(dir) = get("dir").and_then(|v| v.as_num()) {
        appearance.dir = dir as u32;
    }

    if let Some(layer) = get("layer").and_then(|v| v.as_num()) {
        appearance.layer = layer;
    }

    if let Some(plane) = get("plane").and_then(|v| v.as_num()) {
        appearance.plane = plane;
    }

    if let Some(pixel_x) = get("pixel_x").and_then(|v| v.as_num()) {
        appearance.pixel_x = pixel_x as i32;
    }

    if let Some(pixel_y) = get("pixel_y").and_then(|v| v.as_num()) {
        appearance.pixel_y = pixel_y as i32;
    }

    if let Some(pixel_w) = get("pixel_w").and_then(|v| v.as_num()) {
        appearance.pixel_w = pixel_w as i32;
    }

    if let Some(pixel_z) = get("pixel_z").and_then(|v| v.as_num()) {
        appearance.pixel_z = pixel_z as i32;
    }

    if let Some(step_x) = get("step_x").and_then(|v| v.as_num()) {
        appearance.step_x = step_x as i32;
    }

    if let Some(step_y) = get("step_y").and_then(|v| v.as_num()) {
        appearance.step_y = step_y as i32;
    }

    if let Some(alpha) = get("alpha").and_then(|v| v.as_num()) {
        appearance.alpha = alpha.clamp(0.0, 255.0) as u8;
    }

    if let Some(invisibility) = get("invisibility").and_then(|v| v.as_num()) {
        appearance.invisibility = invisibility as i32;
    }

    // TODO: type intrinsics
    if let Some(appearance_flags) = get("appearance_flags").and_then(|v| v.as_num()) {
        appearance.appearance_flags = appearance_flags as u32;
    }

    appearance.color = get("color").and_then(|v| v.as_text().map(str::to_string));

    appearance
}

/// Plane, layer, prefab index
pub fn sort_key(appearance: &Appearance, index: usize) -> (i32, i32, usize) {
    (
        (appearance.plane * 1000.0) as i32,
        (appearance.layer * 1000.0) as i32,
        index,
    )
}

/// `FLOAT_PLANE`
const FLOAT_PLANE: f32 = -32767.0;

/// `RESET_COLOR`
const RESET_COLOR: u32 = 2;

/// `RESET_ALPHA`
const RESET_ALPHA: u32 = 4;

pub fn resolve_delta(tree: &ObjectTree, id: TypeId, prefab: &Prefab, delta: &vm::AppearanceDelta) -> Appearance {
    let mut derived = prefab.clone();
    for (name, value) in &delta.vars {
        derived.set_var(name.clone(), value.clone());
    }

    let mut appearance = resolve_id(tree, id, &derived);
    appearance.lighting = delta.lighting;
    appearance
}

/// `overlays += "edge"`
pub fn resolve_overlay(tree: &ObjectTree, parent: &Appearance, delta: &vm::AppearanceDelta) -> Appearance {
    let prefab = Prefab::new(core::path::TreePath::parse("/image"));
    let id = tree.id_of(&prefab.path).unwrap_or(TypeId::ROOT);
    let mut appearance = resolve_delta(tree, id, &prefab, delta);

    if appearance.icon.is_none() {
        appearance.icon = parent.icon.clone();
    }

    let own_dir = delta
        .vars
        .iter()
        .find(|(name, _)| name.as_str() == "dir")
        .and_then(|(_, value)| value.as_num());
    if own_dir.is_none_or(|dir| dir == 0.0) {
        appearance.dir = parent.dir;
    }

    // `layer = FLOAT_LAYER - 1`
    if appearance.layer < 0.0 {
        appearance.layer = parent.layer;
    }

    if appearance.plane == FLOAT_PLANE {
        appearance.plane = parent.plane;
    }

    appearance.pixel_x = appearance.pixel_x.saturating_add(parent.pixel_x);
    appearance.pixel_y = appearance.pixel_y.saturating_add(parent.pixel_y);
    appearance.pixel_w = appearance.pixel_w.saturating_add(parent.pixel_w);
    appearance.pixel_z = appearance.pixel_z.saturating_add(parent.pixel_z);
    appearance.step_x = appearance.step_x.saturating_add(parent.step_x);
    appearance.step_y = appearance.step_y.saturating_add(parent.step_y);

    let flags = delta
        .vars
        .iter()
        .find(|(name, _)| name.as_str() == "appearance_flags")
        .and_then(|(_, value)| value.as_num())
        .unwrap_or(0.0) as u32;

    if flags & RESET_COLOR == 0 {
        let tint = |color: Option<&str>| color.and_then(render::color::parse).unwrap_or([1.0; 4]);
        let channels = tint(parent.color.as_deref())
            .into_iter()
            .zip(tint(appearance.color.as_deref()))
            .map(|(parent, own)| format!("{:02x}", (parent * own * 255.0).round() as u8))
            .collect::<String>();
        appearance.color = Some(format!("#{channels}"));
    }

    if flags & RESET_ALPHA == 0 {
        appearance.alpha = (u16::from(appearance.alpha) * u16::from(parent.alpha) / 255) as u8;
    }

    appearance
}

#[cfg(test)]
mod tests {
    use core::{location::Location, path::TreePath, types::VarModifiers};

    use objtree::VarDecl;

    use super::*;

    #[test]
    fn resolved_values_report_instance_and_inherited_sources() {
        let mut tree = ObjectTree::new();
        let id = tree.register(&TreePath::parse("/obj/item"), Location::default());
        let name = Identifier::from("pixel_x");
        tree.get_mut(id).unwrap().vars.insert(
            name.clone(),
            VarDecl {
                name: name.clone(),
                declared_type: None,
                modifiers: VarModifiers::default(),
                value: Value::Num(2.0),
                initializer: None,
                declared: true,
                location: Location::default(),
            },
        );
        let mut prefab = Prefab::new(TreePath::parse("/obj/item"));

        let inherited = resolve_value(&tree, id, &prefab, &name).unwrap();
        assert_eq!(inherited.value, &Value::Num(2.0));
        assert_eq!(inherited.inherited, Some(&Value::Num(2.0)));
        assert_eq!(inherited.origin, ValueOrigin::Type);

        prefab.set_var(name.clone(), Value::Num(7.0));
        let overridden = resolve_value(&tree, id, &prefab, &name).unwrap();
        assert_eq!(overridden.value, &Value::Num(7.0));
        assert_eq!(overridden.inherited, Some(&Value::Num(2.0)));
        assert_eq!(overridden.origin, ValueOrigin::Instance);
    }
}
