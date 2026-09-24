use dmm::{Coord, Prefab};
use editor::{
    Environment,
    conflict::Side,
    document::{DocumentId, PrefabInstanceId, Selection},
    frame::{self},
    tool::{
        BlockSelectionMode,
        SelectionMask,
        SelectionRotation,
        Tool,
        rotate_point,
        rotate_prefab,
        rotated_selection_at,
    },
    visual,
};
use render::SpriteInstance;

use super::{PrefabThumbnail, Session};

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PlacementPreview {
    pub thumbnail: PrefabThumbnail,
    pub offset: [i32; 2],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlockPreviewSource {
    Selection(SelectionMask),
    Clipboard,
    Conflict { region: u64, side: Side },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct BlockPreviewKey {
    source: BlockPreviewSource,
    rotation: SelectionRotation,
    pub(super) document_revision: u64,
    clipboard_revision: u64,
    texture_revision: u64,
    tile_size: u32,
    show_areas: bool,
}

pub(super) struct BlockPreviewCache {
    pub(super) key: BlockPreviewKey,
    pub(super) sprites: Vec<SpriteInstance>,
    pub(super) revision: u64,
    pub(super) destination: Coord,
    pub(super) visible: bool,
}

const PREVIEW_OWNER: PrefabInstanceId = match PrefabInstanceId::from_raw(1) {
    Some(id) => id,
    None => unreachable!(),
};

impl Session {
    pub(crate) fn hide_block_preview(&mut self, id: DocumentId) {
        if let Some(preview) = self.caches.get_mut(&id).and_then(|cache| cache.preview.as_mut()) {
            preview.visible = false;
        }
    }

    pub(crate) fn prepare_block_preview(
        &mut self, id: DocumentId, source: BlockPreviewSource, destination: Coord, rotation: SelectionRotation,
    ) {
        let Some(cache) = self.caches.get(&id) else {
            return;
        };

        if self.state.active() != Some(id) {
            return;
        }
        let key = BlockPreviewKey {
            source,
            rotation,
            document_revision: cache.revision,
            clipboard_revision: if source == BlockPreviewSource::Clipboard {
                self.state.clipboard_revision()
            } else {
                0
            },
            texture_revision: self.texture_revision,
            tile_size: self.options.tile_size,
            show_areas: self.options.show_areas,
        };

        if cache.preview.as_ref().is_none_or(|preview| preview.key != key) {
            let origin = Coord::new(1, 1, 1);
            let sprites = match source {
                BlockPreviewSource::Selection(mask) => rotated_selection_at(mask.bounds, origin, rotation)
                    .map_or_else(Vec::new, |target| {
                        self.block_preview_sprites(mask.bounds, target, rotation, mask.mode)
                    }),
                BlockPreviewSource::Clipboard => self
                    .clipboard_footprint(origin, rotation)
                    .map_or_else(Vec::new, |target| self.clipboard_preview_sprites(target, rotation)),
                BlockPreviewSource::Conflict { region, side } => self.conflict_preview_sprites(id, region, side),
            };
            let revision = self.next_preview_revision;
            self.next_preview_revision = revision.wrapping_add(1).max(1);

            if let Some(cache) = self.caches.get_mut(&id) {
                cache.preview = Some(BlockPreviewCache {
                    key,
                    sprites,
                    revision,
                    destination,
                    visible: true,
                });
            }
        } else if let Some(preview) = self.caches.get_mut(&id).and_then(|cache| cache.preview.as_mut()) {
            preview.destination = destination;
            preview.visible = true;
        }
    }

    pub(crate) fn block_preview_sprites(
        &self, source: Selection, target: Selection, rotation: SelectionRotation, mode: BlockSelectionMode,
    ) -> Vec<SpriteInstance> {
        let Some(document) = self.state.active_document() else {
            return Vec::new();
        };

        let Some(environment) = self.state.environment.as_ref() else {
            return Vec::new();
        };

        let mut previews = Vec::new();
        for coord in mode.tiles(source) {
            let relative = (coord.x - source.min.x, coord.y - source.min.y);
            let transformed = rotate_point(relative, source.width(), source.height(), rotation);
            let destination = Coord::new(target.min.x + transformed.0, target.min.y + transformed.1, target.min.z);
            let Some(tile) = document.placed_tile(coord) else {
                continue;
            };
            for placed in tile {
                previews.extend(self.preview_sprite(environment, placed.id(), placed.prefab(), destination, rotation));
            }
        }

        previews.sort_by(|left, right| left.depth.total_cmp(&right.depth));

        previews
    }

    pub(crate) fn clipboard_preview_sprites(
        &self, target: Selection, rotation: SelectionRotation,
    ) -> Vec<SpriteInstance> {
        let Some(environment) = self.state.environment.as_ref() else {
            return Vec::new();
        };
        let Some(block) = self.state.clipboard() else {
            return Vec::new();
        };

        let mut previews = Vec::new();
        for (destination, tile) in block.destinations(target, rotation) {
            for prefab in tile {
                previews.extend(self.preview_sprite(environment, PREVIEW_OWNER, prefab, destination, rotation));
            }
        }

        previews.sort_by(|left, right| left.depth.total_cmp(&right.depth));

        previews
    }

    fn conflict_preview_sprites(&self, id: DocumentId, region_id: u64, side: Side) -> Vec<SpriteInstance> {
        let Some(environment) = self.state.environment.as_ref() else {
            return Vec::new();
        };
        let Some(document) = self.state.document(id) else {
            return Vec::new();
        };
        let Some(conflicts) = self.git_state(id).and_then(|git| git.conflicts.as_ref()) else {
            return Vec::new();
        };
        let Some(region) = conflicts
            .regions(Some(document.z), &document.history)
            .into_iter()
            .find(|region| region.id() == region_id)
        else {
            return Vec::new();
        };

        let mut sprites = Vec::new();
        for coord in &region.tiles {
            let Some(Some(tile)) = conflicts.side_tile(*coord, side) else {
                continue;
            };
            let target = Coord::new(coord.x - region.min.x + 1, coord.y - region.min.y + 1, 1);
            for prefab in tile {
                sprites.extend(self.preview_sprite(
                    environment,
                    PREVIEW_OWNER,
                    prefab,
                    target,
                    SelectionRotation::Original,
                ));
            }
        }

        sprites.sort_by(|left, right| left.depth.total_cmp(&right.depth));

        sprites
    }

    fn preview_sprite(
        &self, environment: &Environment, owner: PrefabInstanceId, prefab: &Prefab, destination: Coord,
        rotation: SelectionRotation,
    ) -> Option<SpriteInstance> {
        let mut prefab = prefab.clone();
        rotate_prefab(&environment.tree, &mut prefab, rotation);

        let id = environment.tree.id_of(&prefab.path)?;
        if !self.type_visibility.is_visible(id) {
            return None;
        }

        let is_area = environment
            .tree
            .roots()
            .area
            .is_some_and(|area| environment.tree.is_subtype_of(id, area));
        if is_area && !self.options.show_areas {
            return None;
        }

        let appearance = visual::resolve_id(&environment.tree, id, &prefab);
        let texture = frame::sprite_texture_or_missing(&environment.icons, &self.textures, &appearance)?;
        let mut sprite = frame::instance_for(
            owner,
            &appearance,
            texture,
            destination,
            self.options.tile_size,
            is_area,
        );
        if self.textures.is_missing_icon(texture) {
            sprite.color = [sprite.color[3]; 4];
        }

        Some(SpriteInstance {
            color: sprite.color.map(|channel| channel * 0.55),
            ..sprite
        })
    }

    pub(crate) fn placement_preview(&mut self) -> Option<PlacementPreview> {
        if self.tool() != Tool::Place {
            return None;
        }

        let prefab = self.palette()?.clone();
        let appearance = self.prefab_appearance(&prefab)?;
        let environment = self.state.environment.as_ref()?;
        let thumbnail = self.prefab_thumbnail_for(environment, &appearance)?;
        let offset = [
            appearance
                .step_x
                .saturating_add(appearance.pixel_x)
                .saturating_add(appearance.pixel_w),
            appearance
                .step_y
                .saturating_add(appearance.pixel_y)
                .saturating_add(appearance.pixel_z),
        ];

        Some(PlacementPreview { thumbnail, offset })
    }
}

#[cfg(test)]
mod tests {
    use core::{path::TreePath, types::Value};

    use dmm::Prefab;
    use editor::tool::Tool;

    use crate::session::{Session, fixtures::examples};

    #[test]
    fn placement_previews_require_place_mode_and_preserve_visual_offsets() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).unwrap();
        let mut prefab = Prefab::new(TreePath::parse("/obj/structure/table"));
        prefab.set_var("step_x".into(), Value::Num(-2.0));
        prefab.set_var("pixel_x".into(), Value::Num(5.0));
        prefab.set_var("pixel_w".into(), Value::Num(1.0));
        prefab.set_var("step_y".into(), Value::Num(3.0));
        prefab.set_var("pixel_y".into(), Value::Num(-4.0));
        prefab.set_var("pixel_z".into(), Value::Num(2.0));

        assert!(session.placement_preview().is_none());
        session.state.choose_prefab(prefab.clone());
        assert!(session.placement_preview().is_none());

        session.set_tool(Tool::Place);
        let preview = session.placement_preview().expect("resolved placement preview");

        assert_eq!(preview.thumbnail, session.prefab_thumbnail(&prefab).unwrap());
        assert_eq!(preview.offset, [4, 1]);

        session
            .state
            .choose_prefab(Prefab::new(TreePath::parse("/obj/unresolved")));
        assert!(session.placement_preview().is_none());
    }
}
