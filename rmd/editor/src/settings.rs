use std::{
    env,
    error::Error,
    fs,
    io,
    path::{Path, PathBuf},
};

use dear_imgui_rs::{Key, Ui};
use editor::frame::FrameOptions;
use render::HighlightStyle;
use serde::{Deserialize, Serialize};

const MAX_RECENT: usize = 10;
const DEFAULT_PREFERRED_EDITOR: &str = "code --goto {file}:{line}:{column}";

pub(crate) const BINDABLE_KEYS: &[Key] = &[
    Key::Tab,
    Key::LeftArrow,
    Key::RightArrow,
    Key::UpArrow,
    Key::DownArrow,
    Key::PageUp,
    Key::PageDown,
    Key::Home,
    Key::End,
    Key::Insert,
    Key::Delete,
    Key::Backspace,
    Key::Space,
    Key::Enter,
    Key::Key0,
    Key::Key1,
    Key::Key2,
    Key::Key3,
    Key::Key4,
    Key::Key5,
    Key::Key6,
    Key::Key7,
    Key::Key8,
    Key::Key9,
    Key::A,
    Key::B,
    Key::C,
    Key::D,
    Key::E,
    Key::F,
    Key::G,
    Key::H,
    Key::I,
    Key::J,
    Key::K,
    Key::L,
    Key::M,
    Key::N,
    Key::O,
    Key::P,
    Key::Q,
    Key::S,
    Key::T,
    Key::U,
    Key::V,
    Key::W,
    Key::X,
    Key::Y,
    Key::Z,
    Key::F1,
    Key::F2,
    Key::F3,
    Key::F4,
    Key::F5,
    Key::F6,
    Key::F7,
    Key::F8,
    Key::F9,
    Key::F10,
    Key::F11,
    Key::F12,
    Key::F13,
    Key::F14,
    Key::F15,
    Key::F16,
    Key::F17,
    Key::F18,
    Key::F19,
    Key::F20,
    Key::F21,
    Key::F22,
    Key::F23,
    Key::F24,
    Key::Apostrophe,
    Key::Comma,
    Key::Minus,
    Key::Period,
    Key::Slash,
    Key::Semicolon,
    Key::Equal,
    Key::LeftBracket,
    Key::Backslash,
    Key::RightBracket,
    Key::GraveAccent,
    Key::PrintScreen,
    Key::Pause,
    Key::Keypad0,
    Key::Keypad1,
    Key::Keypad2,
    Key::Keypad3,
    Key::Keypad4,
    Key::Keypad5,
    Key::Keypad6,
    Key::Keypad7,
    Key::Keypad8,
    Key::Keypad9,
    Key::KeypadDecimal,
    Key::KeypadDivide,
    Key::KeypadMultiply,
    Key::KeypadSubtract,
    Key::KeypadAdd,
    Key::KeypadEnter,
    Key::KeypadEqual,
    Key::Oem102,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct KeyBinding {
    key: Key,
    #[serde(default, skip_serializing_if = "is_false")]
    ctrl: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    shift: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    alt: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    super_key: bool,
}

impl KeyBinding {
    pub const fn new(key: Key) -> Self {
        Self {
            key,
            ctrl: false,
            shift: false,
            alt: false,
            super_key: false,
        }
    }

    pub const fn with_ctrl(key: Key) -> Self {
        Self {
            key,
            ctrl: true,
            shift: false,
            alt: false,
            super_key: false,
        }
    }

    pub const fn with_shift(key: Key) -> Self {
        Self {
            key,
            ctrl: false,
            shift: true,
            alt: false,
            super_key: false,
        }
    }

    pub fn from_input(ui: &Ui, key: Key) -> Self {
        let io = ui.io();

        Self {
            key,
            ctrl: io.key_ctrl(),
            shift: io.key_shift(),
            alt: io.key_alt(),
            super_key: io.key_super(),
        }
    }

    pub fn is_pressed(self, ui: &Ui) -> bool {
        ui.is_key_pressed_with_repeat(self.key, false) && self.modifiers_match(ui)
    }

    pub fn is_down(self, ui: &Ui) -> bool { ui.is_key_down(self.key) && self.modifiers_match(ui) }

    pub fn is_released(self, ui: &Ui) -> bool { ui.is_key_released(self.key) && self.modifiers_match(ui) }

    fn modifiers_match(self, ui: &Ui) -> bool {
        let io = ui.io();

        self.ctrl == io.key_ctrl()
            && self.shift == io.key_shift()
            && self.alt == io.key_alt()
            && self.super_key == io.key_super()
    }

    pub fn label(self, ui: &Ui) -> String {
        let mut label = String::new();
        for (enabled, name) in [
            (self.ctrl, "Ctrl"),
            (self.shift, "Shift"),
            (self.alt, "Alt"),
            (self.super_key, "Super"),
        ] {
            if enabled {
                if !label.is_empty() {
                    label.push('+');
                }
                label.push_str(name);
            }
        }
        if !label.is_empty() {
            label.push('+');
        }
        label.push_str(ui.get_key_name(self.key));

        label
    }
}

const fn is_false(value: &bool) -> bool { !*value }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeybindAction {
    Undo,
    Redo,
    ShowAreas,
    ShowAreaOutlines,
    LevelUp,
    LevelDown,
    Refit,
    PlaceTool,
    SelectTool,
    BlockSelectTool,
    DeleteTool,
    FillTool,
    Rotate,
    Copy,
    Paste,
}

impl KeybindAction {
    pub const ALL: [Self; 15] = [
        Self::Undo,
        Self::Redo,
        Self::ShowAreas,
        Self::ShowAreaOutlines,
        Self::LevelUp,
        Self::LevelDown,
        Self::Refit,
        Self::PlaceTool,
        Self::SelectTool,
        Self::BlockSelectTool,
        Self::DeleteTool,
        Self::FillTool,
        Self::Rotate,
        Self::Copy,
        Self::Paste,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Undo => "Undo",
            Self::Redo => "Redo",
            Self::ShowAreas => "Show areas",
            Self::ShowAreaOutlines => "Show area outlines",
            Self::LevelUp => "Z up",
            Self::LevelDown => "Z down",
            Self::Refit => "Refit",
            Self::PlaceTool => "Place tool",
            Self::SelectTool => "Select tool",
            Self::BlockSelectTool => "Block select tool",
            Self::DeleteTool => "Delete tool",
            Self::FillTool => "Fill tool",
            Self::Rotate => "Rotate",
            Self::Copy => "Copy block",
            Self::Paste => "Paste block",
        }
    }

    pub const fn id(self) -> &'static str {
        match self {
            Self::Undo => "undo",
            Self::Redo => "redo",
            Self::ShowAreas => "show-areas",
            Self::ShowAreaOutlines => "show-area-outlines",
            Self::LevelUp => "level-up",
            Self::LevelDown => "level-down",
            Self::Refit => "refit",
            Self::PlaceTool => "place-tool",
            Self::SelectTool => "select-tool",
            Self::BlockSelectTool => "block-select-tool",
            Self::DeleteTool => "delete-tool",
            Self::FillTool => "fill-tool",
            Self::Rotate => "rotate",
            Self::Copy => "copy",
            Self::Paste => "paste",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct KeyBindings {
    undo: KeyBinding,
    redo: KeyBinding,
    show_areas: KeyBinding,
    show_area_outlines: KeyBinding,
    level_up: KeyBinding,
    level_down: KeyBinding,
    refit: KeyBinding,
    place_tool: KeyBinding,
    select_tool: KeyBinding,
    block_select_tool: KeyBinding,
    delete_tool: KeyBinding,
    fill_tool: KeyBinding,
    rotate: KeyBinding,
    copy: KeyBinding,
    paste: KeyBinding,
}

impl Default for KeyBindings {
    fn default() -> Self {
        Self {
            undo: KeyBinding::with_ctrl(Key::Z),
            redo: KeyBinding::with_ctrl(Key::Y),
            show_areas: KeyBinding::new(Key::A),
            show_area_outlines: KeyBinding::new(Key::O),
            level_up: KeyBinding::new(Key::PageUp),
            level_down: KeyBinding::new(Key::PageDown),
            refit: KeyBinding::new(Key::Home),
            place_tool: KeyBinding::new(Key::W),
            select_tool: KeyBinding::new(Key::S),
            block_select_tool: KeyBinding::with_shift(Key::S),
            delete_tool: KeyBinding::new(Key::X),
            fill_tool: KeyBinding::new(Key::Q),
            rotate: KeyBinding::new(Key::R),
            copy: KeyBinding::with_ctrl(Key::C),
            paste: KeyBinding::with_ctrl(Key::V),
        }
    }
}

impl KeyBindings {
    pub const fn get(self, action: KeybindAction) -> KeyBinding {
        match action {
            KeybindAction::Undo => self.undo,
            KeybindAction::Redo => self.redo,
            KeybindAction::ShowAreas => self.show_areas,
            KeybindAction::ShowAreaOutlines => self.show_area_outlines,
            KeybindAction::LevelUp => self.level_up,
            KeybindAction::LevelDown => self.level_down,
            KeybindAction::Refit => self.refit,
            KeybindAction::PlaceTool => self.place_tool,
            KeybindAction::SelectTool => self.select_tool,
            KeybindAction::BlockSelectTool => self.block_select_tool,
            KeybindAction::DeleteTool => self.delete_tool,
            KeybindAction::FillTool => self.fill_tool,
            KeybindAction::Rotate => self.rotate,
            KeybindAction::Copy => self.copy,
            KeybindAction::Paste => self.paste,
        }
    }

    pub fn rebind(&mut self, action: KeybindAction, binding: KeyBinding) {
        let previous = self.get(action);
        if previous == binding {
            return;
        }
        if let Some(displaced) = KeybindAction::ALL
            .into_iter()
            .find(|candidate| *candidate != action && self.get(*candidate) == binding)
        {
            self.set(displaced, previous);
        }
        self.set(action, binding);
    }

    pub fn is_any_pressed(self, ui: &Ui) -> bool {
        KeybindAction::ALL
            .into_iter()
            .any(|action| self.get(action).is_pressed(ui))
    }

    fn set(&mut self, action: KeybindAction, binding: KeyBinding) {
        match action {
            KeybindAction::Undo => self.undo = binding,
            KeybindAction::Redo => self.redo = binding,
            KeybindAction::ShowAreas => self.show_areas = binding,
            KeybindAction::ShowAreaOutlines => self.show_area_outlines = binding,
            KeybindAction::LevelUp => self.level_up = binding,
            KeybindAction::LevelDown => self.level_down = binding,
            KeybindAction::Refit => self.refit = binding,
            KeybindAction::PlaceTool => self.place_tool = binding,
            KeybindAction::SelectTool => self.select_tool = binding,
            KeybindAction::BlockSelectTool => self.block_select_tool = binding,
            KeybindAction::DeleteTool => self.delete_tool = binding,
            KeybindAction::FillTool => self.fill_tool = binding,
            KeybindAction::Rotate => self.rotate = binding,
            KeybindAction::Copy => self.copy = binding,
            KeybindAction::Paste => self.paste = binding,
        }
    }
}

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
#[serde(default)]
pub(crate) struct Settings {
    pub maximized: bool,
    pub preferred_editor: String,
    pub show_areas: bool,
    pub show_area_outlines: bool,
    pub tile_place_flash: bool,
    pub selection_guide_line: bool,
    pub selection_highlight: SelectionHighlight,
    pub object_tree_search: ObjectTreeSearchOptions,
    pub object_tree_filter: ObjectTreeFilterOptions,
    pub keybindings: KeyBindings,
    pub recent_codebases: Vec<PathBuf>,
    pub recent: Vec<RecentMap>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            maximized: false,
            preferred_editor: String::from(DEFAULT_PREFERRED_EDITOR),
            show_areas: false,
            show_area_outlines: true,
            tile_place_flash: true,
            selection_guide_line: true,
            selection_highlight: SelectionHighlight::Outline,
            object_tree_search: ObjectTreeSearchOptions::default(),
            object_tree_filter: ObjectTreeFilterOptions::default(),
            keybindings: KeyBindings::default(),
            recent_codebases: Vec::new(),
            recent: Vec::new(),
        }
    }
}

