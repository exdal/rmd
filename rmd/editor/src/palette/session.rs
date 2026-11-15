use base::path::TreePath;

use dmm::Prefab;
use objtree::TypeId;
use render::{SpriteTexture, texture::TextureCatalog};

use crate::{
    environment::Environment,
    frame::{self, visual},
    session::Session,
    tool::{Tool, is_placeable},
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PrefabThumbnail {
    pub texture: SpriteTexture,
    pub uv0: [f32; 2],
    pub uv1: [f32; 2],
    pub tint: [f32; 4],
}

const STANDALONE_CACHE_LIMIT: usize = 256;

pub(crate) fn prefab_thumbnail_for(
    textures: &TextureCatalog, environment: &Environment, appearance: &visual::Appearance,
) -> Option<PrefabThumbnail> {
    let texture = frame::sprite_texture(&environment.icons, textures, appearance)?;
    thumbnail_for_texture(textures, appearance, texture)
}

pub(crate) fn prefab_thumbnail_or_missing(
    textures: &TextureCatalog, environment: &Environment, appearance: &visual::Appearance,
) -> Option<PrefabThumbnail> {
    let texture = frame::sprite_texture_or_missing(&environment.icons, textures, appearance)?;
    thumbnail_for_texture(textures, appearance, texture)
}

fn thumbnail_for_texture(
    textures: &TextureCatalog, appearance: &visual::Appearance, texture: SpriteTexture,
) -> Option<PrefabThumbnail> {
    let sheet = textures.texture(texture.index)?;
    let sheet_width = sheet.width() as f32;
    let sheet_height = sheet.height() as f32;
    let right = texture.source_position[0].checked_add(texture.width)?;
    let bottom = texture.source_position[1].checked_add(texture.height)?;
    let mut tint = appearance
        .color
        .as_deref()
        .and_then(render::color::parse)
        .unwrap_or([1.0; 4]);
    tint[3] *= f32::from(appearance.alpha) / 255.0;
    if textures.is_missing_icon(texture) {
        tint[0..3].fill(1.0);
    }

    Some(PrefabThumbnail {
        texture,
        uv0: [
            texture.source_position[0] as f32 / sheet_width,
            texture.source_position[1] as f32 / sheet_height,
        ],
        uv1: [right as f32 / sheet_width, bottom as f32 / sheet_height],
        tint,
    })
}

impl Session {
    pub fn palette(&self) -> Option<&Prefab> {
        self.state.palette.as_ref()
    }

    pub fn recent_prefabs(&self) -> &[Prefab] {
        self.state.recent_prefabs()
    }

    pub(crate) fn prefab_appearance(&mut self, prefab: &Prefab) -> Option<visual::Appearance> {
        let environment = self.state.environment.clone()?;
        let appearance = visual::resolve(&environment.tree, prefab);
        if frame::sprite_texture(&environment.icons, &self.textures, &appearance).is_some() {
            return Some(appearance);
        }

        if let Some((_, cached)) = self.standalone.iter().find(|(cached, _)| cached == prefab) {
            return cached.clone().or(Some(appearance));
        }

        let derived = environment.tree.id_of(&prefab.path).and_then(|id| {
            let delta = self.standalone_baker.appearance(&environment, prefab)?;

            Some(visual::resolve_delta(&environment.tree, id, prefab, &delta))
        });
        if self.standalone.len() >= STANDALONE_CACHE_LIMIT {
            self.standalone.clear();
        }
        self.standalone.push((prefab.clone(), derived.clone()));

        derived.or(Some(appearance))
    }

    pub(crate) fn prefab_thumbnail(&mut self, prefab: &Prefab) -> Option<PrefabThumbnail> {
        let appearance = self.prefab_appearance(prefab)?;
        let environment = self.state.environment.as_ref()?;

        self.prefab_thumbnail_for(environment, &appearance)
    }

    pub(crate) fn type_thumbnail(&self, id: TypeId) -> Option<PrefabThumbnail> {
        self.type_thumbnails.get(&id).copied().flatten()
    }

    pub(crate) fn prefab_thumbnail_for(
        &self, environment: &Environment, appearance: &visual::Appearance,
    ) -> Option<PrefabThumbnail> {
        prefab_thumbnail_or_missing(&self.textures, environment, appearance)
    }

    pub fn choose_type(&mut self, selected: TypeId) -> bool {
        let prefab = self.state.environment.as_ref().and_then(|environment| {
            is_placeable(&environment.tree, selected)
                .then(|| environment.tree.get(selected))
                .flatten()
                .map(|declaration| Prefab::new(TreePath::parse(&declaration.path.to_string())))
        });
        let Some(prefab) = prefab else {
            return false;
        };

        self.state.choose_prefab(prefab);
        if !matches!(self.state.tool, Tool::Fill | Tool::RectFill | Tool::Select) {
            self.state.tool = Tool::Place;
        }

        true
    }

    pub fn choose_recent(&mut self, index: usize) -> bool {
        if !self.state.choose_recent(index) {
            return false;
        }
        if !matches!(self.state.tool, Tool::Fill | Tool::RectFill | Tool::Select) {
            self.state.tool = Tool::Place;
        }

        true
    }

    pub fn choose_prefab(&mut self, prefab: Prefab) {
        self.state.choose_prefab(prefab);
        if !matches!(self.state.tool, Tool::Fill | Tool::RectFill | Tool::Select) {
            self.state.tool = Tool::Place;
        }
    }
}

#[cfg(test)]
mod tests;
