use std::{f32::consts::PI, ops::RangeInclusive};

use dear_imgui_rs::{Direction, Style, StyleColor, TreeLineMode};
use toml::Value;

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
pub const TREE_LINE_MODES: [(TreeLineMode, &str); 3] = [
    (TreeLineMode::NONE, "None"),
    (TreeLineMode::FULL, "Full"),
    (TreeLineMode::TO_NODES, "ToNodes"),
];

pub struct StyleField {
    pub key: &'static str,
    pub kind: FieldKind,
}

pub enum FieldKind {
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

pub const STYLE_FIELDS: &[StyleField] = &[
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

pub const STYLE_COLORS: [(StyleColor, &str); StyleColor::COUNT] = [
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

fn snap_tab_close(value: f32) -> f32 {
    if value < 0.0 { -1.0 } else { value }
}

pub const fn direction_name(direction: Direction) -> &'static str {
    match direction {
        Direction::None => "None",
        Direction::Left => "Left",
        Direction::Right => "Right",
        Direction::Up => "Up",
        Direction::Down => "Down",
    }
}

pub fn tree_lines_name(mode: TreeLineMode) -> &'static str {
    TREE_LINE_MODES
        .iter()
        .find(|(candidate, _)| *candidate == mode)
        .map_or("None", |(_, name)| name)
}

fn float_value(value: f32) -> Value {
    Value::Float(value.to_string().parse().unwrap_or(f64::from(value)))
}

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

fn clamp(value: f32, range: &RangeInclusive<f32>) -> f32 {
    value.clamp(*range.start(), *range.end())
}

impl StyleField {
    pub(super) fn write(&self, style: &Style) -> Value {
        match &self.kind {
            FieldKind::Float { get, .. } => float_value(get(style)),
            FieldKind::Vec2 { get, .. } => Value::Array(get(style).map(float_value).into()),
            FieldKind::Bool { get, .. } => Value::Boolean(get(style)),
            FieldKind::Direction { get, .. } => Value::String(direction_name(get(style)).to_owned()),
            FieldKind::TreeLines { get, .. } => Value::String(tree_lines_name(get(style)).to_owned()),
        }
    }

    pub(super) fn read(&self, style: &mut Style, value: &Value) -> bool {
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
