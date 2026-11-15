use base::{path::TreePath, types::Identifier};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use dmm::Prefab;
use objtree::{ObjectTree, TypeId};
use vm::bake::{LightingMap, SharedAppearance};

use crate::environment::world::Perspective;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TypeVisibility {
    hidden: HashSet<TypeId>,
}

impl TypeVisibility {
    pub fn is_visible(&self, id: TypeId) -> bool {
        !self.hidden.contains(&id)
    }

    pub fn set_subtree(&mut self, tree: &ObjectTree, root: TypeId, visible: bool) -> bool {
        if tree.get(root).is_none() {
            return false;
        }

        let mut changed = false;
        for id in tree.descendants(root) {
            changed |= if visible {
                self.hidden.remove(&id)
            } else {
                self.hidden.insert(id)
            };
        }

        changed
    }

    pub fn show_all(&mut self) -> bool {
        let changed = !self.hidden.is_empty();
        self.hidden.clear();

        changed
    }

    pub fn hides_any(&self) -> bool {
        !self.hidden.is_empty()
    }

    pub fn hidden_types(&self, tree: &ObjectTree) -> HiddenTypes {
        HiddenTypes {
            paths: self
                .hidden
                .iter()
                .filter_map(|id| tree.get(*id))
                .map(|decl| decl.path.segments.clone())
                .collect(),
            areas: None,
        }
    }
}

// this needs to have path as key, it needs to outlive map reloads
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HiddenTypes {
    paths: HashSet<Vec<Identifier>>,
    areas: Option<Arc<HashSet<Vec<Identifier>>>>,
}

impl HiddenTypes {
    pub fn with_areas(self, areas: Arc<HashSet<Vec<Identifier>>>) -> Self {
        Self {
            areas: Some(areas),
            ..self
        }
    }

    pub fn is_empty(&self) -> bool {
        self.paths.is_empty() && self.areas.as_ref().is_none_or(|areas| areas.is_empty())
    }

    pub fn hides(&self, prefab: &Prefab) -> bool {
        self.paths.contains(&prefab.path.segments)
            || self
                .areas
                .as_ref()
                .is_some_and(|areas| areas.contains(&prefab.path.segments))
    }
}

impl FromIterator<TreePath> for HiddenTypes {
    fn from_iter<I: IntoIterator<Item = TreePath>>(paths: I) -> Self {
        Self {
            paths: paths.into_iter().map(|path| path.segments).collect(),
            areas: None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct FrameRenderOptions<'a> {
    pub node_types: Option<&'a HashSet<TypeId>>,
    pub visibility: &'a TypeVisibility,
    pub tile_size: u32,
    /// What the bake made of each atom, keyed by `vm::bake` atom id, empty when baking is off
    pub appearances: &'a HashMap<u64, SharedAppearance>,
    pub lighting: Option<&'a LightingMap>,
    pub perspective: Perspective,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameOptions {
    pub show_areas: bool,
    pub show_area_outlines: bool,
    pub show_lighting: bool,
    /// Minimum displayed light luminance, from 0 to 100 percent.
    pub minimum_light_brightness_percent: u32,
    /// `world.icon_size`
    pub tile_size: u32,
    /// How many levels below the active one to draw behind it, 0 for none.
    pub underlay_depth: u32,
}

impl Default for FrameOptions {
    fn default() -> Self {
        Self {
            show_areas: false,
            show_area_outlines: true,
            show_lighting: true,
            minimum_light_brightness_percent: 0,
            tile_size: 32,
            underlay_depth: 3,
        }
    }
}
