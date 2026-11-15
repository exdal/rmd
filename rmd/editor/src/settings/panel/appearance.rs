use std::ops::RangeInclusive;

use dear_imgui_rs::{DragFlags, StbTrueTypeFontData, Ui};
use ui::{
    fonts::{BUNDLED_FONT_NAME, FontSettings, SystemFonts},
    theme::Themes,
    widgets::{button_width, dpi},
};

use crate::{
    frontend::dialog::SAVE_ERROR_COLOR,
    settings::{FONT_BRIGHTNESS_PERCENT, FONT_SIZE, Settings, UI_SCALE_PERCENT},
};

use super::{
    draw_section_heading,
    theme::{ThemeEditor, draw_theme_settings},
};

#[derive(Default)]
pub(super) struct FontPicker {
    system: SystemFonts,
    filter: String,
    error: Option<String>,
    pub(super) size_draft: Option<u32>,
}

pub(super) fn draw_appearance_settings(
    ui: &Ui, settings: &mut Settings, themes: &mut Themes, ui_scale_draft: &mut Option<u32>,
    theme_editor: &mut ThemeEditor, font_picker: &mut FontPicker,
) {
    draw_section_heading(ui, "UI scale");
    draw_ui_scale_setting(ui, settings, ui_scale_draft);

    ui.separator();
    draw_section_heading(ui, "Font");
    draw_font_settings(ui, &mut settings.font, font_picker);

    ui.separator();
    draw_section_heading(ui, "Theme");
    draw_theme_settings(ui, settings, themes, theme_editor);
}

pub(super) fn draw_ui_scale_setting(ui: &Ui, settings: &mut Settings, ui_scale_draft: &mut Option<u32>) {
    let os_scale_percent = os_scale_percent(ui);
    ui.text(format!("UI scale (%) (OS: {os_scale_percent}%)"));
    let active_percent = settings.ui_scale_override_percent.unwrap_or(os_scale_percent);
    let draft = ui_scale_draft.get_or_insert(active_percent);
    let spacing = ui.clone_style().item_spacing()[0];
    let buttons_width = button_width(ui, "Apply") + spacing + button_width(ui, "Reset");
    let available = ui.content_region_avail()[0];
    ui.set_next_item_width((260.0 * dpi(ui)).min((available - buttons_width - spacing).max(1.0)));
    ui.slider(
        "##ui-scale-percent",
        *UI_SCALE_PERCENT.start(),
        *UI_SCALE_PERCENT.end(),
        draft,
    );
    ui.same_line();
    let can_apply =
        *draft != active_percent || (settings.ui_scale_override_percent.is_some() && *draft == os_scale_percent);
    {
        let _disabled = ui.begin_disabled_with_cond(!can_apply);
        if ui.button("Apply") {
            settings.ui_scale_override_percent = (*draft != os_scale_percent).then_some(*draft);
        }
    }
    ui.same_line();
    let can_reset = settings.ui_scale_override_percent.is_some() || *draft != os_scale_percent;
    let _disabled = ui.begin_disabled_with_cond(!can_reset);
    if ui.button("Reset") {
        settings.ui_scale_override_percent = None;
        *draft = os_scale_percent;
    }
    ui.set_item_tooltip(format!("Use the OS scale ({os_scale_percent}%)"));
}

