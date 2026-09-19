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
    Key::R,
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

    const fn with_primary(key: Key) -> Self {
        if cfg!(target_os = "macos") {
            Self {
                key,
                ctrl: false,
                shift: false,
                alt: false,
                super_key: true,
            }
        } else {
            Self::with_ctrl(key)
        }
    }

    const fn with_primary_shift(key: Key) -> Self {
        Self {
            key,
            ctrl: !cfg!(target_os = "macos"),
            shift: true,
            alt: false,
            super_key: cfg!(target_os = "macos"),
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

    pub fn is_pressed_repeating(self, ui: &Ui) -> bool {
        ui.is_key_pressed_with_repeat(self.key, true) && self.modifiers_match(ui)
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
            (self.super_key, if cfg!(target_os = "macos") { "Cmd" } else { "Super" }),
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
pub(crate) enum KeybindPreset {
    Default,
    StrongDmm,
}

impl KeybindPreset {
    pub fn bindings(self) -> KeyBindings {
        match self {
            Self::Default => KeyBindings::default(),
            Self::StrongDmm => KeyBindings::strong_dmm(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeybindAction {
    Save,
    Undo,
    Redo,
    ShowAreas,
    ShowAreaOutlines,
    ShowLighting,
    LevelUp,
    LevelDown,
    Refit,
    PlaceTool,
    SelectTool,
    NodeTool,
    BlockSelectTool,
    DeleteTool,
    FillTool,
    Rotate,
    Copy,
    Paste,
    Recent1,
    Recent2,
    Recent3,
    Recent4,
    Recent5,
    Recent6,
    Recent7,
    Recent8,
    Recent9,
    Recent0,
    ShowTileGrid,
    ShowPixelGrid,
}

impl KeybindAction {
    pub const ALL: [Self; 30] = [
        Self::Save,
        Self::Undo,
        Self::Redo,
        Self::ShowAreas,
        Self::ShowAreaOutlines,
        Self::ShowLighting,
        Self::LevelUp,
        Self::LevelDown,
        Self::Refit,
        Self::PlaceTool,
        Self::SelectTool,
        Self::NodeTool,
        Self::BlockSelectTool,
        Self::DeleteTool,
        Self::FillTool,
        Self::Rotate,
        Self::Copy,
        Self::Paste,
        Self::Recent1,
        Self::Recent2,
        Self::Recent3,
        Self::Recent4,
        Self::Recent5,
        Self::Recent6,
        Self::Recent7,
        Self::Recent8,
        Self::Recent9,
        Self::Recent0,
        Self::ShowTileGrid,
        Self::ShowPixelGrid,
    ];
    pub const RECENT: [Self; 10] = [
        Self::Recent1,
        Self::Recent2,
        Self::Recent3,
        Self::Recent4,
        Self::Recent5,
        Self::Recent6,
        Self::Recent7,
        Self::Recent8,
        Self::Recent9,
        Self::Recent0,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Save => "Save",
            Self::Undo => "Undo",
            Self::Redo => "Redo",
            Self::ShowAreas => "Show areas",
            Self::ShowAreaOutlines => "Show area outlines",
            Self::ShowLighting => "Show lighting",
            Self::LevelUp => "Z up",
            Self::LevelDown => "Z down",
            Self::Refit => "Refit",
            Self::PlaceTool => "Place tool",
            Self::SelectTool => "Select tool",
            Self::NodeTool => "Node tool",
            Self::BlockSelectTool => "Block select tool",
            Self::DeleteTool => "Delete tool",
            Self::FillTool => "Fill tool",
            Self::Rotate => "Rotate",
            Self::Copy => "Copy block",
            Self::Paste => "Paste block",
            Self::Recent1 => "Recent 1",
            Self::Recent2 => "Recent 2",
            Self::Recent3 => "Recent 3",
            Self::Recent4 => "Recent 4",
            Self::Recent5 => "Recent 5",
            Self::Recent6 => "Recent 6",
            Self::Recent7 => "Recent 7",
            Self::Recent8 => "Recent 8",
            Self::Recent9 => "Recent 9",
            Self::Recent0 => "Recent 0",
            Self::ShowTileGrid => "Show tile grid",
            Self::ShowPixelGrid => "Show pixel grid",
        }
    }

    pub const fn id(self) -> &'static str {
        match self {
            Self::Save => "save",
            Self::Undo => "undo",
            Self::Redo => "redo",
            Self::ShowAreas => "show-areas",
            Self::ShowAreaOutlines => "show-area-outlines",
            Self::ShowLighting => "show-lighting",
            Self::LevelUp => "level-up",
            Self::LevelDown => "level-down",
            Self::Refit => "refit",
            Self::PlaceTool => "place-tool",
            Self::SelectTool => "select-tool",
            Self::NodeTool => "node-tool",
            Self::BlockSelectTool => "block-select-tool",
            Self::DeleteTool => "delete-tool",
            Self::FillTool => "fill-tool",
            Self::Rotate => "rotate",
            Self::Copy => "copy",
            Self::Paste => "paste",
            Self::Recent1 => "recent-1",
            Self::Recent2 => "recent-2",
            Self::Recent3 => "recent-3",
            Self::Recent4 => "recent-4",
            Self::Recent5 => "recent-5",
            Self::Recent6 => "recent-6",
            Self::Recent7 => "recent-7",
            Self::Recent8 => "recent-8",
            Self::Recent9 => "recent-9",
            Self::Recent0 => "recent-0",
            Self::ShowTileGrid => "show-tile-grid",
            Self::ShowPixelGrid => "show-pixel-grid",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct KeyBindings {
    save: KeyBinding,
    undo: KeyBinding,
    redo: KeyBinding,
    show_areas: KeyBinding,
    show_area_outlines: KeyBinding,
    show_lighting: KeyBinding,
    level_up: KeyBinding,
    level_down: KeyBinding,
    refit: KeyBinding,
    place_tool: KeyBinding,
    select_tool: KeyBinding,
    node_tool: KeyBinding,
    block_select_tool: KeyBinding,
    delete_tool: KeyBinding,
    fill_tool: KeyBinding,
    rotate: KeyBinding,
    copy: KeyBinding,
    paste: KeyBinding,
    recent_1: KeyBinding,
    recent_2: KeyBinding,
    recent_3: KeyBinding,
    recent_4: KeyBinding,
    recent_5: KeyBinding,
    recent_6: KeyBinding,
    recent_7: KeyBinding,
    recent_8: KeyBinding,
    recent_9: KeyBinding,
    recent_0: KeyBinding,
    show_tile_grid: KeyBinding,
    show_pixel_grid: KeyBinding,
}

impl Default for KeyBindings {
    fn default() -> Self {
        Self {
            save: KeyBinding::with_ctrl(Key::S),
            undo: KeyBinding::with_ctrl(Key::Z),
            redo: KeyBinding::with_ctrl(Key::Y),
            show_areas: KeyBinding::new(Key::A),
            show_area_outlines: KeyBinding::new(Key::O),
            show_lighting: KeyBinding::new(Key::L),
            level_up: KeyBinding::new(Key::PageUp),
            level_down: KeyBinding::new(Key::PageDown),
            refit: KeyBinding::new(Key::Home),
            place_tool: KeyBinding::new(Key::W),
            select_tool: KeyBinding::new(Key::S),
            node_tool: KeyBinding::new(Key::N),
            block_select_tool: KeyBinding::with_shift(Key::S),
            delete_tool: KeyBinding::new(Key::X),
            fill_tool: KeyBinding::new(Key::Q),
            rotate: KeyBinding::new(Key::R),
            copy: KeyBinding::with_ctrl(Key::C),
            paste: KeyBinding::with_ctrl(Key::V),
            recent_1: KeyBinding::new(Key::Key1),
            recent_2: KeyBinding::new(Key::Key2),
            recent_3: KeyBinding::new(Key::Key3),
            recent_4: KeyBinding::new(Key::Key4),
            recent_5: KeyBinding::new(Key::Key5),
            recent_6: KeyBinding::new(Key::Key6),
            recent_7: KeyBinding::new(Key::Key7),
            recent_8: KeyBinding::new(Key::Key8),
            recent_9: KeyBinding::new(Key::Key9),
            recent_0: KeyBinding::new(Key::Key0),
            show_tile_grid: KeyBinding::new(Key::G),
            show_pixel_grid: KeyBinding::with_shift(Key::G),
        }
    }
}

impl KeyBindings {
    fn strong_dmm() -> Self {
        Self {
            save: KeyBinding::with_primary(Key::S),
            undo: KeyBinding::with_primary(Key::Z),
            redo: KeyBinding::with_primary_shift(Key::Z),
            show_areas: KeyBinding::with_primary(Key::Key1),
            show_area_outlines: KeyBinding::with_shift(Key::O),
            show_lighting: KeyBinding::new(Key::L),
            level_up: KeyBinding::with_primary(Key::UpArrow),
            level_down: KeyBinding::with_primary(Key::DownArrow),
            refit: KeyBinding::new(Key::Home),
            place_tool: KeyBinding::new(Key::Key1),
            select_tool: KeyBinding::new(Key::S),
            node_tool: KeyBinding::new(Key::N),
            block_select_tool: KeyBinding::new(Key::Key3),
            delete_tool: KeyBinding::new(Key::D),
            fill_tool: KeyBinding::new(Key::Key2),
            rotate: KeyBinding::with_shift(Key::R),
            copy: KeyBinding::with_primary(Key::C),
            paste: KeyBinding::with_primary(Key::V),
            recent_1: KeyBinding::new(Key::Q),
            recent_2: KeyBinding::new(Key::W),
            recent_3: KeyBinding::new(Key::E),
            recent_4: KeyBinding::new(Key::R),
            recent_5: KeyBinding::new(Key::T),
            recent_6: KeyBinding::new(Key::Y),
            recent_7: KeyBinding::new(Key::U),
            recent_8: KeyBinding::new(Key::I),
            recent_9: KeyBinding::new(Key::O),
            recent_0: KeyBinding::new(Key::P),
            show_tile_grid: KeyBinding::new(Key::G),
            show_pixel_grid: KeyBinding::with_shift(Key::G),
        }
    }

    pub const fn get(self, action: KeybindAction) -> KeyBinding {
        match action {
            KeybindAction::Save => self.save,
            KeybindAction::Undo => self.undo,
            KeybindAction::Redo => self.redo,
            KeybindAction::ShowAreas => self.show_areas,
            KeybindAction::ShowAreaOutlines => self.show_area_outlines,
            KeybindAction::ShowLighting => self.show_lighting,
            KeybindAction::LevelUp => self.level_up,
            KeybindAction::LevelDown => self.level_down,
            KeybindAction::Refit => self.refit,
            KeybindAction::PlaceTool => self.place_tool,
            KeybindAction::SelectTool => self.select_tool,
            KeybindAction::NodeTool => self.node_tool,
            KeybindAction::BlockSelectTool => self.block_select_tool,
            KeybindAction::DeleteTool => self.delete_tool,
            KeybindAction::FillTool => self.fill_tool,
            KeybindAction::Rotate => self.rotate,
            KeybindAction::Copy => self.copy,
            KeybindAction::Paste => self.paste,
            KeybindAction::Recent1 => self.recent_1,
            KeybindAction::Recent2 => self.recent_2,
            KeybindAction::Recent3 => self.recent_3,
            KeybindAction::Recent4 => self.recent_4,
            KeybindAction::Recent5 => self.recent_5,
            KeybindAction::Recent6 => self.recent_6,
            KeybindAction::Recent7 => self.recent_7,
            KeybindAction::Recent8 => self.recent_8,
            KeybindAction::Recent9 => self.recent_9,
            KeybindAction::Recent0 => self.recent_0,
            KeybindAction::ShowTileGrid => self.show_tile_grid,
            KeybindAction::ShowPixelGrid => self.show_pixel_grid,
        }
    }

    pub fn recent(self, index: usize) -> Option<KeyBinding> {
        KeybindAction::RECENT.get(index).copied().map(|action| self.get(action))
    }

    pub fn pressed_recent(self, ui: &Ui) -> Option<usize> {
        KeybindAction::RECENT
            .into_iter()
            .position(|action| self.get(action).is_pressed(ui))
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
            KeybindAction::Save => self.save = binding,
            KeybindAction::Undo => self.undo = binding,
            KeybindAction::Redo => self.redo = binding,
            KeybindAction::ShowAreas => self.show_areas = binding,
            KeybindAction::ShowAreaOutlines => self.show_area_outlines = binding,
            KeybindAction::ShowLighting => self.show_lighting = binding,
            KeybindAction::LevelUp => self.level_up = binding,
            KeybindAction::LevelDown => self.level_down = binding,
            KeybindAction::Refit => self.refit = binding,
            KeybindAction::PlaceTool => self.place_tool = binding,
            KeybindAction::SelectTool => self.select_tool = binding,
            KeybindAction::NodeTool => self.node_tool = binding,
            KeybindAction::BlockSelectTool => self.block_select_tool = binding,
            KeybindAction::DeleteTool => self.delete_tool = binding,
            KeybindAction::FillTool => self.fill_tool = binding,
            KeybindAction::Rotate => self.rotate = binding,
            KeybindAction::Copy => self.copy = binding,
            KeybindAction::Paste => self.paste = binding,
            KeybindAction::Recent1 => self.recent_1 = binding,
            KeybindAction::Recent2 => self.recent_2 = binding,
            KeybindAction::Recent3 => self.recent_3 = binding,
            KeybindAction::Recent4 => self.recent_4 = binding,
            KeybindAction::Recent5 => self.recent_5 = binding,
            KeybindAction::Recent6 => self.recent_6 = binding,
            KeybindAction::Recent7 => self.recent_7 = binding,
            KeybindAction::Recent8 => self.recent_8 = binding,
            KeybindAction::Recent9 => self.recent_9 = binding,
            KeybindAction::Recent0 => self.recent_0 = binding,
            KeybindAction::ShowTileGrid => self.show_tile_grid = binding,
            KeybindAction::ShowPixelGrid => self.show_pixel_grid = binding,
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
    pub show_lighting: bool,
    pub focus_windows_on_hover: bool,
    pub tile_place_flash: bool,
    pub selection_guide_line: bool,
    pub show_tile_grid: bool,
    pub tile_grid_min_pixels: u32,
    pub show_tile_grid_axis: bool,
    pub show_selected_pixel_grid: bool,
    pub selected_pixel_grid_min_pixels: u32,
    pub show_pixel_grid_axis: bool,
    pub selection_highlight: SelectionHighlight,
    pub object_tree_line_indicators: bool,
    pub object_tree_search: ObjectTreeSearchOptions,
    pub object_tree_filter: ObjectTreeFilterOptions,
    pub keybindings: KeyBindings,
    pub recent_codebases: Vec<PathBuf>,
    pub recent: Vec<RecentMap>,
    pub bake_enabled: bool,
    pub perspective_editor_wall: bool,
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
            focus_windows_on_hover: true,
            tile_place_flash: true,
            selection_guide_line: true,
            show_tile_grid: true,
            tile_grid_min_pixels: 10,
            show_tile_grid_axis: true,
            show_selected_pixel_grid: true,
            selected_pixel_grid_min_pixels: 3,
            show_pixel_grid_axis: true,
            selection_highlight: SelectionHighlight::Outline,
            object_tree_line_indicators: true,
            object_tree_search: ObjectTreeSearchOptions::default(),
            object_tree_filter: ObjectTreeFilterOptions::default(),
            keybindings: KeyBindings::default(),
            recent_codebases: Vec::new(),
            recent: Vec::new(),
            bake_enabled: true,
            perspective_editor_wall: false,
        }
    }
}

impl Settings {
    pub fn load() -> SettingsLoad { SettingsLoad::from_file_result(Self::try_load()) }

    pub fn save(&self) {
        if let Err(error) = self.try_save() {
            log::error!("saving settings: {error}");
        }
    }

    pub fn apply_to(&self, options: &mut FrameOptions) {
        options.show_areas = self.show_areas;
        options.show_area_outlines = self.show_area_outlines;
        options.show_lighting = self.show_lighting;
    }

    pub fn capture_from(&mut self, options: &FrameOptions) {
        self.show_areas = options.show_areas;
        self.show_area_outlines = options.show_area_outlines;
        self.show_lighting = options.show_lighting;
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

#[cfg(target_os = "windows")]
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
                show_lighting: true,
                focus_windows_on_hover: true,
                tile_place_flash: true,
                selection_guide_line: true,
                show_tile_grid: true,
                tile_grid_min_pixels: 10,
                show_tile_grid_axis: true,
                show_selected_pixel_grid: true,
                selected_pixel_grid_min_pixels: 3,
                show_pixel_grid_axis: true,
                selection_highlight: SelectionHighlight::Outline,
                object_tree_line_indicators: true,
                object_tree_search: ObjectTreeSearchOptions::default(),
                object_tree_filter: ObjectTreeFilterOptions::default(),
                keybindings: KeyBindings::default(),
                recent_codebases: Vec::new(),
                recent: Vec::new(),
                bake_enabled: true,
                perspective_editor_wall: false,
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
            (KeybindAction::ShowTileGrid, Key::G),
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
        assert_eq!(bindings.get(KeybindAction::Save), KeyBinding::with_ctrl(Key::S));
        assert_eq!(
            bindings.get(KeybindAction::ShowPixelGrid),
            KeyBinding::with_shift(Key::G)
        );

        let recent = [
            Key::Key1,
            Key::Key2,
            Key::Key3,
            Key::Key4,
            Key::Key5,
            Key::Key6,
            Key::Key7,
            Key::Key8,
            Key::Key9,
            Key::Key0,
        ];
        for (index, key) in recent.into_iter().enumerate() {
            assert_eq!(bindings.recent(index), Some(KeyBinding::new(key)));
        }
        assert_eq!(bindings.recent(recent.len()), None);
    }

    #[test]
    fn strong_dmm_keybindings_match_the_selected_shortcuts() {
        let bindings = KeybindPreset::StrongDmm.bindings();
        let expected = [
            (KeybindAction::Save, KeyBinding::with_primary(Key::S)),
            (KeybindAction::Undo, KeyBinding::with_primary(Key::Z)),
            (KeybindAction::Redo, KeyBinding::with_primary_shift(Key::Z)),
            (KeybindAction::ShowAreas, KeyBinding::with_primary(Key::Key1)),
            (KeybindAction::ShowAreaOutlines, KeyBinding::with_shift(Key::O)),
            (KeybindAction::LevelUp, KeyBinding::with_primary(Key::UpArrow)),
            (KeybindAction::LevelDown, KeyBinding::with_primary(Key::DownArrow)),
            (KeybindAction::Refit, KeyBinding::new(Key::Home)),
            (KeybindAction::PlaceTool, KeyBinding::new(Key::Key1)),
            (KeybindAction::SelectTool, KeyBinding::new(Key::S)),
            (KeybindAction::NodeTool, KeyBinding::new(Key::N)),
            (KeybindAction::BlockSelectTool, KeyBinding::new(Key::Key3)),
            (KeybindAction::DeleteTool, KeyBinding::new(Key::D)),
            (KeybindAction::FillTool, KeyBinding::new(Key::Key2)),
            (KeybindAction::Rotate, KeyBinding::with_shift(Key::R)),
            (KeybindAction::Copy, KeyBinding::with_primary(Key::C)),
            (KeybindAction::Paste, KeyBinding::with_primary(Key::V)),
            (KeybindAction::ShowTileGrid, KeyBinding::new(Key::G)),
            (KeybindAction::ShowPixelGrid, KeyBinding::with_shift(Key::G)),
        ];

        for (action, binding) in expected {
            assert_eq!(bindings.get(action), binding);
        }

        for (index, key) in [
            Key::Q,
            Key::W,
            Key::E,
            Key::R,
            Key::T,
            Key::Y,
            Key::U,
            Key::I,
            Key::O,
            Key::P,
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(bindings.recent(index), Some(KeyBinding::new(key)));
        }
    }

    #[test]
    fn every_preset_uses_unique_bindable_keys() {
        for preset in [KeybindPreset::Default, KeybindPreset::StrongDmm] {
            let bindings = preset.bindings();

            for (index, action) in KeybindAction::ALL.into_iter().enumerate() {
                let binding = bindings.get(action);
                assert!(
                    BINDABLE_KEYS.contains(&binding.key),
                    "{preset:?} binding for {action:?} uses non-bindable key {:?}",
                    binding.key
                );
                for candidate in KeybindAction::ALL.into_iter().skip(index + 1) {
                    assert_ne!(
                        binding,
                        bindings.get(candidate),
                        "{preset:?} assigns the same binding to {action:?} and {candidate:?}"
                    );
                }
            }
        }
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
    fn older_keybinding_tables_receive_new_action_defaults() {
        let mut stored = toml::Value::try_from(KeyBindings::default()).unwrap();
        let table = stored.as_table_mut().unwrap();
        table.remove("save");
        table.remove("undo");
        table.remove("redo");
        for field in [
            "recent_1", "recent_2", "recent_3", "recent_4", "recent_5", "recent_6", "recent_7", "recent_8", "recent_9",
            "recent_0",
        ] {
            table.remove(field);
        }

        let bindings: KeyBindings = stored.try_into().unwrap();

        assert_eq!(bindings.get(KeybindAction::Save), KeyBinding::with_ctrl(Key::S));
        assert_eq!(bindings.get(KeybindAction::Undo), KeyBinding::with_ctrl(Key::Z));
        assert_eq!(bindings.get(KeybindAction::Redo), KeyBinding::with_ctrl(Key::Y));
        assert_eq!(bindings.recent(0), Some(KeyBinding::new(Key::Key1)));
        assert_eq!(bindings.recent(9), Some(KeyBinding::new(Key::Key0)));
    }

    #[test]
    fn settings_written_before_the_option_existed_keep_the_marching_outline() {
        let settings: Settings = toml::from_str("tile_place_flash = false\n").unwrap();

        assert_eq!(settings.selection_highlight, SelectionHighlight::Outline);
        assert_eq!(settings.preferred_editor, DEFAULT_PREFERRED_EDITOR);
        assert_eq!(settings.selection_highlight.style(), HighlightStyle::Outline);
        assert_eq!(SelectionHighlight::Tint.style(), HighlightStyle::Tint);
        assert!(settings.object_tree_line_indicators);
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
            show_lighting: true,
            focus_windows_on_hover: false,
            tile_place_flash: false,
            selection_guide_line: false,
            show_tile_grid: false,
            tile_grid_min_pixels: 12,
            show_tile_grid_axis: false,
            show_selected_pixel_grid: false,
            selected_pixel_grid_min_pixels: 5,
            show_pixel_grid_axis: false,
            selection_highlight: SelectionHighlight::Tint,
            object_tree_line_indicators: false,
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
            bake_enabled: true,
            perspective_editor_wall: false,
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
        settings
            .keybindings
            .rebind(KeybindAction::Recent1, KeyBinding::with_shift(Key::F2));
        let encoded = toml::to_string_pretty(&settings).unwrap();

        assert!(encoded.contains("[keybindings.show_areas]"));
        assert!(encoded.contains("[keybindings.recent_1]"));
        assert!(encoded.contains("maximized = true"));
        assert!(encoded.contains("preferred_editor = \"zed {file}:{line}:{column}\""));
        assert!(encoded.contains("selection_highlight = \"tint\""));
        assert!(encoded.contains("object_tree_line_indicators = false"));
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
            show_lighting: true,
            focus_windows_on_hover: true,
            tile_place_flash: false,
            selection_guide_line: false,
            show_tile_grid: true,
            tile_grid_min_pixels: 10,
            show_tile_grid_axis: true,
            show_selected_pixel_grid: true,
            selected_pixel_grid_min_pixels: 3,
            show_pixel_grid_axis: true,
            selection_highlight: SelectionHighlight::Tint,
            object_tree_line_indicators: false,
            object_tree_search: ObjectTreeSearchOptions {
                type_paths: false,
                names: true,
            },
            object_tree_filter: ObjectTreeFilterOptions::default(),
            keybindings: KeyBindings::default(),
            recent_codebases: Vec::new(),
            recent: Vec::new(),
            bake_enabled: true,
            perspective_editor_wall: false,
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
                object_tree_line_indicators: false,
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
    fn only_a_missing_settings_file_requests_a_keybinding_preset() {
        let missing = Settings::from_file_read(Err(io::Error::from(io::ErrorKind::NotFound))).unwrap();
        let missing = SettingsLoad::from_file_result(Ok(missing));
        assert!(missing.needs_keybind_preset);

        let present = Settings::from_file_read(Ok(String::new())).unwrap();
        let present = SettingsLoad::from_file_result(Ok(present));
        assert!(!present.needs_keybind_preset);
        assert_eq!(present.settings, Settings::default());

        let malformed = Settings::from_file_read(Ok(String::from("show_areas = maybe")));
        let malformed = SettingsLoad::from_file_result(malformed);
        assert!(!malformed.needs_keybind_preset);

        let unreadable = Settings::from_file_read(Err(io::Error::from(io::ErrorKind::PermissionDenied)));
        let unreadable = SettingsLoad::from_file_result(unreadable);
        assert!(!unreadable.needs_keybind_preset);
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
