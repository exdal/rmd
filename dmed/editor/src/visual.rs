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
    pub color: Option<String>,
    pub alpha: u8,
    pub invisibility: i32,
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
            color: None,
            alpha: 255,
            invisibility: 0,
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

/// [`resolve`] for a caller that already looked the prefab's type up
pub fn resolve_id(tree: &ObjectTree, id: TypeId, prefab: &Prefab) -> Appearance {
    let mut appearance = Appearance::default();

    let get = |name: &str| -> Option<Value> {
        let key = Identifier(name.to_string());
        if let Some(value) = prefab.var(&key) {
            return Some(value.clone());
        }

        tree.var_inherited(id, &key).map(|var| var.value.clone())
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

    if let Some(alpha) = get("alpha").and_then(|v| v.as_num()) {
        appearance.alpha = alpha.clamp(0.0, 255.0) as u8;
    }

    if let Some(invisibility) = get("invisibility").and_then(|v| v.as_num()) {
        appearance.invisibility = invisibility as i32;
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
