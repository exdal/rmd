use std::{
    error::Error,
    f32::consts::PI,
    fs,
    ops::RangeInclusive,
    path::{Path, PathBuf},
};

use dear_imgui_rs::{Direction, Style, StyleColor, ThemePreset, TreeLineMode};
use serde::{Deserialize, Serialize};
use toml::{Table, Value};

pub(crate) const DEFAULT_THEME: &str = "imgui_dark";
const LIGHT_THEME: &str = "imgui_light";
const TABLE_ROW_ALT_ALPHA_SCALE: f32 = 0.4;

const UNIT: RangeInclusive<f32> = 0.0..=1.0;
const ALPHA: RangeInclusive<f32> = 0.2..=1.0;
const SIZE: RangeInclusive<f32> = 0.0..=128.0;
const MIN_SIZE: RangeInclusive<f32> = 1.0..=512.0;
const POSITIVE: RangeInclusive<f32> = 0.01..=16.0;
const DELAY: RangeInclusive<f32> = 0.0..=5.0;
const ANGLE: RangeInclusive<f32> = -50.0 * PI / 180.0..=50.0 * PI / 180.0;
// -1.0 always shows the close button
const TAB_CLOSE: RangeInclusive<f32> = -1.0..=64.0;

const MENU_BUTTON_POSITIONS: &[Direction] = &[Direction::None, Direction::Left, Direction::Right];
const COLOR_BUTTON_POSITIONS: &[Direction] = &[Direction::Left, Direction::Right];
pub(crate) const TREE_LINE_MODES: [(TreeLineMode, &str); 3] = [
    (TreeLineMode::NONE, "None"),
    (TreeLineMode::FULL, "Full"),
    (TreeLineMode::TO_NODES, "ToNodes"),
];

pub(crate) struct StyleField {
    pub key: &'static str,
    pub kind: FieldKind,
}

pub(crate) enum FieldKind {
    Float {
        get: fn(&Style) -> f32,
        set: fn(&mut Style, f32),
        range: RangeInclusive<f32>,
    },
    Vec2 {
        get: fn(&Style) -> [f32; 2],
        set: fn(&mut Style, [f32; 2]),
        range: RangeInclusive<f32>,
    },
    Bool {
        get: fn(&Style) -> bool,
        set: fn(&mut Style, bool),
    },
    Direction {
        get: fn(&Style) -> Direction,
        set: fn(&mut Style, Direction),
        options: &'static [Direction],
    },
    TreeLines {
        get: fn(&Style) -> TreeLineMode,
        set: fn(&mut Style, TreeLineMode),
    },
}

const fn float(
    key: &'static str, get: fn(&Style) -> f32, set: fn(&mut Style, f32), range: RangeInclusive<f32>,
) -> StyleField {
    StyleField {
        key,
        kind: FieldKind::Float { get, set, range },
    }
}

const fn vec2(
    key: &'static str, get: fn(&Style) -> [f32; 2], set: fn(&mut Style, [f32; 2]), range: RangeInclusive<f32>,
) -> StyleField {
    StyleField {
        key,
        kind: FieldKind::Vec2 { get, set, range },
    }
}

const fn boolean(key: &'static str, get: fn(&Style) -> bool, set: fn(&mut Style, bool)) -> StyleField {
    StyleField {
        key,
        kind: FieldKind::Bool { get, set },
    }
}

const fn direction(
    key: &'static str, get: fn(&Style) -> Direction, set: fn(&mut Style, Direction), options: &'static [Direction],
) -> StyleField {
    StyleField {
        key,
        kind: FieldKind::Direction { get, set, options },
    }
}

