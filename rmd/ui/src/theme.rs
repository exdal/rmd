pub mod style;

use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
};

use dear_imgui_rs::{Style, StyleColor, ThemePreset};
use serde::{Deserialize, Serialize};
use toml::{Table, Value};

use self::style::{STYLE_COLORS, STYLE_FIELDS};
use crate::color::byte;

pub const DEFAULT_THEME: &str = "imgui_dark";
const LIGHT_THEME: &str = "imgui_light";
const TABLE_ROW_ALT_ALPHA_SCALE: f32 = 0.4;

pub(crate) fn hex_color(rgba: [f32; 4]) -> String {
    let [r, g, b, a] = rgba.map(byte);

    format!("#{r:02X}{g:02X}{b:02X}{a:02X}")
}

fn parse_hex_color(hex: &str) -> Option<[f32; 4]> {
    let digits = hex.strip_prefix('#').unwrap_or(hex);
    if !matches!(digits.len(), 6 | 8) || !digits.is_ascii() {
        return None;
    }

    let channel = |index: usize| {
        digits
            .get(index * 2..index * 2 + 2)
            .map_or(Some(255), |pair| u8::from_str_radix(pair, 16).ok())
    };

    Some([channel(0)?, channel(1)?, channel(2)?, channel(3)?].map(|channel| f32::from(channel) / 255.0))
}

#[derive(Debug, Serialize, Deserialize)]
struct ThemeFile {
    #[serde(default)]
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(default)]
    style: Table,
    #[serde(default)]
    colors: Table,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    pub name: String,
    pub url: String,
    pub style: Style,
}

impl Theme {
    pub fn web_url(&self) -> Option<&str> {
        let url = self.url.trim();
        let is_web = ["https://", "http://"].iter().any(|scheme| {
            url.get(..scheme.len())
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case(scheme))
        });

        is_web.then_some(url)
    }

    fn parse(id: &str, source: &str, base: &Style) -> Result<Self, toml::de::Error> {
        let file: ThemeFile = toml::from_str(source)?;
        let mut style = base.clone();
        for (key, value) in &file.style {
            match STYLE_FIELDS.iter().find(|field| field.key == key) {
                Some(field) if field.read(&mut style, value) => {},
                Some(_) => log::warn!("theme {id}: invalid value for style.{key}"),
                None => log::warn!("theme {id}: unknown style.{key}"),
            }
        }

        for (key, value) in &file.colors {
            let Some((color, _)) = STYLE_COLORS.iter().find(|(_, name)| name == key) else {
                log::warn!("theme {id}: unknown colors.{key}");
                continue;
            };

            match value.as_str().and_then(parse_hex_color) {
                Some(rgba) => style.set_color(*color, rgba),
                None => log::warn!("theme {id}: colors.{key} is not a \"#RRGGBBAA\" color"),
            }
        }

        let name = if file.name.trim().is_empty() {
            id.to_owned()
        } else {
            file.name
        };

        Ok(Self {
            name,
            url: file.url.unwrap_or_default(),
            style,
        })
    }

    fn to_toml(&self) -> Result<String, toml::ser::Error> {
        let file = ThemeFile {
            name: self.name.clone(),
            url: (!self.url.is_empty()).then(|| self.url.clone()),
            style: STYLE_FIELDS
                .iter()
                .map(|field| (field.key.to_owned(), field.write(&self.style)))
                .collect(),
            colors: STYLE_COLORS
                .iter()
                .map(|(color, name)| ((*name).to_owned(), Value::String(hex_color(self.style.color(*color)))))
                .collect(),
        };

        toml::to_string_pretty(&file)
    }
}

