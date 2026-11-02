use ash::vk;
use dmi::IconFile;
use vir::{Image, ImageAttachment};

use super::{
    Renderer,
    upload::{TextureSource, UPLOAD_BATCH_BYTES, batch_ranges, destroy_images, loaded_images},
};
use crate::{
    GpuError, SpriteInstance,
    texture::{TextureCatalog, TextureData},
};

const SHEET_BUDGET_BYTES: usize = 512 * 1024 * 1024;

pub(crate) struct TextureImage {
    pub(super) image: Image,
    pub(super) view: vk::ImageView,
}

impl TextureImage {
    pub(crate) fn attachment(&self, layout: vk::ImageLayout) -> ImageAttachment {
        ImageAttachment::from_image(&self.image, layout).with_image_view(self.view)
    }
}

pub(super) enum SheetState {
    Unloaded,
    Loaded { image: TextureImage, last_used: u64 },
    Failed,
}

pub(crate) struct Sheet {
    data: TextureData,
    pub(super) state: SheetState,
}

impl Sheet {
    fn new(data: TextureData) -> Self {
        Self {
            data,
            state: SheetState::Unloaded,
        }
    }

    fn is_unloaded(&self) -> bool {
        matches!(self.state, SheetState::Unloaded)
    }

    pub(crate) fn image(&self) -> Option<&TextureImage> {
        match &self.state {
            SheetState::Loaded { image, .. } => Some(image),
            SheetState::Unloaded | SheetState::Failed => None,
        }
    }
}

fn new_sheets(textures: &[TextureData]) -> Vec<Sheet> {
    textures.iter().cloned().map(Sheet::new).collect()
}

fn builtin_sheets(sheets: &[Sheet]) -> Vec<usize> {
    sheets
        .iter()
        .enumerate()
        .filter(|(_, sheet)| sheet.data.builtin_pixels().is_some())
        .map(|(index, _)| index)
        .collect()
}

fn unloaded_sheets(sheets: &[Sheet], requested: &[u32]) -> Vec<usize> {
    let mut indices: Vec<usize> = requested
        .iter()
        .map(|&index| index as usize)
        .filter(|&index| sheets.get(index).is_some_and(Sheet::is_unloaded))
        .collect();
    indices.sort_unstable();
    indices.dedup();

    indices
}

pub(super) fn note_sheets(requested: &mut Vec<u32>, sprites: &[SpriteInstance]) {
    requested.extend(sprites.iter().map(|sprite| sprite.texture.index));
}

fn read_sheet(data: &TextureData, dimension_limit: u32) -> Result<Option<IconFile>, GpuError> {
    if data.width() > dimension_limit || data.height() > dimension_limit {
        return Err(GpuError::TextureSheetTooLarge {
            path: data.path().display().to_string(),
            width: data.width(),
            height: data.height(),
            limit: dimension_limit,
        });
    }

    if data.builtin_pixels().is_some() {
        return Ok(None);
    }

    let path = data.path();
    let file = IconFile::load(path).map_err(|error| GpuError::TextureLoad {
        path: path.display().to_string(),
        message: error.to_string(),
    })?;
    if file.sheet_width != data.width() || file.sheet_height != data.height() {
        return Err(GpuError::TextureSourceChanged {
            path: path.display().to_string(),
            expected_width: data.width(),
            expected_height: data.height(),
            actual_width: file.sheet_width,
            actual_height: file.sheet_height,
        });
    }

    Ok(Some(file))
}

#[derive(Debug, Clone, Copy)]
struct SheetUsage {
    bytes: usize,
    last_used: Option<u64>,
    is_pinned: bool,
}

fn sheet_usages(sheets: &[Sheet], pinned: &[bool]) -> Vec<SheetUsage> {
    sheets
        .iter()
        .zip(pinned)
        .map(|(sheet, &is_pinned)| SheetUsage {
            bytes: sheet.data.decoded_bytes(),
            last_used: match sheet.state {
                SheetState::Loaded { last_used, .. } => Some(last_used),
                SheetState::Unloaded | SheetState::Failed => None,
            },
            is_pinned,
        })
        .collect()
}

fn loaded_bytes(sheets: &[SheetUsage]) -> usize {
    sheets
        .iter()
        .filter(|sheet| sheet.last_used.is_some())
        .fold(0usize, |total, sheet| total.saturating_add(sheet.bytes))
}

