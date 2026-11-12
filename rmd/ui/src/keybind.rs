use dear_imgui_rs::{Key, Ui};
use serde::{Deserialize, Serialize};

pub const BINDABLE_KEYS: &[Key] = &[
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

pub const MODIFIER_KEYS: &[Key] = &[Key::ModCtrl, Key::ModShift, Key::ModAlt, Key::ModSuper];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyBinding {
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

    pub const fn with_ctrl_shift(key: Key) -> Self {
        Self {
            key,
            ctrl: true,
            shift: true,
            alt: false,
            super_key: false,
        }
    }

    pub const fn key(self) -> Key {
        self.key
    }

    pub fn from_input(ui: &Ui, key: Key) -> Self {
        let io = ui.io();

        Self {
            key,
            ctrl: io.key_ctrl() && key != Key::ModCtrl,
            shift: io.key_shift() && key != Key::ModShift,
            alt: io.key_alt() && key != Key::ModAlt,
            super_key: io.key_super() && key != Key::ModSuper,
        }
    }

    pub fn is_pressed(self, ui: &Ui) -> bool {
        ui.is_key_pressed_with_repeat(self.key, false) && self.modifiers_match(ui)
    }

    pub fn is_pressed_repeating(self, ui: &Ui) -> bool {
        ui.is_key_pressed_with_repeat(self.key, true) && self.modifiers_match(ui)
    }

    pub fn is_down(self, ui: &Ui) -> bool {
        ui.is_key_down(self.key) && self.modifiers_match(ui)
    }

    pub fn is_key_held(self, ui: &Ui) -> bool {
        ui.is_key_down(self.key)
    }

    pub fn is_held(self, ui: &Ui) -> bool {
        let io = ui.io();

        ui.is_key_down(self.key)
            && (!self.ctrl || io.key_ctrl())
            && (!self.shift || io.key_shift())
            && (!self.alt || io.key_alt())
            && (!self.super_key || io.key_super())
    }

    pub fn is_down_with(self, ui: &Ui, held: Self) -> bool {
        if !held.is_held(ui) {
            return self.is_down(ui);
        }

        Self {
            ctrl: self.ctrl || held.ctrl || held.key == Key::ModCtrl,
            shift: self.shift || held.shift || held.key == Key::ModShift,
            alt: self.alt || held.alt || held.key == Key::ModAlt,
            super_key: self.super_key || held.super_key || held.key == Key::ModSuper,
            ..self
        }
        .is_down(ui)
    }

    pub fn is_released(self, ui: &Ui) -> bool {
        ui.is_key_released(self.key) && self.modifiers_match(ui)
    }

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
            (self.ctrl, CTRL_NAME),
            (self.shift, "Shift"),
            (self.alt, "Alt"),
            (self.super_key, SUPER_NAME),
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

        label.push_str(match self.key {
            Key::ModCtrl => CTRL_NAME,
            Key::ModShift => "Shift",
            Key::ModAlt => "Alt",
            Key::ModSuper => SUPER_NAME,
            key => ui.get_key_name(key),
        });

        label
    }
}

// ImGui swaps Cmd and Ctrl on macOS, so `ctrl` holds Cmd there.
const CTRL_NAME: &str = if cfg!(target_os = "macos") { "Cmd" } else { "Ctrl" };
const SUPER_NAME: &str = if cfg!(target_os = "macos") { "Ctrl" } else { "Super" };

const fn is_false(value: &bool) -> bool {
    !*value
}
