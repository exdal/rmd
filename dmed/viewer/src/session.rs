use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::{Path, PathBuf},
};

use dmi::{IconFile, metadata::Metadata};
use dmm::Map;
use editor::{Environment, document::MapDocument};
use render::{Frame, SpriteInstance, frame::FrameOptions, texture::TextureCatalog};

pub struct Session {
    pub environment: Option<Environment>,
    pub document: Option<MapDocument>,
    pub textures: TextureCatalog,
    pub icons: HashMap<String, Metadata>,
    pub options: FrameOptions,
    sprites: Vec<SpriteInstance>,
    revision: u64,
}

impl Session {
    pub fn new() -> Self {
        Self {
            environment: None,
            document: None,
            textures: TextureCatalog::default(),
            icons: HashMap::new(),
            options: FrameOptions::default(),
            sprites: Vec::new(),
            revision: 0,
        }
    }

    pub fn load_environment(&mut self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let (environment, diagnostics) = Environment::load(path)?;

        report(&environment, &diagnostics);

        self.icons = environment.icons.clone();
        self.environment = Some(environment);

        Ok(())
    }

    pub fn open_map(&mut self, path: &Path, z: u32) -> Result<(), Box<dyn std::error::Error>> {
        let source = std::fs::read_to_string(path)?;
        let (map, errors) = dmm::parser::parse(&source);

        for error in &errors {
            eprintln!("{}: {error}", path.display());
        }

        validate_level(z, map.size.z)?;

        if let Some(environment) = self.environment.as_ref() {
            self.textures = build_textures(environment, &render::frame::icons_used(&environment.tree, &map));
        }

        self.document = Some(MapDocument::open(path, map, z));
        self.rebuild();

        Ok(())
    }

    /// The first `.dmm` the environment includes
    pub fn first_map(&self) -> Option<PathBuf> {
        let environment = self.environment.as_ref()?;

        environment
            .maps
            .iter()
            .find(|path| path.is_file())
            .or_else(|| environment.maps.first())
            .cloned()
    }

    pub fn map(&self) -> Option<&Map> { self.document.as_ref().map(|document| &document.map) }

    pub fn z(&self) -> u32 { self.document.as_ref().map_or(1, |document| document.z) }

    pub fn change_level(&mut self, delta: i32) {
        let Some(document) = self.document.as_mut() else {
            return;
        };

        let levels = document.map.size.z.max(1);
        let next = (document.z as i32 + delta).clamp(1, levels as i32) as u32;

        if next != document.z {
            document.z = next;
            self.rebuild();
        }
    }

    pub fn toggle_areas(&mut self) {
        self.options.show_areas = !self.options.show_areas;
        self.rebuild();
    }

    pub fn rebuild(&mut self) {
        let (Some(environment), Some(document)) = (self.environment.as_ref(), self.document.as_ref()) else {
            self.sprites.clear();
            self.revision = self.revision.wrapping_add(1);

            return;
        };

        self.sprites = render::frame::build(
            &environment.tree,
            &self.icons,
            &self.textures,
            &document.map,
            document.z,
            &self.options,
        );
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn frame(&self, camera: render::Camera) -> Frame {
        Frame {
            sprites: self.sprites.clone(),
            camera,
            revision: self.revision,
        }
    }

    pub fn sprite_count(&self) -> usize { self.sprites.len() }

    pub fn texture_count(&self) -> usize { self.textures.len() }

    pub fn texture_bytes(&self) -> usize { self.textures.textures().iter().map(|t| t.pixels().len()).sum() }

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

/// we only decode used icons to save on VRAM (shit gets expensive real fast)
fn build_textures(environment: &Environment, used: &BTreeMap<String, BTreeSet<String>>) -> TextureCatalog {
    let mut textures = TextureCatalog::default();
    let base = environment.base_dir();

    for (name, states) in used {
        let mut candidates = std::iter::once(base.join(name))
            .chain(environment.resource_dirs.iter().map(|dir| base.join(dir).join(name)));

        let Some(path) = candidates.find(|path| path.is_file()) else {
            eprintln!("warning: could not find '{name}' on disk");
            continue;
        };

        let states: BTreeSet<&str> = states.iter().map(String::as_str).collect();

        match IconFile::load(&path) {
            Ok(file) => {
                if let Err(e) = textures.insert_states(name, &file, &states) {
                    eprintln!("warning: {e}");
                }
            },
            Err(e) => eprintln!("warning: could not decode '{name}': {e}"),
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
    use super::validate_level;

    #[test]
    fn validates_one_based_map_levels() {
        assert!(validate_level(1, 3).is_ok());
        assert!(validate_level(3, 3).is_ok());
        assert!(validate_level(0, 3).is_err());
        assert!(validate_level(4, 3).is_err());
    }
}