pub(crate) const STYLE_FIELDS: &[StyleField] = &[
    float("Alpha", Style::alpha, Style::set_alpha, ALPHA),
    float("DisabledAlpha", Style::disabled_alpha, Style::set_disabled_alpha, UNIT),
    vec2("WindowPadding", Style::window_padding, Style::set_window_padding, SIZE),
    float(
        "WindowRounding",
        Style::window_rounding,
        Style::set_window_rounding,
        SIZE,
    ),
    float(
        "WindowBorderSize",
        Style::window_border_size,
        Style::set_window_border_size,
        SIZE,
    ),
    float(
        "WindowBorderHoverPadding",
        Style::window_border_hover_padding,
        Style::set_window_border_hover_padding,
        POSITIVE,
    ),
    vec2(
        "WindowMinSize",
        Style::window_min_size,
        Style::set_window_min_size,
        MIN_SIZE,
    ),
    vec2(
        "WindowTitleAlign",
        Style::window_title_align,
        Style::set_window_title_align,
        UNIT,
    ),
    direction(
        "WindowMenuButtonPosition",
        Style::window_menu_button_position,
        Style::set_window_menu_button_position,
        MENU_BUTTON_POSITIONS,
    ),
    float("ChildRounding", Style::child_rounding, Style::set_child_rounding, SIZE),
    float(
        "ChildBorderSize",
        Style::child_border_size,
        Style::set_child_border_size,
        SIZE,
    ),
    float("PopupRounding", Style::popup_rounding, Style::set_popup_rounding, SIZE),
    float(
        "PopupBorderSize",
        Style::popup_border_size,
        Style::set_popup_border_size,
        SIZE,
    ),
    vec2("FramePadding", Style::frame_padding, Style::set_frame_padding, SIZE),
    float("FrameRounding", Style::frame_rounding, Style::set_frame_rounding, SIZE),
    float(
        "FrameBorderSize",
        Style::frame_border_size,
        Style::set_frame_border_size,
        SIZE,
    ),
    vec2("ItemSpacing", Style::item_spacing, Style::set_item_spacing, SIZE),
    vec2(
        "ItemInnerSpacing",
        Style::item_inner_spacing,
        Style::set_item_inner_spacing,
        SIZE,
    ),
    vec2("CellPadding", Style::cell_padding, Style::set_cell_padding, SIZE),
    vec2(
        "TouchExtraPadding",
        Style::touch_extra_padding,
        Style::set_touch_extra_padding,
        SIZE,
    ),
    float("IndentSpacing", Style::indent_spacing, Style::set_indent_spacing, SIZE),
    float(
        "ColumnsMinSpacing",
        Style::columns_min_spacing,
        Style::set_columns_min_spacing,
        SIZE,
    ),
    float("ScrollbarSize", Style::scrollbar_size, Style::set_scrollbar_size, SIZE),
    float(
        "ScrollbarRounding",
        Style::scrollbar_rounding,
        Style::set_scrollbar_rounding,
        SIZE,
    ),
    float(
        "ScrollbarPadding",
        Style::scrollbar_padding,
        Style::set_scrollbar_padding,
        SIZE,
    ),
    float("GrabMinSize", Style::grab_min_size, Style::set_grab_min_size, SIZE),
    float("GrabRounding", Style::grab_rounding, Style::set_grab_rounding, SIZE),
    float(
        "LogSliderDeadzone",
        Style::log_slider_deadzone,
        Style::set_log_slider_deadzone,
        SIZE,
    ),
    float("ImageRounding", Style::image_rounding, Style::set_image_rounding, SIZE),
    float(
        "ImageBorderSize",
        Style::image_border_size,
        Style::set_image_border_size,
        SIZE,
    ),
    float("TabRounding", Style::tab_rounding, Style::set_tab_rounding, SIZE),
    float(
        "TabBorderSize",
        Style::tab_border_size,
        Style::set_tab_border_size,
        SIZE,
    ),
    float(
        "TabMinWidthBase",
        Style::tab_min_width_base,
        Style::set_tab_min_width_base,
        SIZE,
    ),
    float(
        "TabMinWidthShrink",
        Style::tab_min_width_shrink,
        Style::set_tab_min_width_shrink,
        SIZE,
    ),
    float(
        "TabCloseButtonMinWidthSelected",
        Style::tab_close_button_min_width_selected,
        |style, value| style.set_tab_close_button_min_width_selected(snap_tab_close(value)),
        TAB_CLOSE,
    ),
    float(
        "TabCloseButtonMinWidthUnselected",
        Style::tab_close_button_min_width_unselected,
        |style, value| style.set_tab_close_button_min_width_unselected(snap_tab_close(value)),
        TAB_CLOSE,
    ),
    float(
        "TabBarBorderSize",
        Style::tab_bar_border_size,
        Style::set_tab_bar_border_size,
        SIZE,
    ),
    float(
        "TabBarOverlineSize",
        Style::tab_bar_overline_size,
        Style::set_tab_bar_overline_size,
        SIZE,
    ),
    float(
        "TableAngledHeadersAngle",
        Style::table_angled_headers_angle,
        Style::set_table_angled_headers_angle,
        ANGLE,
    ),
    vec2(
        "TableAngledHeadersTextAlign",
        Style::table_angled_headers_text_align,
        Style::set_table_angled_headers_text_align,
        UNIT,
    ),
    StyleField {
        key: "TreeLinesFlags",
        kind: FieldKind::TreeLines {
            get: Style::tree_lines_mode,
            set: Style::set_tree_lines_mode,
        },
    },
    float(
        "TreeLinesSize",
        Style::tree_lines_size,
        Style::set_tree_lines_size,
        SIZE,
    ),
    float(
        "TreeLinesRounding",
        Style::tree_lines_rounding,
        Style::set_tree_lines_rounding,
        SIZE,
    ),
    float(
        "DragDropTargetRounding",
        Style::drag_drop_target_rounding,
        Style::set_drag_drop_target_rounding,
        SIZE,
    ),
    float(
        "DragDropTargetBorderSize",
        Style::drag_drop_target_border_size,
        Style::set_drag_drop_target_border_size,
        SIZE,
    ),
    float(
        "DragDropTargetPadding",
        Style::drag_drop_target_padding,
        Style::set_drag_drop_target_padding,
        SIZE,
    ),
    float(
        "ColorMarkerSize",
        Style::color_marker_size,
        Style::set_color_marker_size,
        SIZE,
    ),
    direction(
        "ColorButtonPosition",
        Style::color_button_position,
        Style::set_color_button_position,
        COLOR_BUTTON_POSITIONS,
    ),
    vec2(
        "ButtonTextAlign",
        Style::button_text_align,
        Style::set_button_text_align,
        UNIT,
    ),
    vec2(
        "SelectableTextAlign",
        Style::selectable_text_align,
        Style::set_selectable_text_align,
        UNIT,
    ),
    float("SeparatorSize", Style::separator_size, Style::set_separator_size, SIZE),
    float(
        "SeparatorTextBorderSize",
        Style::separator_text_border_size,
        Style::set_separator_text_border_size,
        SIZE,
    ),
    vec2(
        "SeparatorTextAlign",
        Style::separator_text_align,
        Style::set_separator_text_align,
        UNIT,
    ),
    vec2(
        "SeparatorTextPadding",
        Style::separator_text_padding,
        Style::set_separator_text_padding,
        SIZE,
    ),
    float(
        "MenuItemRounding",
        Style::menu_item_rounding,
        Style::set_menu_item_rounding,
        SIZE,
    ),
    float(
        "SelectableRounding",
        Style::selectable_rounding,
        Style::set_selectable_rounding,
        SIZE,
    ),
    vec2(
        "DisplayWindowPadding",
        Style::display_window_padding,
        Style::set_display_window_padding,
        SIZE,
    ),
    vec2(
        "DisplaySafeAreaPadding",
        Style::display_safe_area_padding,
        Style::set_display_safe_area_padding,
        SIZE,
    ),
    boolean(
        "DockingNodeHasCloseButton",
        Style::docking_node_has_close_button,
        Style::set_docking_node_has_close_button,
    ),
    float(
        "DockingSeparatorSize",
        Style::docking_separator_size,
        Style::set_docking_separator_size,
        SIZE,
    ),
    float(
        "MouseCursorScale",
        Style::mouse_cursor_scale,
        Style::set_mouse_cursor_scale,
        POSITIVE,
    ),
    float(
        "InputTextCursorSize",
        Style::input_text_cursor_size,
        Style::set_input_text_cursor_size,
        POSITIVE,
    ),
    boolean(
        "AntiAliasedLines",
        Style::anti_aliased_lines,
        Style::set_anti_aliased_lines,
    ),
    boolean(
        "AntiAliasedLinesUseTex",
        Style::anti_aliased_lines_use_tex,
        Style::set_anti_aliased_lines_use_tex,
    ),
    boolean(
        "AntiAliasedFill",
        Style::anti_aliased_fill,
        Style::set_anti_aliased_fill,
    ),
    float(
        "CurveTessellationTol",
        Style::curve_tessellation_tol,
        Style::set_curve_tessellation_tol,
        POSITIVE,
    ),
    float(
        "CircleTessellationMaxError",
        Style::circle_tessellation_max_error,
        Style::set_circle_tessellation_max_error,
        POSITIVE,
    ),
    float(
        "HoverStationaryDelay",
        Style::hover_stationary_delay,
        Style::set_hover_stationary_delay,
        DELAY,
    ),
    float(
        "HoverDelayShort",
        Style::hover_delay_short,
        Style::set_hover_delay_short,
        DELAY,
    ),
    float(
        "HoverDelayNormal",
        Style::hover_delay_normal,
        Style::set_hover_delay_normal,
        DELAY,
    ),
];

