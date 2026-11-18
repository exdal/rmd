use std::{
    error::Error,
    fs, io,
    path::{Path, PathBuf},
};

use render::HighlightStyle;
use serde::{Deserialize, Serialize};
use ui::{fonts::FontSettings, theme::DEFAULT_THEME};

use crate::frame::options::FrameOptions;

use keybind::KeyBindings;
use paths::{absolute, settings_path};

const MAX_RECENT: usize = 10;
const DEFAULT_PREFERRED_EDITOR: &str = "code {root} --goto {file}:{line}:{column}";
pub(crate) const DEFAULT_COOP_PORT: u16 = 3131;
pub const UI_SCALE_PERCENT: std::ops::RangeInclusive<u32> = 50..=200;
pub const FONT_SIZE: std::ops::RangeInclusive<u32> = 8..=40;
pub const FONT_BRIGHTNESS_PERCENT: std::ops::RangeInclusive<u32> = 50..=200;

/// how a selected or hovered sprite is marked in the viewport
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SelectionHighlight {
    #[default]
    Outline,
    Tint,
}

impl SelectionHighlight {
    pub const ALL: [Self; 2] = [Self::Outline, Self::Tint];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Outline => "Outline",
            Self::Tint => "Tint",
        }
    }

    pub const fn style(self) -> HighlightStyle {
        match self {
            Self::Outline => HighlightStyle::Outline,
            Self::Tint => HighlightStyle::Tint,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct ObjectTreeSearchOptions {
    pub type_paths: bool,
    pub names: bool,
}

impl Default for ObjectTreeSearchOptions {
    fn default() -> Self {
        Self {
            type_paths: true,
            names: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct CoopSettings {
    pub nick: String,
    pub address: String,
    pub port: u16,
}

impl Default for CoopSettings {
    fn default() -> Self {
        Self {
            nick: String::new(),
            address: String::new(),
            port: DEFAULT_COOP_PORT,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Panel {
    ObjectTree,
    Git,
    Inspector,
    Search,
    MapIssues,
    Runtimes,
}

impl Panel {
    pub const ALL: [Self; 6] = [
        Self::ObjectTree,
        Self::Git,
        Self::Inspector,
        Self::Search,
        Self::MapIssues,
        Self::Runtimes,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct OpenPanels {
    pub object_tree: bool,
    pub git: bool,
    pub inspector: bool,
    pub search: bool,
    #[serde(skip)]
    pub map_issues: bool,
    pub runtimes: bool,
}

impl OpenPanels {
    pub const fn is_open(&self, panel: Panel) -> bool {
        match panel {
            Panel::ObjectTree => self.object_tree,
            Panel::Git => self.git,
            Panel::Inspector => self.inspector,
            Panel::Search => self.search,
            Panel::MapIssues => self.map_issues,
            Panel::Runtimes => self.runtimes,
        }
    }

    pub const fn get_mut(&mut self, panel: Panel) -> &mut bool {
        match panel {
            Panel::ObjectTree => &mut self.object_tree,
            Panel::Git => &mut self.git,
            Panel::Inspector => &mut self.inspector,
            Panel::Search => &mut self.search,
            Panel::MapIssues => &mut self.map_issues,
            Panel::Runtimes => &mut self.runtimes,
        }
    }
}

impl Default for OpenPanels {
    fn default() -> Self {
        Self {
            object_tree: true,
            git: true,
            inspector: true,
            search: true,
            map_issues: false,
            runtimes: false,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct ObjectTreeFilterOptions {
    pub atom: bool,
    pub movable: bool,
    pub obj: bool,
    pub turf: bool,
    pub custom_enabled: bool,
    pub custom_type_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RecentMap {
    pub map: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ProfileSelection {
    pub environment: PathBuf,
    pub profile: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ForcedProfileSelection {
    pub environment: PathBuf,
    pub profile: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Settings {
    pub maximized: bool,
    pub preferred_editor: String,
    pub show_areas: bool,
    pub show_area_outlines: bool,
    pub show_lighting: bool,
    pub minimum_light_brightness_percent: u32,
    pub focus_windows_on_hover: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ui_scale_override_percent: Option<u32>,
    pub theme: String,
    pub font: FontSettings,
    pub tile_place_flash: bool,
    pub selection_guide_line: bool,
    pub show_tile_grid: bool,
    pub tile_grid_min_pixels: u32,
    pub show_tile_grid_axis: bool,
    pub show_selected_pixel_grid: bool,
    pub mirror_camera: bool,
    pub selected_pixel_grid_min_pixels: u32,
    pub show_pixel_grid_axis: bool,
    pub selection_highlight: SelectionHighlight,
    pub object_tree_line_indicators: bool,
    pub object_tree_search: ObjectTreeSearchOptions,
    pub object_tree_filter: ObjectTreeFilterOptions,
    pub keybindings: KeyBindings,
    pub recent_codebases: Vec<PathBuf>,
    pub trusted_codebases: Vec<PathBuf>,
    pub recent: Vec<RecentMap>,
    pub profile_selections: Vec<ProfileSelection>,
    pub forced_profile_selections: Vec<ForcedProfileSelection>,
    pub optimizations_enabled: bool,
    pub bake_enabled: bool,
    pub git_enabled: bool,
    pub blame_depth: u32,
    pub check_for_updates: bool,
    pub sanitize_vars_on_save: bool,
    pub pinned_vars: Vec<String>,
    pub pinned_types: Vec<String>,
    pub coop: CoopSettings,
    pub panels: OpenPanels,
    pub show_dm_ui: bool,
}

pub(crate) struct SettingsLoad {
    pub settings: Settings,
    pub needs_keybind_preset: bool,
}

impl SettingsLoad {
    fn from_file_result(result: Result<Option<Settings>, Box<dyn Error>>) -> Self {
        match result {
            Ok(Some(mut settings)) => {
                settings.normalize();

                Self {
                    settings,
                    needs_keybind_preset: false,
                }
            },
            Ok(None) => Self {
                settings: Settings::default(),
                needs_keybind_preset: true,
            },
            Err(error) => {
                log::error!("loading settings: {error}");

                Self {
                    settings: Settings::default(),
                    needs_keybind_preset: false,
                }
            },
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            maximized: false,
            preferred_editor: String::from(DEFAULT_PREFERRED_EDITOR),
            show_areas: false,
            show_area_outlines: true,
            show_lighting: true,
            minimum_light_brightness_percent: 0,
            focus_windows_on_hover: true,
            ui_scale_override_percent: None,
            theme: String::from(DEFAULT_THEME),
            font: FontSettings::default(),
            tile_place_flash: true,
            selection_guide_line: true,
            show_tile_grid: true,
            tile_grid_min_pixels: 10,
            show_tile_grid_axis: true,
            show_selected_pixel_grid: true,
            mirror_camera: false,
            selected_pixel_grid_min_pixels: 3,
            show_pixel_grid_axis: true,
            selection_highlight: SelectionHighlight::Outline,
            object_tree_line_indicators: true,
            object_tree_search: ObjectTreeSearchOptions::default(),
            object_tree_filter: ObjectTreeFilterOptions::default(),
            keybindings: KeyBindings::default(),
            recent_codebases: Vec::new(),
            trusted_codebases: Vec::new(),
            recent: Vec::new(),
            profile_selections: Vec::new(),
            forced_profile_selections: Vec::new(),
            optimizations_enabled: true,
            bake_enabled: true,
            git_enabled: true,
            blame_depth: 500,
            check_for_updates: true,
            sanitize_vars_on_save: false,
            pinned_vars: Vec::new(),
            pinned_types: Vec::new(),
            coop: CoopSettings::default(),
            panels: OpenPanels::default(),
            show_dm_ui: true,
        }
    }
}

impl Settings {
    pub fn load() -> SettingsLoad {
        SettingsLoad::from_file_result(Self::try_load())
    }

    pub fn save(&self) {
        #[cfg(not(test))]
        if let Err(error) = self.try_save() {
            log::error!("saving settings: {error}");
        }
    }

    pub fn apply_to(&self, options: &mut FrameOptions) {
        options.show_areas = self.show_areas;
        options.show_area_outlines = self.show_area_outlines;
        options.show_lighting = self.show_lighting;
        options.minimum_light_brightness_percent = self.minimum_light_brightness_percent.min(100);
    }

    pub fn capture_from(&mut self, options: &FrameOptions) {
        self.show_areas = options.show_areas;
        self.show_area_outlines = options.show_area_outlines;
        self.show_lighting = options.show_lighting;
        self.minimum_light_brightness_percent = options.minimum_light_brightness_percent.min(100);
    }

    fn normalize(&mut self) {
        self.keybindings.resolve_duplicates();
        self.minimum_light_brightness_percent = self.minimum_light_brightness_percent.min(100);
        self.ui_scale_override_percent = self
            .ui_scale_override_percent
            .map(|scale| scale.clamp(*UI_SCALE_PERCENT.start(), *UI_SCALE_PERCENT.end()));
        self.blame_depth = self.blame_depth.clamp(1, 10_000);
        self.font.size = self.font.size.clamp(*FONT_SIZE.start(), *FONT_SIZE.end());
        self.font.brightness_percent = self
            .font
            .brightness_percent
            .clamp(*FONT_BRIGHTNESS_PERCENT.start(), *FONT_BRIGHTNESS_PERCENT.end());
        if !self.object_tree_search.type_paths && !self.object_tree_search.names {
            self.object_tree_search = ObjectTreeSearchOptions::default();
        }

        // forced profiles used to be stored as an enum
        for selection in &mut self.forced_profile_selections {
            if selection.profile == "second_city" {
                selection.profile = String::from("secondcity");
            }
        }
    }

    pub fn record_recent(&mut self, environment: Option<&Path>, map: &Path) {
        self.push_recent(environment, map);
        self.save();
    }

    fn push_recent(&mut self, environment: Option<&Path>, map: &Path) {
        let entry = RecentMap {
            map: absolute(map),
            environment: environment.map(absolute),
        };

        self.recent.retain(|recent| recent.map != entry.map);
        self.recent.insert(0, entry);
        self.recent.truncate(MAX_RECENT);
    }

    pub fn forget_recent(&mut self, map: &Path) {
        self.remove_recent(map);
        self.save();
    }

    fn remove_recent(&mut self, map: &Path) {
        let map = absolute(map);

        self.recent.retain(|recent| recent.map != map);
    }

    pub fn record_codebase(&mut self, codebase: &Path) {
        self.push_codebase(codebase);
        self.save();
    }

    fn push_codebase(&mut self, codebase: &Path) {
        let codebase = absolute(codebase);

        self.recent_codebases.retain(|recent| *recent != codebase);
        self.recent_codebases.insert(0, codebase);
        self.recent_codebases.truncate(MAX_RECENT);
    }

    pub fn is_trusted(&self, codebase: &Path) -> bool {
        let codebase = absolute(codebase);

        self.trusted_codebases.contains(&codebase)
    }

    pub fn trust_codebase(&mut self, codebase: &Path) {
        let codebase = absolute(codebase);
        if !self.trusted_codebases.contains(&codebase) {
            self.trusted_codebases.push(codebase);
        }

        self.save();
    }

    pub fn forget_codebase(&mut self, codebase: &Path) {
        self.remove_codebase(codebase);
        self.save();
    }

    fn remove_codebase(&mut self, codebase: &Path) {
        let codebase = absolute(codebase);

        self.recent_codebases.retain(|recent| *recent != codebase);
    }

    pub fn recent_maps_for(&self, environment: &Path) -> impl Iterator<Item = &RecentMap> {
        let environment = absolute(environment);

        self.recent
            .iter()
            .filter(move |recent| recent.environment.as_deref() == Some(environment.as_path()))
    }

    pub fn profile_for(&self, environment: &Path) -> Option<&str> {
        let environment = absolute(environment);

        self.profile_selections
            .iter()
            .find(|selection| selection.environment == environment)
            .map(|selection| selection.profile.as_str())
    }

    pub fn set_profile_for(&mut self, environment: &Path, profile: Option<&str>) {
        let environment = absolute(environment);
        self.profile_selections
            .retain(|selection| selection.environment != environment);
        if let Some(profile) = profile {
            self.profile_selections.push(ProfileSelection {
                environment,
                profile: profile.to_owned(),
            });
        }
    }

    pub fn forced_profile_for(&self, environment: &Path) -> Option<&str> {
        let environment = absolute(environment);

        self.forced_profile_selections
            .iter()
            .find(|selection| selection.environment == environment)
            .map(|selection| selection.profile.as_str())
    }

    pub fn set_forced_profile_for(&mut self, environment: &Path, profile: Option<&str>) {
        let environment = absolute(environment);
        self.forced_profile_selections
            .retain(|selection| selection.environment != environment);
        if let Some(profile) = profile {
            self.forced_profile_selections.push(ForcedProfileSelection {
                environment,
                profile: profile.to_owned(),
            });
        }
    }

    fn try_load() -> Result<Option<Self>, Box<dyn Error>> {
        let path = settings_path()?;
        Self::from_file_read(fs::read_to_string(path))
    }

    fn from_file_read(read: io::Result<String>) -> Result<Option<Self>, Box<dyn Error>> {
        let source = match read {
            Ok(source) => source,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };

        Ok(Some(toml::from_str(&source)?))
    }

    #[cfg(not(test))]
    fn try_save(&self) -> Result<(), Box<dyn Error>> {
        let path = settings_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, toml::to_string_pretty(self)?)?;

        Ok(())
    }
}

pub(crate) mod keybind;
pub(crate) mod panel;
pub(crate) mod paths;
#[cfg(test)]
mod tests;