impl Settings {
    pub fn load() -> Self {
        match Self::try_load() {
            Ok(mut settings) => {
                settings.normalize();

                settings
            },
            Err(error) => {
                log::error!("loading settings: {error}");

                Self::default()
            },
        }
    }

    pub fn save(&self) {
        if let Err(error) = self.try_save() {
            log::error!("saving settings: {error}");
        }
    }

    pub fn apply_to(&self, options: &mut FrameOptions) {
        options.show_areas = self.show_areas;
        options.show_area_outlines = self.show_area_outlines;
    }

    pub fn capture_from(&mut self, options: &FrameOptions) {
        self.show_areas = options.show_areas;
        self.show_area_outlines = options.show_area_outlines;
    }

    fn normalize(&mut self) {
        if !self.object_tree_search.type_paths && !self.object_tree_search.names {
            self.object_tree_search = ObjectTreeSearchOptions::default();
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

    fn try_load() -> Result<Self, Box<dyn Error>> {
        let path = settings_path()?;
        let source = match fs::read_to_string(&path) {
            Ok(source) => source,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(error.into()),
        };

        Ok(toml::from_str(&source)?)
    }

    fn try_save(&self) -> Result<(), Box<dyn Error>> {
        let path = settings_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, toml::to_string_pretty(self)?)?;

        Ok(())
    }
}

fn absolute(path: &Path) -> PathBuf { std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf()) }