fn builtin_themes(default_style: &Style) -> [(&'static str, Theme); 2] {
    let theme = |name: &str, preset: ThemePreset| {
        let mut style = default_style.clone();
        dear_imgui_rs::Theme {
            preset,
            ..Default::default()
        }
        .apply_to_style(&mut style);
        let mut alternate_row = style.color(StyleColor::TableRowBgAlt);
        alternate_row[3] *= TABLE_ROW_ALT_ALPHA_SCALE;
        style.set_color(StyleColor::TableRowBgAlt, alternate_row);
        for (color, _) in STYLE_COLORS {
            let rgba = style.color(color).map(|channel| f32::from(byte(channel)) / 255.0);
            style.set_color(color, rgba);
        }

        Theme {
            name: name.to_owned(),
            url: String::new(),
            style,
        }
    };

    [
        (DEFAULT_THEME, theme("ImGui Dark", ThemePreset::Dark)),
        (LIGHT_THEME, theme("ImGui Light", ThemePreset::Light)),
    ]
}

struct ThemeEntry {
    id: String,
    theme: Theme,
    saved: Theme,
}

impl ThemeEntry {
    fn new(id: String, theme: Theme) -> Self {
        Self {
            id,
            saved: theme.clone(),
            theme,
        }
    }
}

pub struct Themes {
    dir: Option<PathBuf>,
    builtins: [(&'static str, Theme); 2],
    entries: Vec<ThemeEntry>,
    active: usize,
}

impl Themes {
    pub fn load(dir: Option<PathBuf>, default_style: &Style, selected: &str) -> Self {
        let builtins = builtin_themes(default_style);
        let mut entries = Vec::new();
        if let Some(dir) = &dir {
            if let Err(error) = write_missing(dir, &builtins) {
                log::error!("writing built-in themes: {error}");
            }

            entries = read_themes(dir, &builtins);
        }

        for (id, theme) in &builtins {
            if !entries.iter().any(|entry| entry.id == *id) {
                entries.push(ThemeEntry::new((*id).to_owned(), theme.clone()));
            }
        }

        let mut themes = Self {
            dir,
            builtins,
            entries,
            active: 0,
        };
        themes.sort();
        if !themes.select(selected) {
            themes.select(DEFAULT_THEME);
        }

        themes
    }

    fn sort(&mut self) {
        self.entries.sort_by_key(|entry| entry.theme.name.to_lowercase());
    }

    pub fn active_style(&self) -> &Style {
        &self.entries[self.active].theme.style
    }

    pub fn active_id(&self) -> &str {
        &self.entries[self.active].id
    }

    pub fn active_theme(&self) -> &Theme {
        &self.entries[self.active].theme
    }

    pub fn active_theme_mut(&mut self) -> &mut Theme {
        &mut self.entries[self.active].theme
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &Theme)> {
        self.entries.iter().map(|entry| (entry.id.as_str(), &entry.theme))
    }

    pub fn select(&mut self, id: &str) -> bool {
        let Some(index) = self.entries.iter().position(|entry| entry.id == id) else {
            return false;
        };

        self.active = index;

        true
    }

    pub fn is_dirty(&self) -> bool {
        let entry = &self.entries[self.active];

        entry.theme != entry.saved
    }

    pub fn save_active(&mut self) -> Result<(), Box<dyn Error>> {
        let dir = self.dir.as_deref().ok_or("the themes folder is unavailable")?;
        let entry = &mut self.entries[self.active];
        fs::create_dir_all(dir)?;
        fs::write(theme_path(dir, &entry.id), entry.theme.to_toml()?)?;
        entry.saved = entry.theme.clone();

        Ok(())
    }

    pub fn revert_active(&mut self) {
        let entry = &mut self.entries[self.active];

        entry.theme = entry.saved.clone();
    }

    fn active_builtin(&self) -> Option<&Theme> {
        let id = self.active_id();

        self.builtins
            .iter()
            .find(|(builtin, _)| *builtin == id)
            .map(|(_, theme)| theme)
    }

    pub fn is_active_builtin(&self) -> bool {
        self.active_builtin().is_some()
    }

    pub fn is_active_default(&self) -> bool {
        self.active_builtin() == Some(self.active_theme())
    }

    pub fn reset_active(&mut self) {
        if let Some(builtin) = self.active_builtin().cloned() {
            *self.active_theme_mut() = builtin;
        }
    }

    pub fn create(&mut self, name: &str) -> Result<&str, Box<dyn Error>> {
        let dir = self.dir.as_deref().ok_or("the themes folder is unavailable")?;
        let id = self.unused_id(dir, name);
        let theme = Theme {
            name: name.trim().to_owned(),
            url: String::new(),
            style: self.active_style().clone(),
        };
        fs::create_dir_all(dir)?;
        fs::write(theme_path(dir, &id), theme.to_toml()?)?;
        self.entries.push(ThemeEntry::new(id.clone(), theme));
        self.sort();
        self.select(&id);

        Ok(self.active_id())
    }

    fn unused_id(&self, dir: &Path, name: &str) -> String {
        let mut slug = String::new();
        for character in name.trim().chars().flat_map(char::to_lowercase) {
            if character.is_ascii_alphanumeric() {
                slug.push(character);
            } else if !slug.is_empty() && !slug.ends_with('_') {
                slug.push('_');
            }
        }

        let slug = match slug.trim_end_matches('_') {
            "" => "theme",
            slug => slug,
        };

        let is_taken = |id: &str| self.entries.iter().any(|entry| entry.id == id) || theme_path(dir, id).exists();
        let mut id = slug.to_owned();
        let mut suffix = 2;
        while is_taken(&id) {
            id = format!("{slug}_{suffix}");
            suffix += 1;
        }

        id
    }
}

fn theme_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.toml"))
}

fn write_missing(dir: &Path, builtins: &[(&str, Theme)]) -> Result<(), Box<dyn Error>> {
    fs::create_dir_all(dir)?;
    for (id, theme) in builtins {
        let path = theme_path(dir, id);
        if !path.exists() {
            fs::write(path, theme.to_toml()?)?;
        }
    }

    Ok(())
}

fn read_themes(dir: &Path, builtins: &[(&str, Theme)]) -> Vec<ThemeEntry> {
    let read_dir = match fs::read_dir(dir) {
        Ok(read_dir) => read_dir,
        Err(error) => {
            log::error!("reading {}: {error}", dir.display());

            return Vec::new();
        },
    };

    let mut entries = Vec::new();
    for path in read_dir.filter_map(Result::ok).map(|entry| entry.path()) {
        if path.extension().is_none_or(|extension| extension != "toml") {
            continue;
        }

        let Some(id) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };

        let base = builtins
            .iter()
            .find(|(builtin, _)| *builtin == id)
            .map_or(&builtins[0].1.style, |(_, theme)| &theme.style);
        let theme = fs::read_to_string(&path)
            .map_err(Box::<dyn Error>::from)
            .and_then(|source| Ok(Theme::parse(id, &source, base)?));
        match theme {
            Ok(theme) => entries.push(ThemeEntry::new(id.to_owned(), theme)),
            Err(error) => log::error!("loading theme {}: {error}", path.display()),
        }
    }

    entries
}

#[cfg(test)]
mod tests;
