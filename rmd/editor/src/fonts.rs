use std::{
    error::Error,
    mem,
    path::PathBuf,
    thread::{self, JoinHandle},
};

use dear_imgui_rs::{FontAtlas, FontConfig, FontId, FontSource, StbTrueTypeFontData};

use crate::settings::FontSettings;

const FONT_DATA: &[u8] = include_bytes!("../assets/FiraMono-Regular.ttf");
const MDI_FONT_DATA: &[u8] = include_bytes!("../assets/materialdesignicons-webfont.ttf");
const REFERENCE_SIZE: f32 = 16.0;
pub(crate) const BUNDLED_FONT_NAME: &str = "Fira Mono";

pub(crate) struct UiFont {
    id: FontId,
    loaded: FontSettings,
}

impl UiFont {
    pub fn new(atlas: &FontAtlas, settings: &mut FontSettings) -> Result<Self, Box<dyn Error>> {
        let sources = match font_sources(settings) {
            Ok(sources) => sources,
            Err(error) => {
                log::error!("could not load the font {:?}: {error}", settings.path);
                settings.path = None;
                font_sources(settings)?
            },
        };
        Ok(Self {
            id: atlas.add_font(&sources),
            loaded: settings.clone(),
        })
    }

    pub fn sync(&mut self, atlas: &FontAtlas, settings: &mut FontSettings) {
        let is_current = FontSettings {
            size: self.loaded.size,
            ..settings.clone()
        } == self.loaded;
        if is_current {
            return;
        }

        match font_sources(settings) {
            Ok(sources) => {
                atlas.remove_font(self.id);
                self.id = atlas.add_font(&sources);
                self.loaded = settings.clone();
            },
            Err(error) => {
                log::error!("could not load the font {:?}: {error}", settings.path);
                *settings = FontSettings {
                    size: settings.size,
                    ..self.loaded.clone()
                };
            },
        }
    }
}

fn font_sources(settings: &FontSettings) -> Result<[FontSource<'static>; 2], Box<dyn Error>> {
    let text = match &settings.path {
        Some(path) => StbTrueTypeFontData::from_file(path)?,
        None => StbTrueTypeFontData::from_slice(FONT_DATA)?,
    };
    let icons = StbTrueTypeFontData::from_slice(MDI_FONT_DATA)?;
    let config = || {
        FontConfig::new()
            .rasterizer_multiply(settings.brightness_percent as f32 / 100.0)
            .pixel_snap_h(settings.pixel_snap)
    };

    Ok([
        FontSource::stb_truetype_with_size(text, REFERENCE_SIZE).with_config(config()),
        FontSource::stb_truetype_with_size(icons, REFERENCE_SIZE).with_config(config()),
    ])
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FontFace {
    pub label: String,
    pub path: PathBuf,
}

#[derive(Default)]
pub(crate) enum SystemFonts {
    #[default]
    Unscanned,
    Scanning(JoinHandle<Vec<FontFace>>),
    Ready(Vec<FontFace>),
}

impl SystemFonts {
    pub fn faces(&mut self) -> Option<&[FontFace]> {
        if matches!(self, Self::Unscanned) {
            *self = Self::Scanning(thread::spawn(scan_system_fonts));
        }

        if matches!(self, Self::Scanning(handle) if handle.is_finished())
            && let Self::Scanning(handle) = mem::take(self)
        {
            *self = Self::Ready(handle.join().unwrap_or_default());
        }

        match self {
            Self::Ready(faces) => Some(faces),
            Self::Unscanned | Self::Scanning(_) => None,
        }
    }
}

fn scan_system_fonts() -> Vec<FontFace> {
    let mut database = fontdb::Database::new();
    database.load_system_fonts();
    let mut faces = database
        .faces()
        .filter_map(|face| {
            let (fontdb::Source::File(path) | fontdb::Source::SharedFile(path, _)) = &face.source else {
                return None;
            };

            let is_ttf = path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("ttf"));
            if face.index != 0 || !is_ttf {
                return None;
            }

            let (family, _) = face.families.first()?;

            Some(FontFace {
                label: face_label(family, face.weight, face.style),
                path: path.clone(),
            })
        })
        .collect::<Vec<_>>();
    faces.sort_by_key(|face| face.label.to_lowercase());
    faces.dedup_by(|a, b| a.path == b.path);

    faces
}

fn face_label(family: &str, weight: fontdb::Weight, style: fontdb::Style) -> String {
    let weight = match weight.0 {
        0..=149 => "Thin",
        150..=249 => "ExtraLight",
        250..=324 => "Light",
        325..=374 => "SemiLight",
        375..=449 => "",
        450..=549 => "Medium",
        550..=649 => "SemiBold",
        650..=749 => "Bold",
        750..=849 => "ExtraBold",
        _ => "Black",
    };
    let style = match style {
        fontdb::Style::Normal => "",
        fontdb::Style::Italic => "Italic",
        fontdb::Style::Oblique => "Oblique",
    };

    [family, weight, style]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use dear_imgui_rs::{Context, FramePrepareOptions};

    use super::*;

    #[test]
    fn fonts_swap_between_frames_and_bad_files_keep_the_current_font() {
        let _context = crate::ui::IMGUI_CONTEXT.lock().unwrap();
        let mut context = Context::create();
        context.prepare_frame(FramePrepareOptions::new([800.0, 600.0], 1.0 / 60.0).renderer_has_textures());
        let consumer = context.create_synchronous_renderer_consumer().unwrap();
        let mut settings = FontSettings::default();
        let mut font = UiFont::new(context.font_atlas(), &mut settings).unwrap();
        let frame = |context: &mut Context| {
            let frame = context.begin_frame();
            frame.ui().text("swap");
            drop(frame.render(&consumer));
        };

        frame(&mut context);
        for brightness_percent in [150, 80, 120] {
            settings.brightness_percent = brightness_percent;
            settings.pixel_snap = !settings.pixel_snap;
            font.sync(context.font_atlas(), &mut settings);
            frame(&mut context);
            assert_eq!(font.loaded, settings);
        }

        let loaded = font.loaded.clone();
        settings.path = Some(PathBuf::from("missing-font.ttf"));
        settings.size = 20;
        font.sync(context.font_atlas(), &mut settings);
        frame(&mut context);
        assert_eq!(settings.path, None);
        assert_eq!(settings.size, 20);
        assert_eq!(font.loaded, loaded);
    }

    #[test]
    fn face_labels_skip_regular_weight_and_style() {
        assert_eq!(
            face_label("Fira Mono", fontdb::Weight::NORMAL, fontdb::Style::Normal),
            "Fira Mono"
        );
        assert_eq!(
            face_label("Fira Mono", fontdb::Weight::BOLD, fontdb::Style::Italic),
            "Fira Mono Bold Italic"
        );
        assert_eq!(
            face_label("Caskaydia", fontdb::Weight(350), fontdb::Style::Normal),
            "Caskaydia SemiLight"
        );
    }
}
