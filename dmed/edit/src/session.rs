use std::path::{Path, PathBuf};

use dmi::IconFile;
use dmm::Map;
use editor::{EditorState, Environment, document::MapDocument};
use objtree::ObjectTree;
use render::{Frame, SpriteInstance, frame::FrameOptions, texture::TextureCatalog};

pub struct Session {
    pub state: EditorState,
    pub textures: TextureCatalog,
    pub options: FrameOptions,
    sprite_instances: Vec<SpriteInstance>,
    revision: u64,
    texture_revision: u64,
}

impl Session {
    pub fn new() -> Self {
        Self {
            state: EditorState::new(),
            textures: TextureCatalog::default(),
            options: FrameOptions::default(),
            sprite_instances: Vec::new(),
            revision: 0,
            texture_revision: 0,
        }
    }

    pub fn load_environment(&mut self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let (environment, diagnostics) = Environment::load(path)?;

        report(&environment, &diagnostics);
        self.textures = build_textures(&environment);
        self.texture_revision = self.texture_revision.wrapping_add(1);
        self.state.environment = Some(environment);

        Ok(())
    }

    pub fn open_map(&mut self, path: &Path, z: u32) -> Result<(), Box<dyn std::error::Error>> {
        let source = std::fs::read_to_string(path)?;
        let (map, errors) = dmm::parser::parse(&source);

        for error in &errors {
            eprintln!("{}: {error}", path.display());
        }

        validate_level(z, map.size.z)?;

        self.sprite_instances = self.state.environment.as_ref().map_or_else(Vec::new, |environment| {
            render::frame::build(
                &environment.tree,
                &environment.icons,
                &self.textures,
                &map,
                self.options.tile_size,
            )
        });
        self.state.open_document(MapDocument::open(path, map, z));
        self.revision = self.revision.wrapping_add(1);

        Ok(())
    }

    pub fn first_map(&self) -> Option<PathBuf> {
        let environment = self.state.environment.as_ref()?;

        environment
            .maps
            .iter()
            .find(|path| path.is_file())
            .or_else(|| environment.maps.first())
            .cloned()
    }

    pub fn tree(&self) -> Option<&ObjectTree> { self.state.environment.as_ref().map(|environment| &environment.tree) }

    pub fn map(&self) -> Option<&Map> { self.state.active_document().map(|document| &document.map) }

    pub fn z(&self) -> u32 { self.state.active_document().map_or(1, |document| document.z) }

    pub fn level_count(&self) -> u32 {
        self.state
            .active_document()
            .map_or(1, |document| document.map.size.z.max(1))
    }

    pub fn set_level(&mut self, z: u32) {
        let Some(document) = self.state.active_document_mut() else {
            return;
        };

        if z == document.z || !(1..=document.map.size.z.max(1)).contains(&z) {
            return;
        }

        document.z = z;
    }

    pub fn change_level(&mut self, delta: i32) {
        let Some(document) = self.state.active_document_mut() else {
            return;
        };

        let levels = document.map.size.z.max(1);
        let next = (document.z as i32 + delta).clamp(1, levels as i32) as u32;

        if next != document.z {
            document.z = next;
        }
    }

    pub fn toggle_areas(&mut self) { self.options.show_areas = !self.options.show_areas; }

    pub fn set_underlay_depth(&mut self, depth: u32) {
        if depth == self.options.underlay_depth {
            return;
        }

        self.options.underlay_depth = depth;
    }

    pub fn frame(&self, camera: render::Camera) -> Frame<'_> {
        Frame {
            sprite_instances: &self.sprite_instances,
            active_z: self.z(),
            underlay_depth: self.options.underlay_depth,
            show_areas: self.options.show_areas,
            camera,
            revision: self.revision,
        }
    }

    pub fn texture_revision(&self) -> u64 { self.texture_revision }

    pub fn extent_px(&self) -> (f32, f32) {
        let tile = self.options.tile_size.max(1) as f32;

        self.map().map_or((tile, tile), |map| {
            (map.size.x.max(1) as f32 * tile, map.size.y.max(1) as f32 * tile)
        })
    }
}

fn validate_level(z: u32, levels: u32) -> std::io::Result<()> {
    let levels = levels.max(1);

    if !(1..=levels).contains(&z) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("z level {z} is out of range; map has {levels} level(s)"),
        ));
    }

    Ok(())
}

fn build_textures(environment: &Environment) -> TextureCatalog {
    let mut textures = TextureCatalog::default();
    let base = environment.base_dir();

    for name in environment.icon_paths() {
        if !environment.icons.contains_key(name) {
            continue;
        }

        let mut candidates =
            std::iter::once(base.join(name)).chain(environment.resource_dirs.iter().map(|dir| dir.join(name)));

        let Some(path) = candidates.find(|path| path.is_file()) else {
            eprintln!("warning: could not find '{name}' on disk");
            continue;
        };

        match IconFile::load_info(&path) {
            Ok(info) => {
                if let Err(e) = textures.insert_info(name, &info) {
                    eprintln!("warning: {e}");
                }
            },
            Err(e) => eprintln!("warning: could not read '{name}': {e}"),
        }
    }

    textures
}

fn report(environment: &Environment, diagnostics: &editor::environment::LoadDiagnostics) {
    let root = environment.base_dir();
    let path = |file| {
        let path = environment.file(file)?;

        Some(path.strip_prefix(root).unwrap_or(path))
    };

    for error in &diagnostics.preprocess {
        eprintln!("{}", error.display(path(error.location.file)));
    }

    for error in &diagnostics.sema {
        eprintln!("{}", error.display(path(error.location.file)));
    }

    for (name, error) in &diagnostics.icons {
        eprintln!("warning: could not read '{name}': {error}");
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use dmi::IconFile;
    use dmm::{Map, Size};
    use editor::{Environment, document::MapDocument};

    use super::{Session, build_textures, validate_level};

    fn examples() -> PathBuf {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/env");

        path.canonicalize().unwrap_or(path)
    }

    #[test]
    fn validates_one_based_map_levels() {
        assert!(validate_level(1, 3).is_ok());
        assert!(validate_level(3, 3).is_ok());
        assert!(validate_level(0, 3).is_err());
        assert!(validate_level(4, 3).is_err());
    }

    #[test]
    fn environment_loading_packs_every_cell_in_each_dmi() {
        let root = examples();
        let (environment, diagnostics) = Environment::load(root.join("test.dme")).expect("load environment");
        assert!(diagnostics.icons.is_empty(), "{:?}", diagnostics.icons);
        let file = IconFile::load(root.join("icons/test.dmi")).expect("load icon");

        let textures = build_textures(&environment);

        assert_eq!(textures.len(), 1);
        assert_eq!(textures.cell_count(), file.cell_count());
        assert!((0..file.cell_count()).all(|cell| textures.lookup("icons/test.dmi", cell).is_some()));
    }

    #[test]
    fn view_changes_do_not_change_the_sprite_revision() {
        let mut session = Session::new();
        session
            .state
            .open_document(MapDocument::new(Map::new(Size { x: 1, y: 1, z: 3 }), 1));
        session.revision = 7;

        session.set_level(2);
        session.set_underlay_depth(1);
        session.toggle_areas();

        let frame = session.frame(Default::default());
        assert_eq!(frame.revision, 7);
        assert_eq!(frame.active_z, 2);
        assert_eq!(frame.underlay_depth, 1);
        assert!(frame.show_areas);
    }
}
