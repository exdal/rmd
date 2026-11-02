use std::ptr;

use dear_imgui_rs::{StyleColor, Ui, sys};
use dmm::Prefab;
use editor::{document::DocumentId, icons::materialdesignicons::ICON_IMAGE_BROKEN, tool::Tool};
use render::Renderer;

use super::{
    common::{dpi, fit_icon},
    context_menu::begin_map_popup,
};
use crate::{
    session::{PrefabThumbnail, Session},
    settings::{KeybindAction, Settings},
};

const POPUP: &str = "Prefab variants##prefab-variants";
const ROW_HEIGHT: f32 = 64.0;
const VISIBLE_ROWS: usize = 6;
const MAX_WIDTH: f32 = 480.0;
const EMPTY_TEXT: &str = "No instances in this map";

struct Row {
    prefab: Prefab,
    thumbnail: Option<PrefabThumbnail>,
    path: String,
    overrides: Vec<String>,
}

pub(super) struct PrefabMenu {
    document: DocumentId,
    row_height: f32,
    rows: Vec<Row>,
    selected: Option<usize>,
}

impl PrefabMenu {
    fn new(ui: &Ui, session: &mut Session, document: DocumentId) -> Option<Self> {
        let brush = session.palette()?.clone();
        let row_height = ROW_HEIGHT * dpi(ui);
        let max_lines = (row_height / ui.text_line_height_with_spacing()).floor().max(1.0) as usize;
        let rows = session
            .map_prefabs_of(document, &brush.path)
            .into_iter()
            .map(|prefab| Row {
                thumbnail: session.prefab_thumbnail(&prefab),
                path: prefab.path.to_string(),
                overrides: override_lines(&prefab, max_lines - 1),
                prefab,
            })
            .collect::<Vec<_>>();
        let selected = rows.iter().position(|row| row.prefab == brush);

        Some(Self {
            document,
            row_height,
            rows,
            selected,
        })
    }
}

fn override_lines(prefab: &Prefab, max: usize) -> Vec<String> {
    let mut lines = prefab
        .vars
        .iter()
        .map(|(name, value)| format!("{name} = {}", dmm::writer::format_value(&value.value)))
        .collect::<Vec<_>>();
    if lines.len() > max {
        let shown = max.saturating_sub(1);
        let hidden = lines.len() - shown;
        lines.truncate(shown);
        lines.push(format!("+{hidden} more"));
    }

    lines
}

pub(super) fn update_prefab_menu(
    ui: &Ui, session: &mut Session, settings: &Settings, document: DocumentId, is_focused: bool,
    menu: &mut Option<PrefabMenu>,
) {
    let is_open_requested = settings.keybindings.get(KeybindAction::PrefabVariants).is_pressed(ui)
        && session.tool() == Tool::Place
        && is_focused
        && !ui.io().want_text_input();
    if is_open_requested && let Some(opened) = PrefabMenu::new(ui, session, document) {
        *menu = Some(opened);
        ui.open_popup(POPUP);
    }

    if !ui.is_popup_open(POPUP) {
        *menu = None;
        return;
    }

    let Some(open) = menu.as_ref() else {
        return;
    };

    let style = ui.clone_style();
    let spacing_y = style.item_spacing()[1];
    let max_height = VISIBLE_ROWS as f32 * (open.row_height + spacing_y) - spacing_y + 2.0 * style.window_padding()[1];
    unsafe {
        sys::igSetNextWindowSizeConstraints(
            sys::ImVec2_c { x: 0.0, y: 0.0 },
            sys::ImVec2_c {
                x: MAX_WIDTH * dpi(ui),
                y: max_height,
            },
            None,
            ptr::null_mut(),
        );
    }
    let Some(_popup) = begin_map_popup(ui, session, POPUP, open.document) else {
        return;
    };

    if let Some(prefab) = draw_rows(ui, open) {
        session.choose_prefab(prefab.clone());
    }
}

fn draw_rows<'a>(ui: &Ui, menu: &'a PrefabMenu) -> Option<&'a Prefab> {
    if menu.rows.is_empty() {
        ui.text_disabled(EMPTY_TEXT);
        return None;
    }

    let mut chosen = None;
    for (index, row) in menu.rows.iter().enumerate() {
        let origin = ui.cursor_screen_pos();
        ui.set_next_item_allow_overlap();
        if ui
            .selectable_config(format!("##prefab-{index}"))
            .selected(menu.selected == Some(index))
            .size([0.0, menu.row_height])
            .build()
        {
            chosen = Some(&row.prefab);
        }

        if !ui.is_item_visible() {
            continue;
        }

        ui.set_cursor_screen_pos(origin);
        draw_icon(ui, row.thumbnail, menu.row_height);
        ui.same_line();
        ui.separator_vertical();
        ui.same_line();
        ui.group(|| {
            ui.text(&row.path);
            for line in &row.overrides {
                ui.text(line);
            }
        });
    }

    chosen
}

fn draw_icon(ui: &Ui, thumbnail: Option<PrefabThumbnail>, extent: f32) {
    ui.dummy([extent, extent]);
    let min = ui.item_rect_min();
    let draw = ui.get_window_draw_list();
    match thumbnail {
        Some(thumbnail) => {
            let size = fit_icon(thumbnail.texture.width, thumbnail.texture.height, extent);
            let min = [min[0] + (extent - size[0]) * 0.5, min[1] + (extent - size[1]) * 0.5];
            draw.add_image(
                Renderer::sprite_texture(thumbnail.texture.index),
                min,
                [min[0] + size[0], min[1] + size[1]],
                thumbnail.uv0,
                thumbnail.uv1,
                thumbnail.tint,
            );
        },
        None => {
            let icon = ICON_IMAGE_BROKEN.to_string();
            let size = ui.calc_text_size(&icon);
            draw.add_text(
                [min[0] + (extent - size[0]) * 0.5, min[1] + (extent - size[1]) * 0.5],
                ui.style_color(StyleColor::TextDisabled),
                icon,
            );
        },
    }
}
