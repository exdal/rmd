use std::collections::{BTreeSet, HashMap};

use dmi::IconFile;

use crate::SpriteTexture;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextureError {
    ZeroCellSize { icon: String },
    TooManyTextures,
    CellDataTooLarge { icon: String, width: u32, height: u32 },
}

impl std::fmt::Display for TextureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ZeroCellSize { icon } => write!(f, "'{icon}' declares a zero cell size"),
            Self::TooManyTextures => write!(f, "the texture catalog exceeds the u32 texture-index limit"),
            Self::CellDataTooLarge { icon, width, height } => {
                write!(f, "'{icon}' has a {width}x{height} cell whose RGBA data is too large")
            },
        }
    }
}

impl std::error::Error for TextureError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextureData {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl TextureData {
    pub fn width(&self) -> u32 { self.width }

    pub fn height(&self) -> u32 { self.height }

    pub fn pixels(&self) -> &[u8] { &self.pixels }
}

#[derive(Debug, Default)]
pub struct TextureCatalog {
    textures: Vec<TextureData>,
    sheets: HashMap<String, Vec<SpriteTexture>>,
}

impl TextureCatalog {
    pub fn new() -> Self { Self::default() }

    pub fn len(&self) -> usize { self.textures.len() }

    pub fn is_empty(&self) -> bool { self.textures.is_empty() }

    pub fn textures(&self) -> &[TextureData] { &self.textures }

    /// `metadata.find(state)?.sprite_index(dir, frame)` gives `cell`.
    pub fn lookup(&self, icon: &str, cell: usize) -> Option<SpriteTexture> {
        let texture = self.sheets.get(icon)?.get(cell)?;

        (texture.width > 0 && texture.height > 0).then_some(*texture)
    }

    pub fn insert(&mut self, icon: &str, file: &IconFile) -> Result<(), TextureError> {
        self.insert_cells(icon, file, None)
    }

    pub fn insert_states(&mut self, icon: &str, file: &IconFile, states: &BTreeSet<&str>) -> Result<(), TextureError> {
        let mut keep = BTreeSet::new();

        for name in states {
            let Some(state) = file.metadata.find(name) else {
                continue;
            };

            for cell in state.offset..state.offset.saturating_add(state.sprite_count()) {
                keep.insert(cell);
            }
        }

        self.insert_cells(icon, file, Some(&keep))
    }

    fn insert_cells(
        &mut self, icon: &str, file: &IconFile, keep: Option<&BTreeSet<usize>>,
    ) -> Result<(), TextureError> {
        let (width, height) = (file.metadata.width, file.metadata.height);

        if width == 0 || height == 0 {
            return Err(TextureError::ZeroCellSize { icon: icon.to_string() });
        }

        let cells = file.cell_count();
        let Some(total) = self.textures.len().checked_add(cells) else {
            return Err(TextureError::TooManyTextures);
        };
        if total > u32::MAX as usize {
            return Err(TextureError::TooManyTextures);
        }

        let base = self.textures.len();
        let mut data = Vec::with_capacity(cells);
        let mut textures = Vec::with_capacity(cells);
        for cell in 0..cells {
            if keep.is_some_and(|keep| !keep.contains(&cell)) {
                textures.push(SpriteTexture::default());
                continue;
            }

            let (src_x, src_y, ..) = file.metadata.sprite_rect(cell, file.sheet_width);
            let in_bounds = src_x.checked_add(width).is_some_and(|right| right <= file.sheet_width)
                && src_y
                    .checked_add(height)
                    .is_some_and(|bottom| bottom <= file.sheet_height);

            if !in_bounds {
                textures.push(SpriteTexture::default());
                continue;
            }

            let Some(pixels) = extract_cell(file, src_x, src_y, width, height) else {
                return Err(TextureError::CellDataTooLarge {
                    icon: icon.to_string(),
                    width,
                    height,
                });
            };
            let Some(next) = base.checked_add(data.len()) else {
                return Err(TextureError::TooManyTextures);
            };
            let Ok(index) = u32::try_from(next) else {
                return Err(TextureError::TooManyTextures);
            };

            data.push(TextureData { width, height, pixels });
            textures.push(SpriteTexture { index, width, height });
        }

        self.textures.extend(data);
        self.sheets.insert(icon.to_string(), textures);

        Ok(())
    }
}

fn extract_cell(file: &IconFile, src_x: u32, src_y: u32, width: u32, height: u32) -> Option<Vec<u8>> {
    let row_bytes = (width as usize).checked_mul(4)?;
    let texture_bytes = row_bytes.checked_mul(height as usize)?;
    let source_stride = (file.sheet_width as usize).checked_mul(4)?;
    let source_x = (src_x as usize).checked_mul(4)?;
    let mut pixels = vec![0; texture_bytes];

    for row in 0..height as usize {
        let source_start = (src_y as usize)
            .checked_add(row)?
            .checked_mul(source_stride)?
            .checked_add(source_x)?;
        let source_end = source_start.checked_add(row_bytes)?;
        let target_start = row.checked_mul(row_bytes)?;
        let target_end = target_start.checked_add(row_bytes)?;
        let source = file.pixels.get(source_start..source_end)?;
        let target = pixels.get_mut(target_start..target_end)?;

        target.copy_from_slice(source);
    }

    Some(pixels)
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeSet, path::PathBuf};