pub(crate) const STYLE_COLORS: [(StyleColor, &str); StyleColor::COUNT] = [
    (StyleColor::Text, "Text"),
    (StyleColor::TextDisabled, "TextDisabled"),
    (StyleColor::WindowBg, "WindowBg"),
    (StyleColor::ChildBg, "ChildBg"),
    (StyleColor::PopupBg, "PopupBg"),
    (StyleColor::Border, "Border"),
    (StyleColor::BorderShadow, "BorderShadow"),
    (StyleColor::FrameBg, "FrameBg"),
    (StyleColor::FrameBgHovered, "FrameBgHovered"),
    (StyleColor::FrameBgActive, "FrameBgActive"),
    (StyleColor::TitleBg, "TitleBg"),
    (StyleColor::TitleBgActive, "TitleBgActive"),
    (StyleColor::TitleBgCollapsed, "TitleBgCollapsed"),
    (StyleColor::MenuBarBg, "MenuBarBg"),
    (StyleColor::ScrollbarBg, "ScrollbarBg"),
    (StyleColor::ScrollbarGrab, "ScrollbarGrab"),
    (StyleColor::ScrollbarGrabHovered, "ScrollbarGrabHovered"),
    (StyleColor::ScrollbarGrabActive, "ScrollbarGrabActive"),
    (StyleColor::CheckMark, "CheckMark"),
    (StyleColor::CheckboxSelectedBg, "CheckboxSelectedBg"),
    (StyleColor::SliderGrab, "SliderGrab"),
    (StyleColor::SliderGrabActive, "SliderGrabActive"),
    (StyleColor::Button, "Button"),
    (StyleColor::ButtonHovered, "ButtonHovered"),
    (StyleColor::ButtonActive, "ButtonActive"),
    (StyleColor::Header, "Header"),
    (StyleColor::HeaderHovered, "HeaderHovered"),
    (StyleColor::HeaderActive, "HeaderActive"),
    (StyleColor::Separator, "Separator"),
    (StyleColor::SeparatorHovered, "SeparatorHovered"),
    (StyleColor::SeparatorActive, "SeparatorActive"),
    (StyleColor::ResizeGrip, "ResizeGrip"),
    (StyleColor::ResizeGripHovered, "ResizeGripHovered"),
    (StyleColor::ResizeGripActive, "ResizeGripActive"),
    (StyleColor::InputTextCursor, "InputTextCursor"),
    (StyleColor::TabHovered, "TabHovered"),
    (StyleColor::Tab, "Tab"),
    (StyleColor::TabSelected, "TabSelected"),
    (StyleColor::TabSelectedOverline, "TabSelectedOverline"),
    (StyleColor::TabDimmed, "TabDimmed"),
    (StyleColor::TabDimmedSelected, "TabDimmedSelected"),
    (StyleColor::TabDimmedSelectedOverline, "TabDimmedSelectedOverline"),
    (StyleColor::DockingPreview, "DockingPreview"),
    (StyleColor::DockingEmptyBg, "DockingEmptyBg"),
    (StyleColor::PlotLines, "PlotLines"),
    (StyleColor::PlotLinesHovered, "PlotLinesHovered"),
    (StyleColor::PlotHistogram, "PlotHistogram"),
    (StyleColor::PlotHistogramHovered, "PlotHistogramHovered"),
    (StyleColor::TableHeaderBg, "TableHeaderBg"),
    (StyleColor::TableBorderStrong, "TableBorderStrong"),
    (StyleColor::TableBorderLight, "TableBorderLight"),
    (StyleColor::TableRowBg, "TableRowBg"),
    (StyleColor::TableRowBgAlt, "TableRowBgAlt"),
    (StyleColor::TextLink, "TextLink"),
    (StyleColor::TextSelectedBg, "TextSelectedBg"),
    (StyleColor::TreeLines, "TreeLines"),
    (StyleColor::DragDropTarget, "DragDropTarget"),
    (StyleColor::DragDropTargetBg, "DragDropTargetBg"),
    (StyleColor::UnsavedMarker, "UnsavedMarker"),
    (StyleColor::NavCursor, "NavCursor"),
    (StyleColor::NavWindowingHighlight, "NavWindowingHighlight"),
    (StyleColor::NavWindowingDimBg, "NavWindowingDimBg"),
    (StyleColor::ModalWindowDimBg, "ModalWindowDimBg"),
];

