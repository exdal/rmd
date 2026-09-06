use std::path::{Path, PathBuf};

use dmi::IconFile;
use dmm::Map;
use editor::{Environment, document::MapDocument};
use render::{Frame, SpriteInstance, frame::FrameOptions, texture::TextureCatalog};

pub struct Session {
    pub environment: Option<Environment>,
    pub document: Option<MapDocument>,
    pub textures: TextureCatalog,
    pub options: FrameOptions,
    sprite_instances: Vec<SpriteInstance>,
    revision: u64,
}

impl Session {
    pub fn new() -> Self {
        Self {
            environment: None,
            document: None,
            textures: TextureCatalog::default(),
            options: FrameOptions::default(),
            sprite_instances: Vec::new(),
            revision: 0,
        }
    }

    pub fn load_environment(&mut self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let (environment, diagnostics) = Environment::load(path)?;

        report(&environment, &diagnostics);

        self.textures = build_textures(&environment);
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
        let document = MapDocument::open(path, map, z);

        self.sprite_instances = self.environment.as_ref().map_or_else(Vec::new, |environment| {
            render::frame::build(
                &environment.tree,
                &environment.icons,
                &self.textures,
                &document,
                self.options.tile_size,
            )
        });
        self.document = Some(document);
        self.revision = self.revision.wrapping_add(1);

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
        }
    }

    pub fn toggle_areas(&mut self) { self.options.show_areas = !self.options.show_areas; }

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

    pub fn sprite_count(&self) -> usize { self.sprite_instances.len() }

    pub fn texture_count(&self) -> usize { self.textures.len() }

    pub fn texture_cell_count(&self) -> usize { self.textures.cell_count() }

    pub fn texture_bytes(&self) -> usize { self.textures.decoded_bytes() }

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

/// Catalog every icon up front so map navigation and future edits never have to extend it.
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
    use super::validate_level;

    #[test]
    fn validates_one_based_map_levels() {
        assert!(validate_level(1, 3).is_ok());
        assert!(validate_level(3, 3).is_ok());
        assert!(validate_level(0, 3).is_err());
        assert!(validate_level(4, 3).is_err());
    }
}