    use dmi::{
        IconFile,
        metadata::{IconState, Metadata},
    };

    use crate::texture::{TextureCatalog, TextureError};

    /// A sheet of `cells` cells laid out in one row, each a flat colour.
    fn sheet(width: u32, height: u32, cells: u32) -> IconFile {
        let sheet_width = width * cells;
        let mut pixels = vec![0u8; (sheet_width * height * 4) as usize];

        for (index, pixel) in pixels.chunks_exact_mut(4).enumerate() {
            let cell = (index as u32 % sheet_width) / width;
            pixel.copy_from_slice(&[cell as u8 + 1, 0, 0, 255]);
        }

        IconFile {
            path: PathBuf::from("test.dmi"),
            metadata: Metadata {
                version: String::from("4.0"),
                width,
                height,
                states: vec![IconState {
                    name: String::from("a"),
                    dirs: 1,
                    frames: cells,
                    ..Default::default()
                }],
            },
            sheet_width,
            sheet_height: height,
            pixels,
        }
    }

    #[test]
    fn every_cell_gets_a_standalone_texture() {
        let mut catalog = TextureCatalog::new();
        catalog.insert("test.dmi", &sheet(32, 16, 3)).expect("insert");

        let textures: Vec<_> = (0..3)
            .map(|cell| catalog.lookup("test.dmi", cell).expect("cell"))
            .collect();

        assert_eq!(catalog.len(), 3);
        assert_eq!(
            textures.iter().map(|texture| texture.index).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert!(
            textures
                .iter()
                .all(|texture| texture.width == 32 && texture.height == 16)
        );
    }

    #[test]
    fn extracts_each_cells_exact_pixels() {
        let mut catalog = TextureCatalog::new();
        catalog.insert("test.dmi", &sheet(2, 2, 2)).expect("insert");

        let first = catalog.textures().first().expect("first texture");
        let second = catalog.textures().get(1).expect("second texture");
        assert_eq!(
            first.pixels(),
            &[1, 0, 0, 255, 1, 0, 0, 255, 1, 0, 0, 255, 1, 0, 0, 255]
        );
        assert_eq!(
            second.pixels(),
            &[2, 0, 0, 255, 2, 0, 0, 255, 2, 0, 0, 255, 2, 0, 0, 255]
        );
    }

    #[test]
    fn texture_indices_continue_across_sheets() {
        let mut catalog = TextureCatalog::new();
        catalog.insert("first.dmi", &sheet(1, 1, 2)).expect("first insert");
        catalog.insert("second.dmi", &sheet(1, 1, 1)).expect("second insert");

        assert_eq!(catalog.lookup("second.dmi", 0).map(|texture| texture.index), Some(2));
    }

    #[test]
    fn a_truncated_cell_is_not_exposed() {
        let mut catalog = TextureCatalog::new();
        let mut file = sheet(32, 32, 2);
        file.sheet_width = 32;
        file.pixels.truncate((32 * 32 * 4) as usize);
        catalog.insert("truncated.dmi", &file).expect("insert");

        assert!(catalog.lookup("truncated.dmi", 0).is_some());
        assert_eq!(catalog.lookup("truncated.dmi", 1), None);
    }

    #[test]
    fn a_zero_cell_size_is_an_error() {
        let mut catalog = TextureCatalog::new();
        let mut file = sheet(32, 32, 1);
        file.metadata.width = 0;

        assert_eq!(
            catalog.insert("bad.dmi", &file),
            Err(TextureError::ZeroCellSize {
                icon: String::from("bad.dmi")
            })
        );
    }

    #[test]
    fn an_unknown_icon_or_cell_looks_up_to_nothing() {
        let mut catalog = TextureCatalog::new();
        catalog.insert("test.dmi", &sheet(32, 32, 1)).expect("insert");

        assert_eq!(catalog.lookup("missing.dmi", 0), None);
        assert_eq!(catalog.lookup("test.dmi", 9), None);
    }

    /// A sheet of one cell per named state.
    fn state_sheet(names: &[&str]) -> IconFile {
        let mut file = sheet(2, 2, names.len() as u32);
        file.metadata.states = names
            .iter()
            .enumerate()
            .map(|(index, name)| IconState {
                name: String::from(*name),
                dirs: 1,
                frames: 1,
                offset: index,
                ..Default::default()
            })
            .collect();

        file
    }

    #[test]
    fn insert_states_packs_only_the_named_states() {
        let mut catalog = TextureCatalog::new();
        let file = state_sheet(&["a", "b", "c"]);
        catalog
            .insert_states("test.dmi", &file, &BTreeSet::from(["a", "c"]))
            .expect("insert");

        assert_eq!(catalog.len(), 2);
        assert!(catalog.lookup("test.dmi", 0).is_some());
        assert!(catalog.lookup("test.dmi", 1).is_none());
        assert!(catalog.lookup("test.dmi", 2).is_some());
    }

    /// A skipped cell must not shift the cells after it, since `sprite_index` addresses by offset.
    #[test]
    fn a_skipped_cell_keeps_the_later_cells_addressable() {
        let mut catalog = TextureCatalog::new();
        let file = state_sheet(&["a", "b", "c"]);
        catalog
            .insert_states("test.dmi", &file, &BTreeSet::from(["c"]))
            .expect("insert");

        let third = catalog.lookup("test.dmi", 2).expect("third cell");
        let data = catalog.textures().get(third.index as usize).expect("texture");

        assert_eq!(data.pixels(), &[3, 0, 0, 255, 3, 0, 0, 255, 3, 0, 0, 255, 3, 0, 0, 255]);
    }

    #[test]
    fn insert_states_covers_every_dir_and_frame_of_a_state() {
        let mut catalog = TextureCatalog::new();
        let mut file = sheet(2, 2, 6);
        file.metadata.states = vec![
            IconState {
                name: String::from("a"),
                dirs: 4,
                frames: 1,
                offset: 0,
                ..Default::default()
            },
            IconState {
                name: String::from("b"),
                dirs: 1,
                frames: 2,
                offset: 4,
                ..Default::default()
            },
        ];

        catalog
            .insert_states("test.dmi", &file, &BTreeSet::from(["a"]))
            .expect("insert");

        assert_eq!(catalog.len(), 4);
        assert!((0..4).all(|cell| catalog.lookup("test.dmi", cell).is_some()));
        assert!((4..6).all(|cell| catalog.lookup("test.dmi", cell).is_none()));
    }

    #[test]
    fn a_state_the_sheet_does_not_have_packs_nothing() {
        let mut catalog = TextureCatalog::new();
        let file = state_sheet(&["a"]);
        catalog
            .insert_states("test.dmi", &file, &BTreeSet::from(["missing"]))
            .expect("insert");

        assert_eq!(catalog.len(), 0);
        assert!(catalog.lookup("test.dmi", 0).is_none());
    }
}
