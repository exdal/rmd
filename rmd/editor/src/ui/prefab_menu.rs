use std::{iter, ptr};

use dear_imgui_rs::{Ui, sys};
use dmm::Prefab;
use editor::document::DocumentId;

use super::{
    common::{IconOrigin, dpi, draw_thumbnail},
    context_menu::begin_map_popup,
};
use crate::{
    session::{PlacementPreview, Session},
    settings::{KeybindAction, Settings},
};

const POPUP: &str = "Prefab variants##prefab-variants";
const ROW_HEIGHT: f32 = 64.0;
const VISIBLE_ROWS: usize = 6;
const MAX_WIDTH: f32 = 480.0;
const EMPTY_TEXT: &str = "No instances in this map";

struct Row {
    prefab: Prefab,
    placement: Option<PlacementPreview>,
    path: String,
    overrides: Vec<String>,
    height: f32,
}

pub(super) struct PrefabMenu {
    document: DocumentId,
    tile_size: u32,
    icon_extent: f32,
    text_width: f32,
    rows: Vec<Row>,
    selected: Option<usize>,
}

impl PrefabMenu {
    fn new(ui: &Ui, session: &mut Session, document: DocumentId) -> Option<Self> {
        let brush = session.palette()?.clone();
        let icon_extent = ROW_HEIGHT * dpi(ui);
        let style = ui.clone_style();
        let text_width = (MAX_WIDTH * dpi(ui)
            - 2.0 * style.window_padding()[0]
            - icon_extent
            - 2.0 * style.item_spacing()[0]
            - 1.0
            - style.scrollbar_size())
        .max(1.0);
        let spacing_y = style.item_spacing()[1];
        let rows = session
            .map_prefabs_of(document, &brush.path)
            .into_iter()
            .map(|prefab| {
                let path = prefab.path.to_string();
                let overrides = override_lines(&prefab);
                let text_height = iter::once(&path)
                    .chain(&overrides)
                    .map(|line| ui.calc_text_size_with_opts(line, false, text_width)[1] + spacing_y)
                    .sum::<f32>()
                    - spacing_y;

                Row {
                    placement: session.prefab_placement(&prefab),
                    path,
                    overrides,
                    height: icon_extent.max(text_height),
                    prefab,
                }
            })
            .collect::<Vec<_>>();
        let selected = rows.iter().position(|row| row.prefab == brush);

        Some(Self {
            document,
            tile_size: session.options.tile_size,
            icon_extent,
            text_width,
            rows,
            selected,
        })
    }
}

fn override_lines(prefab: &Prefab) -> Vec<String> {
    prefab
        .vars
        .iter()
        .map(|(name, value)| format!("{name} = {}", dmm::writer::format_value(&value.value)))
        .collect()
}

pub(super) fn update_prefab_menu(
    ui: &Ui, session: &mut Session, settings: &Settings, document: DocumentId, is_focused: bool,
    menu: &mut Option<PrefabMenu>,
) {
    let is_open_requested = settings.keybindings.get(KeybindAction::PrefabVariants).is_pressed(ui)
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
    let max_height = VISIBLE_ROWS as f32 * (open.icon_extent + spacing_y) - spacing_y + 2.0 * style.window_padding()[1];
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
        if index > 0 {
            ui.separator();
        }

        let origin = ui.cursor_screen_pos();
        ui.set_next_item_allow_overlap();
        if ui
            .selectable_config(format!("##prefab-{index}"))
            .selected(menu.selected == Some(index))
            .size([0.0, row.height])
            .build()
        {
            chosen = Some(&row.prefab);
        }

        if !ui.is_item_visible() {
            continue;
        }

        ui.set_cursor_screen_pos(origin);
        draw_icon(ui, row.placement, menu.tile_size, menu.icon_extent, row.height);
        ui.same_line();
        ui.separator_vertical();
        ui.same_line();
        let _wrap = ui.push_text_wrap_pos(ui.cursor_pos()[0] + menu.text_width);
        ui.group(|| {
            ui.text(&row.path);
            for line in &row.overrides {
                ui.text(line);
            }
        });
    }

    chosen
}

fn draw_icon(ui: &Ui, placement: Option<PlacementPreview>, tile_size: u32, extent: f32, row_height: f32) {
    ui.dummy([extent, row_height]);
    draw_thumbnail(
        ui,
        placement.map(|placement| placement.thumbnail),
        ui.item_rect_min(),
        extent,
        placement.map(|placement| IconOrigin {
            offset: placement.offset,
            tile_size,
        }),
    );
}