fn sheets_to_evict(sheets: &[SheetUsage], incoming_bytes: usize, budget: usize) -> Vec<usize> {
    let mut total = loaded_bytes(sheets).saturating_add(incoming_bytes);
    if total <= budget {
        return Vec::new();
    }

    let mut candidates = sheets
        .iter()
        .enumerate()
        .filter(|(_, sheet)| !sheet.is_pinned)
        .filter_map(|(index, sheet)| Some((index, sheet.last_used?)))
        .collect::<Vec<(_, _)>>();
    candidates.sort_by_key(|&(index, last_used)| (last_used, index));

    let mut evicted = Vec::new();
    for (index, _) in candidates {
        if total <= budget {
            break;
        }

        total = total.saturating_sub(sheets[index].bytes);
        evicted.push(index);
    }

    evicted
}

pub(super) fn sheet_indices(sprites: &[SpriteInstance]) -> Vec<u32> {
    let mut indices = Vec::new();
    note_sheets(&mut indices, sprites);
    indices.sort_unstable();
    indices.dedup();

    indices
}

pub(super) fn merge_sheets(set: &mut Vec<u32>, added: &[u32]) {
    set.extend_from_slice(added);
    set.sort_unstable();
    set.dedup();
}

impl Renderer {
    pub fn set_textures(&mut self, catalog: &TextureCatalog) -> Result<(), GpuError> {
        let requested = u32::try_from(catalog.len()).map_err(|_| GpuError::TextureUploadTooLarge)?;
        if requested > self.bindless.count {
            return Err(GpuError::TextureCapacityExceeded {
                requested,
                capacity: self.bindless.count,
            });
        }

        self.device.wait_idle()?;

        let old = std::mem::replace(&mut self.sheets, new_sheets(catalog.textures()));
        destroy_images(&mut self.device, loaded_images(old));
        self.recorded = None;
        self.uploaded.clear();

        let uploaded = self.upload_sheets(&builtin_sheets(&self.sheets));
        self.write_descriptors()?;

        uploaded
    }

    fn load_sheets(&mut self, requested: &[u32]) -> Result<(), GpuError> {
        let indices = unloaded_sheets(&self.sheets, requested);
        if indices.is_empty() {
            return Ok(());
        }

        self.device.wait_idle()?;
        self.evict_sheets(&indices, requested);
        self.upload_sheets(&indices)?;

        self.write_descriptors()
    }

    fn evict_sheets(&mut self, incoming: &[usize], requested: &[u32]) {
        let incoming_bytes = incoming.iter().fold(0usize, |total, &index| {
            total.saturating_add(self.sheets[index].data.decoded_bytes())
        });
        let mut pinned = vec![false; self.sheets.len()];
        let pins = self
            .uploaded
            .iter()
            .flat_map(|view| &view.sheets)
            .chain(self.previews.iter().flatten().flat_map(|preview| &preview.sheets))
            .chain(requested);
        for &index in pins {
            if let Some(is_pinned) = pinned.get_mut(index as usize) {
                *is_pinned = true;
            }
        }

        for (is_pinned, sheet) in pinned.iter_mut().zip(&self.sheets) {
            let is_current = matches!(sheet.state, SheetState::Loaded { last_used, .. } if last_used == self.frame);
            *is_pinned |= is_current || sheet.data.builtin_pixels().is_some();
        }

        let evicted = sheets_to_evict(&sheet_usages(&self.sheets, &pinned), incoming_bytes, SHEET_BUDGET_BYTES);
        for index in evicted {
            let state = std::mem::replace(&mut self.sheets[index].state, SheetState::Unloaded);
            if let SheetState::Loaded { image, .. } = state {
                destroy_images(&mut self.device, [image]);
            }
        }
    }

    pub(super) fn load_requested_sheets(&mut self) -> Result<(), GpuError> {
        let requested = std::mem::take(&mut self.requested_sheets);
        for &index in &requested {
            let Some(sheet) = self.sheets.get_mut(index as usize) else {
                continue;
            };

            if let SheetState::Loaded { last_used, .. } = &mut sheet.state {
                *last_used = self.frame;
            }
        }

        let result = self.load_sheets(&requested);
        self.requested_sheets = requested;
        self.requested_sheets.clear();

        result
    }

    pub(super) fn write_descriptors(&self) -> Result<(), GpuError> {
        let fallback = self.fallback.as_ref().ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
        let info = vk::DescriptorImageInfo::default()
            .sampler(self.sampler)
            .image_view(fallback.view)
            .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
        let mut infos = vec![info; self.bindless.count as usize];

        for (info, sheet) in infos.iter_mut().zip(&self.sheets) {
            if let Some(image) = sheet.image() {
                info.image_view = image.view;
            }
        }

        self.bindless.write(&infos, &self.device);

        Ok(())
    }

