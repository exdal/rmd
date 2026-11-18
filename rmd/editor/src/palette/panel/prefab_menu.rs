use std::{iter, ptr};

use dear_imgui_rs::{Ui, sys};
use dmm::{Prefab, prefabs_equal};
use ui::widgets::dpi;

use crate::{
    document::DocumentId,
    palette::panel::thumbnail::{IconOrigin, draw_thumbnail},
    search::panel::{FindPanel, SimilarMatchKind},
    session::Session,
    settings::{Settings, keybind::KeybindAction},
    viewport::{panel::context_menu::begin_map_popup, preview::PlacementPreview},
};

const POPUP: &str = "Prefab variants##prefab-variants";
const ROW_HEIGHT: f32 = 64.0;
const VISIBLE_ROWS: usize = 6;
const MAX_WIDTH: f32 = 480.0;

enum RowAction<'a> {
    Choose(&'a Prefab),
    Search(&'a Prefab, SimilarMatchKind),
    GenerateDirections(usize),
    GenerateIcons(usize),
}

struct Row {
    prefab: Prefab,
    placement: Option<PlacementPreview>,
    path: String,
    overrides: Vec<String>,
    height: f32,
    is_generated: bool,
}

pub(crate) struct PrefabMenu {
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
        let rows = session
            .map_prefabs_of(document, &brush.path)
            .into_iter()
            .map(|prefab| build_row(ui, session, document, prefab, text_width, icon_extent))
            .collect::<Vec<_>>();
        let selected = rows.iter().position(|row| prefabs_equal(&row.prefab, &brush));

        Some(Self {
            document,
            tile_size: session.options.tile_size,
            icon_extent,
            text_width,
            rows,
            selected,
        })
    }

    fn insert_variants(&mut self, ui: &Ui, session: &mut Session, index: usize, variants: Vec<Prefab>) {
        session.pin_prefabs(self.document, &variants);
        let rows = variants
            .into_iter()
            .filter(|prefab| !self.rows.iter().any(|row| prefabs_equal(&row.prefab, prefab)))
            .map(|prefab| build_row(ui, session, self.document, prefab, self.text_width, self.icon_extent))
            .collect::<Vec<_>>();
        let count = rows.len();
        let at = if self.rows[index].is_generated {
            index + 1
        } else {
            self.rows
                .iter()
                .position(|row| row.is_generated)
                .unwrap_or(self.rows.len())
        };

        self.rows.splice(at..at, rows);

        if let Some(selected) = &mut self.selected
            && *selected >= at
        {
            *selected += count;
        }
    }
}

fn build_row(
    ui: &Ui, session: &mut Session, document: DocumentId, prefab: Prefab, text_width: f32, icon_extent: f32,
) -> Row {
    let spacing_y = ui.clone_style().item_spacing()[1];
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
        is_generated: !session.is_prefab_placed(document, &prefab),
        prefab,
    }
}

fn override_lines(prefab: &Prefab) -> Vec<String> {
    prefab
        .vars
        .iter()
        .map(|(name, value)| format!("{name} = {}", dmm::writer::format_value(&value.value)))
        .collect()
}

pub(crate) fn update_prefab_menu(
    ui: &Ui, session: &mut Session, settings: &Settings, document: DocumentId, is_focused: bool,
    menu: &mut Option<PrefabMenu>, find: &mut FindPanel,
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

    let Some(open) = menu.as_mut() else {
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

    match draw_rows(ui, open) {
        Some(RowAction::Choose(prefab)) => session.choose_prefab(prefab.clone()),
        Some(RowAction::Search(prefab, kind)) => {
            find.open_for_prefab(session, open.document, prefab, kind);
            ui.close_current_popup();
        },
        Some(RowAction::GenerateDirections(index)) => {
            let variants = session.direction_variants(&open.rows[index].prefab);
            open.insert_variants(ui, session, index, variants);
        },
        Some(RowAction::GenerateIcons(index)) => {
            let variants = session.icon_state_variants(&open.rows[index].prefab);
            open.insert_variants(ui, session, index, variants);
        },
        None => {},
    }
}

fn draw_rows<'a>(ui: &Ui, menu: &'a PrefabMenu) -> Option<RowAction<'a>> {
    if menu.rows.is_empty() {
        ui.text_disabled("No instances in this map");
        return None;
    }

    let mut action = None;
    for (index, row) in menu.rows.iter().enumerate() {
        let is_section_start = row.is_generated
            && index
                .checked_sub(1)
                .is_none_or(|previous| !menu.rows[previous].is_generated);
        if is_section_start {
            ui.separator_with_text("Generated");
        } else if index > 0 {
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
            action = Some(RowAction::Choose(&row.prefab));
        }

        if let Some(_menu) = ui.begin_popup_context_item() {
            if ui.menu_item("Search by Type") {
                action = Some(RowAction::Search(&row.prefab, SimilarMatchKind::Type));
            }

            if ui.menu_item("Search by Prefab ID") {
                action = Some(RowAction::Search(&row.prefab, SimilarMatchKind::Prefab));
            }

            ui.separator();
            if ui.menu_item("Generate Directions") {
                action = Some(RowAction::GenerateDirections(index));
            }

            if ui.menu_item("Generate Icons") {
                action = Some(RowAction::GenerateIcons(index));
            }
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

    action
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
