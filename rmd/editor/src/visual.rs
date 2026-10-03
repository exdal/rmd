use core::{
    types::{Identifier, Value},
    vars,
};

use defines::{FLOAT_PLANE, RESET_ALPHA, RESET_COLOR, RESET_TRANSFORM, SOUTH};
use dmm::Prefab;
use objtree::{ObjectTree, TypeId};
use vm::matrix::Matrix;

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
    pub mouse_opacity: u8,
    pub appearance_flags: u32,
    pub transform: Matrix,
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
            dir: SOUTH,
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
            mouse_opacity: 1,
            appearance_flags: 0,
            transform: Matrix::IDENTITY,
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

    appearance.name = get(vars::NAME).and_then(|v| v.as_text().map(str::to_string));
    appearance.icon = get(vars::ICON).and_then(|v| v.as_text().map(str::to_string));
    appearance.icon_state = get(vars::ICON_STATE).and_then(|v| v.as_text().map(str::to_string));

    if let Some(dir) = get(vars::DIR).and_then(|v| v.as_num()) {
        appearance.dir = dir as u32;
    }

    if let Some(layer) = get(vars::LAYER).and_then(|v| v.as_num()) {
        appearance.layer = layer;
    }

    if let Some(plane) = get(vars::PLANE).and_then(|v| v.as_num()) {
        appearance.plane = plane;
    }

    if let Some(pixel_x) = get(vars::PIXEL_X).and_then(|v| v.as_num()) {
        appearance.pixel_x = pixel_x as i32;
    }

    if let Some(pixel_y) = get(vars::PIXEL_Y).and_then(|v| v.as_num()) {
        appearance.pixel_y = pixel_y as i32;
    }

    if let Some(pixel_w) = get(vars::PIXEL_W).and_then(|v| v.as_num()) {
        appearance.pixel_w = pixel_w as i32;
    }

    if let Some(pixel_z) = get(vars::PIXEL_Z).and_then(|v| v.as_num()) {
        appearance.pixel_z = pixel_z as i32;
    }

    if let Some(step_x) = get(vars::STEP_X).and_then(|v| v.as_num()) {
        appearance.step_x = step_x as i32;
    }

    if let Some(step_y) = get(vars::STEP_Y).and_then(|v| v.as_num()) {
        appearance.step_y = step_y as i32;
    }

    if let Some(alpha) = get(vars::ALPHA).and_then(|v| v.as_num()) {
        appearance.alpha = alpha.clamp(0.0, 255.0) as u8;
    }

    if let Some(invisibility) = get(vars::INVISIBILITY).and_then(|v| v.as_num()) {
        appearance.invisibility = invisibility as i32;
    }

    if let Some(mouse_opacity) = get(vars::MOUSE_OPACITY).and_then(|v| v.as_num()) {
        appearance.mouse_opacity = mouse_opacity.clamp(0.0, 2.0) as u8;
    }

    // TODO: type intrinsics
    if let Some(appearance_flags) = get(vars::APPEARANCE_FLAGS).and_then(|v| v.as_num()) {
        appearance.appearance_flags = appearance_flags as u32;
    }

    appearance.color = get(vars::COLOR).and_then(|v| v.as_text().map(str::to_string));

    if let Some(Value::List(entries)) = get(vars::TRANSFORM)
        && let Ok(components) = <[_; 6]>::try_from(
            entries
                .iter()
                .map(|entry| entry.key.as_num())
                .collect::<Option<Vec<f32>>>()
                .unwrap_or_default(),
        )
    {
        appearance.transform = Matrix(components);
    }

    appearance
}

pub(crate) fn sort_component(value: f32) -> i32 {
    let bits = value.to_bits() as i32;
    bits ^ (((bits >> 31) as u32 >> 1) as i32)
}

pub fn sort_key(appearance: &Appearance, index: usize) -> (i32, i32, usize) {
    (
        sort_component(appearance.plane),
        sort_component(appearance.layer),
        index,
    )
}

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
        .find(|(name, _)| name.as_str() == vars::DIR)
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
        .find(|(name, _)| name.as_str() == vars::APPEARANCE_FLAGS)
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

    if flags & RESET_TRANSFORM == 0 {
        appearance.transform = appearance.transform.then(parent.transform);
    }

    appearance
}

#[cfg(test)]
mod tests {
    use core::{
        location::Location,
        path::TreePath,
        types::{ListEntry, VarModifiers},
    };

    use objtree::VarDecl;

    use super::*;

    #[test]
    fn sort_keys_preserve_float_order_and_placement_ties() {
        let values = [
            -f32::INFINITY,
            -13.001,
            -13.0,
            -0.0,
            0.0,
            2.0,
            2.0001,
            2.001,
            f32::INFINITY,
        ];
        for left in values {
            for right in values {
                assert_eq!(sort_component(left).cmp(&sort_component(right)), left.total_cmp(&right));
            }
        }
        let floor = Appearance {
            plane: -13.0,
            layer: 2.0,
            ..Default::default()
        };
        let decal = Appearance {
            layer: 2.001,
            ..floor.clone()
        };
        let lower_plane = Appearance {
            plane: -13.001,
            layer: 3.0,
            ..floor.clone()
        };
        assert!(sort_key(&floor, 1) < sort_key(&decal, 0));
        assert!(sort_key(&lower_plane, 1) < sort_key(&floor, 0));
        assert!(sort_key(&floor, 0) < sort_key(&floor, 1));
    }

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
                resolved_type: None,
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

    fn transform_delta(components: [f32; 6], appearance_flags: u32) -> vm::AppearanceDelta {
        let entries = components.map(|component| ListEntry {
            key: Value::Num(component),
            value: None,
        });

        vm::AppearanceDelta {
            vars: vec![
                ("transform".into(), Value::List(entries.into())),
                ("appearance_flags".into(), Value::Num(appearance_flags as f32)),
            ],
            ..Default::default()
        }
    }

    #[test]
    fn overlays_apply_their_own_transform_before_their_parents() {
        let mut tree = ObjectTree::new();
        tree.register(&TreePath::parse("/image"), Location::default());
        let parent = Appearance {
            transform: Matrix::translate(-4.0, -4.0),
            ..Default::default()
        };
        let scaled = transform_delta([2.0, 0.0, 0.0, 0.0, 2.0, 0.0], 0);

        let overlay = resolve_overlay(&tree, &parent, &scaled);
        assert_eq!(overlay.transform, Matrix([2.0, 0.0, -4.0, 0.0, 2.0, -4.0]));

        let reset = resolve_overlay(
            &tree,
            &parent,
            &transform_delta([1.0, 0.0, 0.0, 0.0, 1.0, 0.0], RESET_TRANSFORM),
        );
        assert!(reset.transform.is_identity());
    }
}