fn draw_font_settings(ui: &Ui, font: &mut FontSettings, picker: &mut FontPicker) {
    let FontPicker {
        system,
        filter,
        error,
        size_draft,
    } = picker;
    let faces = system.faces();
    let bundled = format!("{BUNDLED_FONT_NAME} (bundled)");
    let current = match &font.path {
        None => bundled.clone(),
        Some(path) => faces
            .and_then(|faces| faces.iter().find(|face| face.path == *path))
            .map_or_else(|| path.display().to_string(), |face| face.label.clone()),
    };

    let mut picked = None;
    ui.set_next_item_width(260.0 * dpi(ui));
    if let Some(_combo) = ui.begin_combo("##font", &current) {
        if ui.is_window_appearing() {
            ui.set_keyboard_focus_here();
        }

        ui.set_next_item_width(-1.0);
        ui.input_text("##font-filter", filter).hint("Search").build();
        if ui.selectable_config(&bundled).selected(font.path.is_none()).build() {
            picked = Some(None);
        }

        match faces {
            Some(faces) => {
                let needle = filter.to_lowercase();
                for face in faces.iter().filter(|face| face.label.to_lowercase().contains(&needle)) {
                    let is_selected = font.path.as_ref() == Some(&face.path);
                    if ui
                        .selectable_config(format!("{}##{}", face.label, face.path.display()))
                        .selected(is_selected)
                        .build()
                    {
                        picked = Some(Some(face.path.clone()));
                    }

                    if ui.is_item_hovered() {
                        ui.set_tooltip(face.path.display().to_string());
                    }
                }
            },
            None => ui.text_disabled("Loading system fonts..."),
        }
    }

    if let Some(path) = picked {
        let validation = path
            .as_deref()
            .map_or(Ok(()), |path| StbTrueTypeFontData::from_file(path).map(drop));
        match validation {
            Ok(()) => {
                font.path = path;
                *error = None;
            },
            Err(cause) => *error = Some(format!("This font can't be used: {cause}")),
        }
    }

    ui.same_line();
    if ui.button("Reset##font") {
        *font = FontSettings::default();
        *error = None;
        *size_draft = None;
    }

    ui.set_item_tooltip(format!("Use {BUNDLED_FONT_NAME} at the default size"));
    if let Some(error) = error {
        ui.text_colored(SAVE_ERROR_COLOR, error);
    }

    draw_font_size_setting(ui, font, size_draft);
    ui.set_next_item_width(120.0 * dpi(ui));
    drag_u32(
        ui,
        "Brightness (%)",
        &FONT_BRIGHTNESS_PERCENT,
        &mut font.brightness_percent,
    );
    ui.set_item_tooltip("Scales glyph coverage. Above 100% makes thin fonts bolder");
    ui.checkbox("Pixel snap", &mut font.pixel_snap);
    ui.set_item_tooltip("Align glyphs to whole pixels horizontally");
}

fn draw_font_size_setting(ui: &Ui, font: &mut FontSettings, size_draft: &mut Option<u32>) {
    let default_size = FontSettings::default().size;
    let draft = size_draft.get_or_insert(font.size);
    ui.set_next_item_width(120.0 * dpi(ui));
    drag_u32(ui, "##font-size", &FONT_SIZE, draft);
    ui.same_line();
    {
        let _disabled = ui.begin_disabled_with_cond(*draft == font.size);
        if ui.button("Apply##font-size") {
            font.size = *draft;
        }
    }

    ui.same_line();
    {
        let _disabled = ui.begin_disabled_with_cond(font.size == default_size && *draft == default_size);
        if ui.button("Reset##font-size") {
            font.size = default_size;
            *draft = default_size;
        }

        ui.set_item_tooltip(format!("Use the default size ({default_size} px)"));
    }

    ui.same_line();
    ui.text("Size (px)");
}

fn drag_u32(ui: &Ui, label: &str, range: &RangeInclusive<u32>, value: &mut u32) {
    let (min, max) = (*range.start() as i32, *range.end() as i32);
    let mut current = *value as i32;
    if ui
        .drag_int_config(label)
        .range(min, max)
        .flags(DragFlags::ALWAYS_CLAMP)
        .build(ui, &mut current)
    {
        *value = current.clamp(min, max) as u32;
    }
}

fn os_scale_percent(ui: &Ui) -> u32 {
    let scale = ui
        .main_viewport()
        .dpi_scale()
        .max(ui.io().display_framebuffer_scale()[0]);
    if scale.is_finite() && scale > 0.0 {
        (scale * 100.0).round().clamp(50.0, 500.0) as u32
    } else {
        100
    }
}
