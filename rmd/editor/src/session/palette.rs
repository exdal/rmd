use core::path::TreePath;

use dmm::Prefab;
use editor::{
    Environment,
    frame::{self},
    tool::{Tool, is_placeable},
    visual,
};
use objtree::TypeId;
use render::{SpriteTexture, texture::TextureCatalog};

use super::Session;

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
    pub fn palette(&self) -> Option<&Prefab> { self.state.palette.as_ref() }

    pub fn recent_prefabs(&self) -> &[Prefab] { self.state.recent_prefabs() }

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

    pub(super) fn prefab_thumbnail_for(
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
        if !matches!(self.state.tool, Tool::Fill | Tool::BlockSelect) {
            self.state.tool = Tool::Place;
        }

        true
    }

    pub fn choose_recent(&mut self, index: usize) -> bool {
        if !self.state.choose_recent(index) {
            return false;
        }
        if !matches!(self.state.tool, Tool::Fill | Tool::BlockSelect) {
            self.state.tool = Tool::Place;
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use core::{path::TreePath, types::Value};
    use std::sync::Arc;

    use dmm::{Coord, Prefab};
    use editor::{progress::Progress, tool::Tool};

    use crate::session::{Session, build_textures, fixtures::examples};

    #[test]
    fn prefab_thumbnails_resolve_overrides_and_sheet_coordinates() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).unwrap();
        let mut prefab = Prefab::new(TreePath::parse("/obj/structure/table"));
        let inherited = session.prefab_thumbnail(&prefab).expect("inherited table thumbnail");

        prefab.set_var("icon_state".into(), Value::Text(String::from("light")));
        prefab.set_var("color".into(), Value::Text(String::from("#ff000080")));
        prefab.set_var("alpha".into(), Value::Num(128.0));
        let overridden = session.prefab_thumbnail(&prefab).expect("overridden light thumbnail");

        assert_eq!(inherited.texture.width, 32);
        assert_eq!(inherited.texture.height, 32);
        assert_ne!(inherited.uv0, overridden.uv0);
        assert_ne!(inherited.uv1, overridden.uv1);
        assert_eq!(overridden.tint[0..3], [1.0, 0.0, 0.0]);
        assert!((overridden.tint[3] - (128.0 / 255.0) * (128.0 / 255.0)).abs() < f32::EPSILON);

        prefab.set_var("icon".into(), Value::Resource(String::from("icons/missing.dmi")));
        let missing = session.prefab_thumbnail(&prefab).expect("missing icon thumbnail");
        assert_eq!(missing.texture, session.textures.missing_icon().unwrap());
        assert_eq!(missing.tint[0..3], [1.0; 3]);
        assert_eq!(missing.tint[3], overridden.tint[3]);
        assert!(
            session
                .prefab_thumbnail(&Prefab::new(TreePath::parse("/area/station")))
                .is_none()
        );
    }

    /// A smoothed type's static `icon_state` is only a prefix, so the sheet holds nothing under it
    /// and the palette, the recent list and the placement preview would all draw nothing.
    #[test]
    fn palette_thumbnails_fall_back_to_a_standalone_bake_when_no_sprite_matches() {
        let root = examples();
        let profile = r#"
/turf/closed/wall/smoothed
    icon_state = "smooth"
/datum/demir/test
    default = TRUE
/datum/demir/test/bake(atom/target)
    if(istype(target, /turf/closed/wall/smoothed))
        target.icon_state = "wall"
"#;
        let compile = |baking| {
            let arena = core::arena::StrArena::new();
            let prelude = preprocessor::prelude_files()
                .into_iter()
                .chain([preprocessor::PreludeFile::Embedded("<test-standalone.dm>", profile)]);
            let preprocessed = preprocessor::Preprocessor::new(&arena)
                .with_prelude(prelude)
                .with_baking(baking)
                .run(root.join("test.dm"))
                .expect("preprocess");
            assert!(preprocessed.is_ok(), "{:?}", preprocessed.errors);

            let ast = ast::parse(&preprocessed.tokens).expect("parse");
            let (tree, module, errors) = sema::analyze(&ast, baking);
            assert!(errors.is_empty(), "{errors:?}");

            (tree, module)
        };
        let (editor_tree, _) = compile(false);
        let (bake_tree, module) = compile(true);
        let selected_profile = vm::bake::profile_type(&bake_tree).expect("default profile");
        let mut environment = editor::Environment::new(root.join("test.dme"), editor_tree);
        environment.bake_program = Some(editor::BakeProgram {
            tree: bake_tree,
            module: codegen::generate(&module).expect("codegen"),
            profile: selected_profile,
            files: Default::default(),
            icon_states: Default::default(),
        });
        assert!(environment.load_icons(&[], &Progress::new()).is_empty());

        let mut session = Session::new();
        session.textures = build_textures(&environment, &Progress::new());
        session.state.environment = Some(Arc::new(environment));

        let smoothed = Prefab::new(TreePath::parse("/turf/closed/wall/smoothed"));

        // "smooth" is in no sheet, so only a standalone bake makes this drawable
        assert_eq!(
            session.prefab_appearance(&smoothed).and_then(|a| a.icon_state),
            Some(String::from("wall"))
        );
        assert!(session.prefab_thumbnail(&smoothed).is_some());

        session.state.choose_prefab(smoothed.clone());
        session.set_tool(Tool::Place);

        assert!(session.placement_preview().is_some());

        // a type whose static state already matches never reaches the baker
        let plain = Prefab::new(TreePath::parse("/turf/closed/wall"));

        assert_eq!(
            session.prefab_appearance(&plain).and_then(|a| a.icon_state),
            Some(String::from("wall"))
        );
        assert_eq!(session.standalone.len(), 1);
    }

    #[test]
    fn tree_choices_place_new_objects_incrementally_and_select_them() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).unwrap();
        session.open_map(&root.join("test.dmm"), 1).unwrap();
        let table = session
            .tree()
            .unwrap()
            .id_of(&TreePath::parse("/obj/structure/table"))
            .unwrap();
        let coord = Coord::new(2, 2, 1);
        let before_len = session.map().unwrap().tile_at(coord).unwrap().len();
        let revision = session.revision();

        assert_eq!(session.place_at(coord, None), None);
        assert!(session.choose_type(table));
        assert_eq!(session.tool(), Tool::Place);
        let selected = session.place_at(coord, None).unwrap();

        assert_eq!(session.selected_instance(), Some(selected));
        assert_eq!(session.map().unwrap().tile_at(coord).unwrap().len(), before_len + 1);
        assert_eq!(
            session.selected_prefab().unwrap().path,
            TreePath::parse("/obj/structure/table")
        );
        assert_eq!(session.recent_prefabs()[0], *session.selected_prefab().unwrap());
        assert!(session.instances().unwrap().sprite(selected).is_some());
        assert_eq!(session.revision(), revision.wrapping_add(1));
        assert_eq!(session.frame_update().unwrap().previous_revision, revision);
    }

    #[test]
    fn palette_choices_preserve_fill_and_block_select_modes() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).unwrap();
        let floor = session
            .tree()
            .unwrap()
            .id_of(&TreePath::parse("/turf/open/floor"))
            .unwrap();

        for tool in [Tool::Fill, Tool::BlockSelect] {
            session.set_tool(tool);
            assert!(session.choose_type(floor));
            assert_eq!(session.tool(), tool);

            assert!(session.choose_recent(0));
            assert_eq!(session.tool(), tool);
        }
    }
}