    fn upload_sheets(&mut self, indices: &[usize]) -> Result<(), GpuError> {
        let dimension_limit = self.device.max_image_dimension_2d.min(u16::MAX.into());
        let sizes = indices
            .iter()
            .map(|&index| self.sheets.get(index).map_or(0, |sheet| sheet.data.decoded_bytes()))
            .collect::<Vec<_>>();
        let ranges = batch_ranges(&sizes, UPLOAD_BATCH_BYTES).ok_or(GpuError::TextureUploadTooLarge)?;

        for range in ranges {
            let batch = indices.get(range).ok_or(GpuError::TextureUploadTooLarge)?;
            let mut loaded = Vec::with_capacity(batch.len());

            for &index in batch {
                let sheet = self.sheets.get_mut(index).ok_or(GpuError::TextureUploadTooLarge)?;
                match read_sheet(&sheet.data, dimension_limit) {
                    Ok(file) => loaded.push((index, file)),
                    Err(error) => {
                        log::warn!("sprite sheet unavailable: {error}");
                        sheet.state = SheetState::Failed;
                    },
                }
            }

            let sources = loaded
                .iter()
                .map(|(index, file)| {
                    let data = &self.sheets[*index].data;

                    TextureSource {
                        width: data.width(),
                        height: data.height(),
                        pixels: data
                            .builtin_pixels()
                            .unwrap_or_else(|| &file.as_ref().expect("file texture").pixels),
                    }
                })
                .collect::<Vec<_>>();
            let images = self.upload_images(&sources)?;

            for ((index, _), image) in loaded.iter().zip(images) {
                self.sheets[*index].state = SheetState::Loaded {
                    image,
                    last_used: self.frame,
                };
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use dmi::metadata::{IconState, Metadata};

    use super::*;
    use crate::texture::TextureCatalog;

    fn sheet_catalog() -> TextureCatalog {
        let mut catalog = TextureCatalog::new();
        let file = IconFile {
            path: PathBuf::from("test.dmi"),
            metadata: Metadata {
                version: String::from("4.0"),
                width: 32,
                height: 32,
                states: vec![IconState {
                    name: String::from("a"),
                    dirs: 1,
                    frames: 1,
                    ..Default::default()
                }],
            },
            sheet_width: 32,
            sheet_height: 32,
            pixels: vec![0; 32 * 32 * 4],
        };
        catalog.insert("test.dmi", &file).expect("insert sheet");
        catalog.insert_missing_icon().expect("insert missing icon");
        catalog.insert("other.dmi", &file).expect("insert sheet");

        catalog
    }

    #[test]
    fn new_sheets_start_unloaded_and_only_builtins_load_eagerly() {
        let catalog = sheet_catalog();
        let sheets = new_sheets(catalog.textures());

        assert_eq!(sheets.len(), 3);
        assert!(sheets.iter().all(|sheet| matches!(sheet.state, SheetState::Unloaded)));
        assert_eq!(builtin_sheets(&sheets), [1]);
    }

    fn usage(last_used: Option<u64>, is_pinned: bool) -> SheetUsage {
        SheetUsage {
            bytes: 100,
            last_used,
            is_pinned,
        }
    }

    #[test]
    fn nothing_is_evicted_while_under_budget() {
        let sheets = [usage(Some(1), false), usage(Some(2), false)];

        assert!(sheets_to_evict(&sheets, 100, 300).is_empty());
        assert_eq!(loaded_bytes(&sheets), 200);
    }

    #[test]
    fn eviction_takes_least_recently_used_until_under_budget() {
        let sheets = [
            usage(Some(5), false),
            usage(Some(1), false),
            usage(Some(3), false),
            usage(Some(2), false),
        ];

        assert_eq!(sheets_to_evict(&sheets, 100, 300), [1, 3]);
    }

    #[test]
    fn pinned_unloaded_and_failed_sheets_are_never_evicted() {
        let sheets = [
            usage(Some(1), true),
            usage(None, false),
            usage(Some(2), false),
            usage(Some(3), true),
        ];

        assert_eq!(sheets_to_evict(&sheets, 500, 100), [2]);
    }

    #[test]
    fn merged_sheet_indices_stay_sorted_and_unique() {
        let mut set = vec![1, 4, 9];
        merge_sheets(&mut set, &[9, 2, 4, 12]);

        assert_eq!(set, [1, 2, 4, 9, 12]);
    }

    #[test]
    fn requested_sheets_keep_only_unloaded_in_range_unique_indices() {
        let catalog = sheet_catalog();
        let mut sheets = new_sheets(catalog.textures());
        sheets[0].state = SheetState::Failed;

        assert_eq!(unloaded_sheets(&sheets, &[2, 1, 2, 0, 3, u32::MAX]), [1, 2]);
        assert!(unloaded_sheets(&sheets, &[0, 7]).is_empty());
        assert!(unloaded_sheets(&sheets, &[]).is_empty());
    }
}