fn settings_path() -> io::Result<PathBuf> {
    #[cfg(target_os = "windows")]
    let variable = "LOCALAPPDATA";
    #[cfg(not(target_os = "windows"))]
    let variable = "HOME";

    let root = env::var_os(variable)
        .filter(|root| !root.is_empty())
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("{variable} is not set")))?;

    Ok(settings_path_from(Path::new(&root), cfg!(target_os = "windows")))
}

pub(crate) fn imgui_ini_path() -> io::Result<PathBuf> { Ok(settings_path()?.with_file_name("imgui.ini")) }

pub(crate) fn log_path() -> io::Result<PathBuf> { Ok(settings_path()?.with_file_name("latest.log")) }

fn settings_path_from(root: &Path, windows: bool) -> PathBuf {
    if windows {
        root.join("rmd/settings.toml")
    } else {
        root.join(".config/rmd/settings.toml")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_preserve_existing_editor_behavior() {
        assert_eq!(
            Settings::default(),
            Settings {
                maximized: false,
                preferred_editor: String::from(DEFAULT_PREFERRED_EDITOR),
                show_areas: false,
                show_area_outlines: true,
                tile_place_flash: true,
                selection_guide_line: true,
                selection_highlight: SelectionHighlight::Outline,
                object_tree_search: ObjectTreeSearchOptions::default(),
                object_tree_filter: ObjectTreeFilterOptions::default(),
                keybindings: KeyBindings::default(),
                recent_codebases: Vec::new(),
                recent: Vec::new(),
            }
        );
    }

    #[test]
    fn default_keybindings_match_the_replaced_shortcuts() {
        let bindings = KeyBindings::default();
        let expected = [
            (KeybindAction::ShowAreas, Key::A),
            (KeybindAction::ShowAreaOutlines, Key::O),
            (KeybindAction::LevelUp, Key::PageUp),
            (KeybindAction::LevelDown, Key::PageDown),
            (KeybindAction::Refit, Key::Home),
            (KeybindAction::PlaceTool, Key::W),
            (KeybindAction::SelectTool, Key::S),
            (KeybindAction::DeleteTool, Key::X),
            (KeybindAction::FillTool, Key::Q),
            (KeybindAction::Rotate, Key::R),
        ];

        for (action, key) in expected {
            assert_eq!(bindings.get(action), KeyBinding::new(key));
        }
        assert_eq!(
            bindings.get(KeybindAction::BlockSelectTool),
            KeyBinding::with_shift(Key::S)
        );
        assert_eq!(bindings.get(KeybindAction::Undo), KeyBinding::with_ctrl(Key::Z));
        assert_eq!(bindings.get(KeybindAction::Redo), KeyBinding::with_ctrl(Key::Y));
    }

    #[test]
    fn missing_fields_use_their_defaults() {
        let settings: Settings = toml::from_str("show_areas = true\n").unwrap();

        assert_eq!(
            settings,
            Settings {
                show_areas: true,
                ..Settings::default()
            }
        );
    }

    #[test]
    fn older_keybinding_tables_receive_undo_and_redo_defaults() {
        let mut stored = toml::Value::try_from(KeyBindings::default()).unwrap();
        let table = stored.as_table_mut().unwrap();
        table.remove("undo");
        table.remove("redo");

        let bindings: KeyBindings = stored.try_into().unwrap();

        assert_eq!(bindings.get(KeybindAction::Undo), KeyBinding::with_ctrl(Key::Z));
        assert_eq!(bindings.get(KeybindAction::Redo), KeyBinding::with_ctrl(Key::Y));
    }

    #[test]
    fn settings_written_before_the_option_existed_keep_the_marching_outline() {
        let settings: Settings = toml::from_str("tile_place_flash = false\n").unwrap();

        assert_eq!(settings.selection_highlight, SelectionHighlight::Outline);
        assert_eq!(settings.preferred_editor, DEFAULT_PREFERRED_EDITOR);
        assert_eq!(settings.selection_highlight.style(), HighlightStyle::Outline);
        assert_eq!(SelectionHighlight::Tint.style(), HighlightStyle::Tint);
        assert_eq!(settings.object_tree_search, ObjectTreeSearchOptions::default());
        assert_eq!(settings.object_tree_filter, ObjectTreeFilterOptions::default());
    }

    #[test]
    fn object_tree_search_settings_keep_one_mode_enabled() {
        let mut settings: Settings =
            toml::from_str("[object_tree_search]\ntype_paths = false\nnames = false\n").unwrap();

        settings.normalize();

        assert_eq!(settings.object_tree_search, ObjectTreeSearchOptions::default());
    }

    #[test]
    fn settings_round_trip_through_toml() {
        let mut settings = Settings {
            maximized: true,
            preferred_editor: String::from("zed {file}:{line}:{column}"),
            show_areas: true,
            show_area_outlines: false,
            tile_place_flash: false,
            selection_guide_line: false,
            selection_highlight: SelectionHighlight::Tint,
            object_tree_search: ObjectTreeSearchOptions {
                type_paths: false,
                names: true,
            },
            object_tree_filter: ObjectTreeFilterOptions {
                atom: true,
                movable: false,
                obj: true,
                turf: false,
                custom_enabled: true,
                custom_type_path: String::from("/atom/movable/lighting"),
            },
            keybindings: KeyBindings::default(),
            recent_codebases: Vec::new(),
            recent: Vec::new(),
        };
        settings.keybindings.rebind(
            KeybindAction::ShowAreas,
            KeyBinding {
                key: Key::G,
                ctrl: true,
                shift: false,
                alt: false,
                super_key: false,
            },
        );
        let encoded = toml::to_string_pretty(&settings).unwrap();

        assert!(encoded.contains("[keybindings.show_areas]"));
        assert!(encoded.contains("maximized = true"));
        assert!(encoded.contains("preferred_editor = \"zed {file}:{line}:{column}\""));
        assert!(encoded.contains("selection_highlight = \"tint\""));
        assert!(encoded.contains("[object_tree_search]"));
        assert!(encoded.contains("type_paths = false"));
        assert!(encoded.contains("names = true"));
        assert!(encoded.contains("[object_tree_filter]"));
        assert!(encoded.contains("atom = true"));
        assert!(encoded.contains("obj = true"));
        assert!(encoded.contains("custom_enabled = true"));
        assert!(encoded.contains("custom_type_path = \"/atom/movable/lighting\""));
        assert!(encoded.contains("key = \"G\""));
        assert!(encoded.contains("ctrl = true"));
        assert_eq!(toml::from_str::<Settings>(&encoded).unwrap(), settings);
    }

    #[test]
    fn frame_options_apply_and_capture_without_touching_other_settings() {
        let mut settings = Settings {
            maximized: true,
            preferred_editor: String::from("editor {file}"),
            show_areas: true,
            show_area_outlines: false,
            tile_place_flash: false,
            selection_guide_line: false,
            selection_highlight: SelectionHighlight::Tint,
            object_tree_search: ObjectTreeSearchOptions {
                type_paths: false,
                names: true,
            },
            object_tree_filter: ObjectTreeFilterOptions::default(),
            keybindings: KeyBindings::default(),
            recent_codebases: Vec::new(),
            recent: Vec::new(),
        };
        let mut options = FrameOptions::default();

        settings.apply_to(&mut options);
        assert!(options.show_areas);
        assert!(!options.show_area_outlines);

        options.show_areas = false;
        options.show_area_outlines = true;
        settings.capture_from(&options);
        assert_eq!(
            settings,
            Settings {
                maximized: true,
                preferred_editor: String::from("editor {file}"),
                tile_place_flash: false,
                selection_guide_line: false,
                selection_highlight: SelectionHighlight::Tint,
                object_tree_search: ObjectTreeSearchOptions {
                    type_paths: false,
                    names: true,
                },
                ..Settings::default()
            }
        );
    }

    #[test]
    fn malformed_toml_is_rejected() {
        assert!(toml::from_str::<Settings>("show_areas = maybe").is_err());
    }

    #[test]
    fn rebinding_to_an_assigned_chord_swaps_the_actions() {
        let mut bindings = KeyBindings::default();

        bindings.rebind(KeybindAction::ShowAreas, KeyBinding::new(Key::W));

        assert_eq!(bindings.get(KeybindAction::ShowAreas), KeyBinding::new(Key::W));
        assert_eq!(bindings.get(KeybindAction::PlaceTool), KeyBinding::new(Key::A));
    }

    #[test]
    fn recent_maps_move_to_the_front_without_duplicating() {
        let mut settings = Settings::default();
        let env = Path::new("station.dme");

        settings.push_recent(Some(env), Path::new("a.dmm"));
        settings.push_recent(Some(env), Path::new("b.dmm"));
        settings.push_recent(Some(env), Path::new("a.dmm"));

        assert_eq!(
            settings.recent,
            vec![
                RecentMap {
                    map: absolute(Path::new("a.dmm")),
                    environment: Some(absolute(env)),
                },
                RecentMap {
                    map: absolute(Path::new("b.dmm")),
                    environment: Some(absolute(env)),
                },
            ]
        );
    }

    #[test]
    fn recent_entries_are_stored_absolute_so_they_survive_a_different_working_directory() {
        let mut settings = Settings::default();

        settings.push_recent(Some(Path::new("env/test.dme")), Path::new("env/test.dmm"));

        assert!(settings.recent[0].map.is_absolute());
        assert!(
            settings.recent[0]
                .environment
                .as_ref()
                .is_some_and(|path| path.is_absolute())
        );
    }

    #[test]
    fn the_same_map_named_two_ways_is_one_recent_entry() {
        let mut settings = Settings::default();
        let relative = Path::new("maps/station.dmm");

        settings.push_recent(None, relative);
        settings.push_recent(None, &absolute(relative));

        assert_eq!(settings.recent.len(), 1);
    }

    #[test]
    fn recent_maps_remember_a_map_opened_without_an_environment() {
        let mut settings = Settings::default();

        settings.push_recent(None, Path::new("lone.dmm"));

        assert_eq!(settings.recent[0].environment, None);
    }

    #[test]
    fn the_recent_list_is_capped() {
        let mut settings = Settings::default();

        for index in 0..MAX_RECENT + 5 {
            settings.push_recent(None, Path::new(&format!("map{index}.dmm")));
        }

        assert_eq!(settings.recent.len(), MAX_RECENT);
        assert_eq!(settings.recent[0].map, absolute(Path::new("map14.dmm")));
    }

    #[test]
    fn recent_maps_round_trip_through_toml() {
        let mut settings = Settings::default();
        settings.push_recent(Some(Path::new("station.dme")), Path::new("station.dmm"));
        let encoded = toml::to_string_pretty(&settings).unwrap();

        assert_eq!(toml::from_str::<Settings>(&encoded).unwrap(), settings);
    }

    #[test]
    fn recent_codebases_move_to_the_front_without_duplicating() {
        let mut settings = Settings::default();

        settings.push_codebase(Path::new("a.dme"));
        settings.push_codebase(Path::new("b.dme"));
        settings.push_codebase(Path::new("a.dme"));

        assert_eq!(
            settings.recent_codebases,
            vec![absolute(Path::new("a.dme")), absolute(Path::new("b.dme"))]
        );
    }

    #[test]
    fn recent_maps_are_filtered_to_one_codebase() {
        let mut settings = Settings::default();
        let station = Path::new("station.dme");

        settings.push_recent(Some(station), Path::new("one.dmm"));
        settings.push_recent(Some(Path::new("other.dme")), Path::new("two.dmm"));
        settings.push_recent(Some(station), Path::new("three.dmm"));

        let maps = settings
            .recent_maps_for(station)
            .map(|recent| &recent.map)
            .collect::<Vec<_>>();

        assert_eq!(
            maps,
            [&absolute(Path::new("three.dmm")), &absolute(Path::new("one.dmm"))]
        );
    }

    #[test]
    fn a_map_opened_without_a_codebase_belongs_to_no_codebase() {
        let mut settings = Settings::default();

        settings.push_recent(None, Path::new("lone.dmm"));

        assert_eq!(settings.recent_maps_for(Path::new("station.dme")).count(), 0);
    }

    #[test]
    fn platform_paths_use_the_requested_application_directory() {
        let root = Path::new("profile");

        assert_eq!(settings_path_from(root, true), root.join("rmd/settings.toml"));
        assert_eq!(settings_path_from(root, false), root.join(".config/rmd/settings.toml"));
        assert_eq!(
            settings_path_from(root, true).with_file_name("imgui.ini"),
            root.join("rmd/imgui.ini")
        );
        assert_eq!(
            settings_path_from(root, false).with_file_name("imgui.ini"),
            root.join(".config/rmd/imgui.ini")
        );
        assert_eq!(
            settings_path_from(root, true).with_file_name("latest.log"),
            root.join("rmd/latest.log")
        );
        assert_eq!(
            settings_path_from(root, false).with_file_name("latest.log"),
            root.join(".config/rmd/latest.log")
        );
    }
}
