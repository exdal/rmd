use std::ops::RangeInclusive;

use dear_imgui_rs::{ColorDisplayMode, ColorEditFlags, DragFlags, Style, TableFlags, TableSizingPolicy, Ui};
use ui::{
    theme::{
        FieldKind, STYLE_COLORS, STYLE_FIELDS, StyleField, TREE_LINE_MODES, Themes, direction_name, tree_lines_name,
    },
    widgets::{DialogButton, begin_centered_modal, dialog_buttons, dpi},
};

use crate::{
    frontend::dialog::{MODAL_FLAGS, SAVE_ERROR_COLOR},
    settings::Settings,
};

const NEW_THEME_POPUP: &str = "New theme";

#[derive(Default)]
pub(super) struct ThemeEditor {
    new_theme: Option<NewTheme>,
    save_error: Option<String>,
}

struct NewTheme {
    name: String,
    error: Option<String>,
}

pub(super) fn draw_theme_settings(ui: &Ui, settings: &mut Settings, themes: &mut Themes, editor: &mut ThemeEditor) {
    let mut selected = None;
    ui.set_next_item_width(260.0 * dpi(ui));
    if let Some(_combo) = ui.begin_combo("##theme", &themes.active_theme().name) {
        for (id, theme) in themes.iter() {
            if ui
                .selectable_config(format!("{}##{id}", theme.name))
                .selected(id == themes.active_id())
                .build()
            {
                selected = Some(id.to_owned());
            }
        }
    }

    if let Some(id) = selected
        && themes.select(&id)
    {
        settings.theme = id;
        editor.save_error = None;
    }

    let is_dirty = themes.is_dirty();
    {
        let _disabled = ui.begin_disabled_with_cond(!is_dirty);
        ui.same_line();
        if ui.button("Save") {
            editor.save_error = themes
                .save_active()
                .err()
                .map(|error| format!("Could not save the theme: {error}"));
        }

        ui.same_line();
        if ui.button("Revert") {
            themes.revert_active();
            editor.save_error = None;
        }
    }

    ui.same_line();
    if ui.button("New...") {
        editor.new_theme = Some(NewTheme {
            name: format!("{} copy", themes.active_theme().name),
            error: None,
        });
        ui.open_popup(NEW_THEME_POPUP);
    }

    ui.set_item_tooltip("Copy the current theme into a new file");
    if themes.is_active_builtin() {
        let _disabled = ui.begin_disabled_with_cond(themes.is_active_default());
        ui.same_line();
        if ui.button("Reset to default") {
            themes.reset_active();
        }

        ui.set_item_tooltip("Restore the built-in values. Save keeps them");
    }

    if let Some(id) = draw_new_theme_dialog(ui, themes, &mut editor.new_theme) {
        settings.theme = id;
        editor.save_error = None;
    }

    if let Some(error) = &editor.save_error {
        ui.text_colored(SAVE_ERROR_COLOR, error);
    }

    let theme = themes.active_theme_mut();
    ui.set_next_item_width(260.0 * dpi(ui));
    ui.input_text("Name", &mut theme.name).build();
    ui.set_next_item_width(260.0 * dpi(ui));
    ui.input_text("URL", &mut theme.url)
        .hint("https://github.com/...")
        .build();
    if let Some(url) = theme.web_url() {
        ui.same_line();
        ui.text_link_open_url("Open##theme-url", url);
    }

    ui.table("theme-colors")
        .flags(TableFlags::BORDERS_INNER_V | TableFlags::ROW_BG)
        .sizing_policy(TableSizingPolicy::StretchProp)
        .headers(true)
        .column("Color")
        .weight(1.0)
        .done()
        .column("Value")
        .weight(1.0)
        .done()
        .build(|ui| {
            for (color, name) in STYLE_COLORS {
                draw_theme_row(ui, name);
                let mut rgba = theme.style.color(color);
                if ui
                    .color_edit4_config(format!("##{name}"), &mut rgba)
                    .flags(ColorEditFlags::ALPHA_BAR | ColorEditFlags::ALPHA_PREVIEW_HALF)
                    .display_mode(ColorDisplayMode::Hex)
                    .build()
                {
                    theme.style.set_color(color, rgba);
                }
            }
        });

    ui.table("theme-style")
        .flags(TableFlags::BORDERS_INNER_V | TableFlags::ROW_BG)
        .sizing_policy(TableSizingPolicy::StretchProp)
        .headers(true)
        .column("Style")
        .weight(1.0)
        .done()
        .column("Value")
        .weight(1.0)
        .done()
        .build(|ui| {
            for field in STYLE_FIELDS {
                draw_theme_row(ui, field.key);
                draw_style_field(ui, &mut theme.style, field);
            }
        });
}