fn snap_tab_close(value: f32) -> f32 { if value < 0.0 { -1.0 } else { value } }

pub(crate) const fn direction_name(direction: Direction) -> &'static str {
    match direction {
        Direction::None => "None",
        Direction::Left => "Left",
        Direction::Right => "Right",
        Direction::Up => "Up",
        Direction::Down => "Down",
    }
}

pub(crate) fn tree_lines_name(mode: TreeLineMode) -> &'static str {
    TREE_LINE_MODES
        .iter()
        .find(|(candidate, _)| *candidate == mode)
        .map_or("None", |(_, name)| name)
}

fn color_byte(channel: f32) -> u8 { (channel.clamp(0.0, 1.0) * 255.0).round() as u8 }

pub(crate) fn hex_color(rgba: [f32; 4]) -> String {
    let [r, g, b, a] = rgba.map(color_byte);

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

fn float_value(value: f32) -> Value { Value::Float(value.to_string().parse().unwrap_or(f64::from(value))) }

fn read_float(value: &Value) -> Option<f32> {
    let value = match value {
        Value::Float(value) => *value as f32,
        Value::Integer(value) => *value as f32,
        _ => return None,
    };

    value.is_finite().then_some(value)
}

fn read_vec2(value: &Value) -> Option<[f32; 2]> {
    match value.as_array()?.as_slice() {
        [x, y] => Some([read_float(x)?, read_float(y)?]),
        _ => None,
    }
}

fn clamp(value: f32, range: &RangeInclusive<f32>) -> f32 { value.clamp(*range.start(), *range.end()) }

impl StyleField {
    fn write(&self, style: &Style) -> Value {
        match &self.kind {
            FieldKind::Float { get, .. } => float_value(get(style)),
            FieldKind::Vec2 { get, .. } => Value::Array(get(style).map(float_value).into()),
            FieldKind::Bool { get, .. } => Value::Boolean(get(style)),
            FieldKind::Direction { get, .. } => Value::String(direction_name(get(style)).to_owned()),
            FieldKind::TreeLines { get, .. } => Value::String(tree_lines_name(get(style)).to_owned()),
        }
    }

    fn read(&self, style: &mut Style, value: &Value) -> bool {
        match &self.kind {
            FieldKind::Float { set, range, .. } => read_float(value).map(|value| set(style, clamp(value, range))),
            FieldKind::Vec2 { set, range, .. } => {
                read_vec2(value).map(|value| set(style, value.map(|value| clamp(value, range))))
            },
            FieldKind::Bool { set, .. } => value.as_bool().map(|value| set(style, value)),
            FieldKind::Direction { set, options, .. } => value
                .as_str()
                .and_then(|name| options.iter().find(|option| direction_name(**option) == name))
                .map(|option| set(style, *option)),
            FieldKind::TreeLines { set, .. } => value
                .as_str()
                .and_then(|name| TREE_LINE_MODES.iter().find(|(_, candidate)| *candidate == name))
                .map(|(mode, _)| set(style, *mode)),
        }
        .is_some()
    }
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
pub(crate) struct Theme {
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
            let rgba = style.color(color).map(|channel| f32::from(color_byte(channel)) / 255.0);
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

pub(crate) struct Themes {
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

    fn sort(&mut self) { self.entries.sort_by_key(|entry| entry.theme.name.to_lowercase()); }

    pub fn active_style(&self) -> &Style { &self.entries[self.active].theme.style }

    pub fn active_id(&self) -> &str { &self.entries[self.active].id }

    pub fn active_theme(&self) -> &Theme { &self.entries[self.active].theme }

    pub fn active_theme_mut(&mut self) -> &mut Theme { &mut self.entries[self.active].theme }

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

    pub fn is_active_builtin(&self) -> bool { self.active_builtin().is_some() }

    pub fn is_active_default(&self) -> bool { self.active_builtin() == Some(self.active_theme()) }

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

fn theme_path(dir: &Path, id: &str) -> PathBuf { dir.join(format!("{id}.toml")) }

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
mod tests {
    use dear_imgui_rs::Context;

    use super::*;

    fn default_style() -> Style { Context::create().style().clone() }

    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rmd-theme-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);

        dir
    }

    #[test]
    fn style_colors_cover_every_color_once() {
        for (index, (color, _)) in STYLE_COLORS.iter().enumerate() {
            assert_eq!(*color as usize, index);
        }
    }

    #[test]
    fn only_web_urls_can_be_opened() {
        let theme = |url: &str| Theme {
            name: String::new(),
            url: url.to_owned(),
            style: default_style(),
        };
        let _context = crate::ui::IMGUI_CONTEXT.lock().unwrap();

        assert_eq!(
            theme(" https://github.com/a/b ").web_url(),
            Some("https://github.com/a/b")
        );
        assert_eq!(theme("HTTP://example.com").web_url(), Some("HTTP://example.com"));
        assert_eq!(theme("file:///tmp/payload").web_url(), None);
        assert_eq!(theme("\\\\server\\share").web_url(), None);
        assert_eq!(theme("").web_url(), None);
    }

    #[test]
    fn builtin_files_fill_missing_keys_from_their_own_preset() {
        let _context = crate::ui::IMGUI_CONTEXT.lock().unwrap();
        let dir = scratch_dir("preset-base");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("imgui_light.toml"), "name = \"ImGui Light\"\n").unwrap();

        let themes = Themes::load(Some(dir.clone()), &default_style(), LIGHT_THEME);
        assert!(themes.is_active_default());

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn hex_colors_round_trip() {
        assert_eq!(hex_color([1.0, 0.0, 0.5, 0.94]), "#FF0080F0");
        assert_eq!(
            parse_hex_color("#FF0080F0").map(hex_color).as_deref(),
            Some("#FF0080F0")
        );
        assert_eq!(parse_hex_color("FF0080"), Some([1.0, 0.0, 128.0 / 255.0, 1.0]));
        assert_eq!(parse_hex_color("#FF00"), None);
        assert_eq!(parse_hex_color("#GG0000FF"), None);
    }

    #[test]
    fn builtin_themes_survive_a_round_trip() {
        let _context = crate::ui::IMGUI_CONTEXT.lock().unwrap();
        let default_style = default_style();
        for (id, theme) in builtin_themes(&default_style) {
            let written = theme.to_toml().unwrap();
            let read = Theme::parse(id, &written, &default_style).unwrap();
            assert_eq!(read, theme);
        }
    }

    #[test]
    fn partial_theme_keeps_defaults_and_ignores_unknown_keys() {
        let _context = crate::ui::IMGUI_CONTEXT.lock().unwrap();
        let base = default_style();
        let source = r##"
            [style]
            WindowRounding = 6
            Alpha = 4.0
            Bogus = 1.0

            [colors]
            WindowBg = "#102030FF"
            NotAColor = "#000000FF"
        "##;
        let theme = Theme::parse("partial", source, &base).unwrap();

        assert_eq!(theme.name, "partial");
        assert_eq!(theme.style.window_rounding(), 6.0);
        assert_eq!(theme.style.alpha(), 1.0);
        assert_eq!(
            theme.style.color(StyleColor::WindowBg),
            parse_hex_color("#102030FF").unwrap()
        );
        assert_eq!(theme.style.window_padding(), base.window_padding());
        assert_eq!(theme.style.color(StyleColor::Text), base.color(StyleColor::Text));
    }

    #[test]
    fn loading_writes_builtins_and_falls_back_to_the_default() {
        let _context = crate::ui::IMGUI_CONTEXT.lock().unwrap();
        let dir = scratch_dir("load");
        let default_style = default_style();
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("custom.toml"),
            "name = \"Custom\"\n[colors]\nText = \"#FF0000FF\"\n",
        )
        .unwrap();
        fs::write(dir.join("broken.toml"), "name = ").unwrap();

        let themes = Themes::load(Some(dir.clone()), &default_style, "missing");
        assert!(dir.join("imgui_dark.toml").exists());
        assert!(dir.join("imgui_light.toml").exists());
        assert_eq!(themes.active_id(), DEFAULT_THEME);
        assert_eq!(
            themes.iter().map(|(id, _)| id).collect::<Vec<_>>(),
            ["custom", "imgui_dark", "imgui_light"]
        );

        let mut themes = Themes::load(Some(dir.clone()), &default_style, "custom");
        assert_eq!(themes.active_theme().name, "Custom");
        themes.active_theme_mut().style.set_window_rounding(9.0);
        assert!(themes.is_dirty());
        themes.revert_active();
        assert!(!themes.is_dirty());

        themes.active_theme_mut().style.set_window_rounding(9.0);
        themes.save_active().unwrap();
        assert!(!themes.is_dirty());
        let themes = Themes::load(Some(dir.clone()), &default_style, "custom");
        assert_eq!(themes.active_style().window_rounding(), 9.0);

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn builtins_reset_and_new_themes_copy_the_active_one() {
        let _context = crate::ui::IMGUI_CONTEXT.lock().unwrap();
        let dir = scratch_dir("create");
        let mut themes = Themes::load(Some(dir.clone()), &default_style(), DEFAULT_THEME);
        themes.active_theme_mut().style.set_window_rounding(7.0);
        themes.save_active().unwrap();

        let mut themes = Themes::load(Some(dir.clone()), &default_style(), DEFAULT_THEME);
        assert!(themes.is_active_builtin());
        assert!(themes.select(LIGHT_THEME));
        assert!(themes.is_active_default());
        assert!(themes.select(DEFAULT_THEME));
        assert!(!themes.is_active_default());
        themes.reset_active();
        assert!(themes.is_active_default());
        assert!(themes.is_dirty());

        themes.active_theme_mut().style.set_window_rounding(3.0);
        assert_eq!(themes.create("  My Theme! ").unwrap(), "my_theme");
        assert_eq!(themes.create("My Theme").unwrap(), "my_theme_2");
        assert_eq!(themes.create("???").unwrap(), "theme");
        assert_eq!(themes.active_theme().name, "???");
        assert!(!themes.is_active_builtin());
        assert!(!themes.is_dirty());

        let themes = Themes::load(Some(dir.clone()), &default_style(), "my_theme");
        assert_eq!(themes.active_theme().name, "My Theme!");
        assert_eq!(themes.active_style().window_rounding(), 3.0);

        fs::remove_dir_all(dir).unwrap();
    }
}