fn draw_new_theme_dialog(ui: &Ui, themes: &mut Themes, new_theme: &mut Option<NewTheme>) -> Option<String> {
    let _modal = begin_centered_modal(ui, NEW_THEME_POPUP, MODAL_FLAGS)?;
    let NewTheme { name, error } = new_theme.get_or_insert_with(|| NewTheme {
        name: String::new(),
        error: None,
    });
    if ui.is_window_appearing() {
        ui.set_keyboard_focus_here();
    }

    ui.set_next_item_width(260.0 * dpi(ui));
    let is_entered = ui.input_text("Name", name).enter_returns_true(true).build();
    if let Some(error) = error {
        ui.text_colored(SAVE_ERROR_COLOR, error);
    }

    let is_valid = !name.trim().is_empty();
    let pressed = dialog_buttons(
        ui,
        &[
            DialogButton::cancel("Cancel"),
            DialogButton::primary("Create Theme").enabled(is_valid),
        ],
        true,
    );
    if pressed == Some(0) {
        *new_theme = None;
        ui.close_current_popup();

        return None;
    }

    if !(pressed == Some(1) || is_entered) || !is_valid {
        return None;
    }

    match themes.create(name) {
        Ok(id) => {
            let id = id.to_owned();
            *new_theme = None;
            ui.close_current_popup();

            Some(id)
        },
        Err(cause) => {
            *error = Some(format!("Could not create the theme: {cause}"));

            None
        },
    }
}

fn draw_theme_row(ui: &Ui, key: &str) {
    ui.table_next_row();
    ui.table_next_column();
    ui.align_text_to_frame_padding();
    ui.text(key);
    ui.table_next_column();
    ui.set_next_item_width(-1.0);
}

fn draw_style_field(ui: &Ui, style: &mut Style, field: &StyleField) {
    let id = format!("##{}", field.key);
    match &field.kind {
        FieldKind::Float { get, set, range } => {
            let mut value = get(style);
            if style_drag(ui, &id, range).build(ui, &mut value) {
                set(style, value);
            }
        },
        FieldKind::Vec2 { get, set, range } => {
            let mut value = get(style);
            if style_drag(ui, &id, range).build_array(ui, &mut value) {
                set(style, value);
            }
        },
        FieldKind::Bool { get, set } => {
            let mut value = get(style);
            if ui.checkbox(&id, &mut value) {
                set(style, value);
            }
        },
        FieldKind::Direction { get, set, options } => {
            let current = get(style);
            if let Some(_combo) = ui.begin_combo(&id, direction_name(current)) {
                for &option in *options {
                    if ui
                        .selectable_config(direction_name(option))
                        .selected(option == current)
                        .build()
                    {
                        set(style, option);
                    }
                }
            }
        },
        FieldKind::TreeLines { get, set } => {
            let current = get(style);
            if let Some(_combo) = ui.begin_combo(&id, tree_lines_name(current)) {
                for (mode, name) in TREE_LINE_MODES {
                    if ui.selectable_config(name).selected(mode == current).build() {
                        set(style, mode);
                    }
                }
            }
        },
    }
}

fn style_drag<'a>(ui: &Ui, id: &'a str, range: &RangeInclusive<f32>) -> dear_imgui_rs::Drag<f32, &'a str> {
    let (min, max) = (*range.start(), *range.end());

    ui.drag_config(id)
        .range(min, max)
        .speed((max - min) * 0.005)
        .flags(DragFlags::ALWAYS_CLAMP)
}
