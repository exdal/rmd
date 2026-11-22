use std::{mem, ptr};

use dear_imgui_rs::{Condition, Key, StyleColor, Ui, sys};
use ui::widgets::dpi;

use super::{
    CommandPalette, Mode, Outcome, POPUP,
    candidate::{ActionCandidate, AreaCandidate, PrefabCandidate, Thumbnail},
    command::Command,
};
use crate::{
    palette::panel::thumbnail::draw_placement,
    session::Session,
    settings::{Settings, keybind::KeybindAction},
};

const WIDTH: f32 = 560.0;
const TOP_OFFSET: f32 = 40.0;
const THUMBNAIL: f32 = 32.0;
const VISIBLE_ROWS: usize = 12;
const INPUT_ID: &str = "##command-palette-input";
const HINT: &str = "Search atoms  (> actions, @ areas)";

struct RowFrame {
    min: [f32; 2],
    width: f32,
    height: f32,
    thumbnail: f32,
    padding: f32,
    spacing: f32,
    line: f32,
    text: [f32; 4],
    dim: [f32; 4],
}

impl RowFrame {
    fn left(&self, has_thumbnail: bool) -> f32 {
        let thumbnail = if has_thumbnail {
            self.thumbnail + self.spacing
        } else {
            0.0
        };

        self.min[0] + self.padding + thumbnail
    }

    fn draw_text(&self, ui: &Ui, left: f32, lines: &[(&str, [f32; 4])], right: Option<&str>) {
        let draw = ui.get_window_draw_list();
        let mut limit = self.min[0] + self.width - self.padding;
        if let Some(right) = right {
            limit -= ui.calc_text_size(right)[0];
            draw.add_text([limit, self.min[1] + (self.height - self.line) * 0.5], self.dim, right);
            limit -= self.spacing;
        }

        let block = lines.len() as f32 * self.line + lines.len().saturating_sub(1) as f32 * self.spacing;
        let mut y = self.min[1] + (self.height - block) * 0.5;
        ui.with_clip_rect([left, self.min[1]], [limit, self.min[1] + self.height], true, || {
            for (text, color) in lines {
                draw.add_text([left, y], *color, text);
                y += self.line + self.spacing;
            }
        });
    }
}

impl CommandPalette {
    pub(super) fn draw(&mut self, ui: &Ui, session: &mut Session, settings: &Settings) -> Outcome {
        let style = ui.clone_style();
        let width = WIDTH * dpi(ui);
        let inner_width = width - 2.0 * style.window_padding()[0];
        let spacing_y = style.item_spacing()[1];
        let row_height = (THUMBNAIL * dpi(ui)).max(2.0 * ui.text_line_height() + spacing_y);
        let viewport = ui.main_viewport();
        let (position, size) = (viewport.work_pos(), viewport.work_size());
        unsafe {
            sys::igSetNextWindowPos(
                sys::ImVec2 {
                    x: position[0] + size[0] * 0.5,
                    y: position[1] + TOP_OFFSET * dpi(ui),
                },
                Condition::Always as i32,
                sys::ImVec2 { x: 0.5, y: 0.0 },
            );
            sys::igSetNextWindowSizeConstraints(
                sys::ImVec2_c { x: width, y: 0.0 },
                sys::ImVec2_c { x: width, y: f32::MAX },
                None,
                ptr::null_mut(),
            );
        }
        ui.set_next_window_viewport(viewport.id());

        let Some(_popup) = ui.begin_popup(POPUP) else {
            return Outcome::Close;
        };

        let toggle = settings.keybindings.get(KeybindAction::CommandPalette);
        if ui.is_key_pressed(Key::Escape) || (!ui.is_window_appearing() && toggle.is_pressed(ui)) {
            ui.close_current_popup();

            return Outcome::Close;
        }

        self.move_selection(ui);

        ui.set_next_item_width(inner_width);
        if ui.is_window_appearing() || !ui.is_any_item_active() {
            ui.set_keyboard_focus_here();
        }

        ui.input_text(INPUT_ID, &mut self.query).hint(HINT).build();

        if self.ranked.as_deref() != Some(self.query.as_str()) {
            let query = mem::take(&mut self.query);
            self.rank(&query);
            self.query = query;
        }

        let is_enter = ui.is_key_pressed(Key::Enter) || ui.is_key_pressed(Key::KeypadEnter);
        let mut chosen = is_enter.then_some(self.selected);
        if self.results.is_empty() {
            ui.text_disabled("No matches");
        } else {
            let rows = self.results.len().min(VISIBLE_ROWS);
            let height = rows as f32 * (row_height + spacing_y) - spacing_y;
            let clicked = ui
                .child_window("##command-results")
                .size([inner_width, height])
                .build(ui, || self.draw_rows(ui, session, settings, row_height))
                .flatten();
            chosen = clicked.or(chosen);
        }

        self.is_scroll_pending = false;

        match chosen.and_then(|row| self.choice(row)) {
            Some(choice) => {
                ui.close_current_popup();

                Outcome::Choose(choice)
            },
            None => Outcome::Open,
        }
    }

    fn move_selection(&mut self, ui: &Ui) {
        let last = self.results.len().saturating_sub(1);
        let previous = self.selected;
        if ui.is_key_pressed_with_repeat(Key::DownArrow, true) {
            self.selected = (self.selected + 1).min(last);
        }

        if ui.is_key_pressed_with_repeat(Key::UpArrow, true) {
            self.selected = self.selected.saturating_sub(1);
        }

        if ui.is_key_pressed_with_repeat(Key::PageDown, true) {
            self.selected = (self.selected + VISIBLE_ROWS).min(last);
        }

        if ui.is_key_pressed_with_repeat(Key::PageUp, true) {
            self.selected = self.selected.saturating_sub(VISIBLE_ROWS);
        }

        if self.selected != previous {
            self.is_scroll_pending = true;
        }
    }

    fn draw_rows(&mut self, ui: &Ui, session: &mut Session, settings: &Settings, height: f32) -> Option<usize> {
        let Self {
            prefabs,
            areas,
            actions,
            maps,
            map_indices,
            mode,
            results,
            selected,
            is_scroll_pending,
            ..
        } = self;
        let style = ui.clone_style();
        let mut clicked = None;
        for (row, &index) in results.iter().enumerate() {
            let is_selected = row == *selected;
            let min = ui.cursor_screen_pos();
            let width = ui.content_region_avail()[0];
            if ui
                .selectable_config(format!("##row-{row}"))
                .selected(is_selected)
                .size([0.0, height])
                .build()
            {
                clicked = Some(row);
            }

            if is_selected && *is_scroll_pending {
                let window_top = ui.window_pos()[1];
                if ui.item_rect_min()[1] < window_top {
                    ui.set_scroll_here_y(0.0);
                } else if ui.item_rect_max()[1] > window_top + ui.window_size()[1] {
                    ui.set_scroll_here_y(1.0);
                }
            }

            if !ui.is_item_visible() {
                continue;
            }

            let frame = RowFrame {
                min,
                width,
                height,
                thumbnail: THUMBNAIL * dpi(ui),
                padding: style.frame_padding()[0],
                spacing: style.item_spacing()[0],
                line: ui.text_line_height(),
                text: ui.style_color(StyleColor::Text),
                dim: ui.style_color(StyleColor::TextDisabled),
            };
            match mode {
                Mode::Prefabs => draw_prefab(ui, session, &mut prefabs[index], &frame),
                Mode::Actions => {
                    let candidate = &actions[index];
                    let is_opening_maps = candidate.command == Command::OpenMap && !map_indices.is_empty();
                    let names = is_opening_maps.then(|| {
                        map_indices
                            .iter()
                            .filter_map(|&map| maps[map].path.file_name())
                            .map(|name| name.to_string_lossy())
                            .collect::<Vec<_>>()
                            .join(", ")
                    });

                    draw_action(ui, settings, candidate, names.as_deref(), &frame);
                },
                Mode::Areas => draw_area(ui, &areas[index], &frame),
            }
        }

        clicked
    }
}

fn draw_prefab(ui: &Ui, session: &mut Session, candidate: &mut PrefabCandidate, frame: &RowFrame) {
    if matches!(candidate.thumbnail, Thumbnail::Pending) {
        candidate.thumbnail = Thumbnail::Resolved(session.prefab_placement(&candidate.prefab));
    }

    let placement = match candidate.thumbnail {
        Thumbnail::Pending => None,
        Thumbnail::Resolved(placement) => placement,
    };
    let thumbnail_min = [
        frame.min[0] + frame.padding,
        frame.min[1] + (frame.height - frame.thumbnail) * 0.5,
    ];
    draw_placement(ui, placement, thumbnail_min, frame.thumbnail, session.options.tile_size);

    let count = (candidate.count > 0).then(|| format!("×{}", candidate.count));
    let mut lines = vec![(candidate.path.as_str(), frame.text)];
    if !candidate.details.is_empty() {
        lines.push((candidate.details.as_str(), frame.dim));
    }

    frame.draw_text(ui, frame.left(true), &lines, count.as_deref());
}

fn draw_action(ui: &Ui, settings: &Settings, candidate: &ActionCandidate, maps: Option<&str>, frame: &RowFrame) {
    let keybind = candidate
        .command
        .keybind()
        .map(|action| settings.keybindings.get(action).label(ui));

    let mut lines = vec![(candidate.command.label(), frame.text)];
    if let Some(maps) = maps {
        lines.push((maps, frame.dim));
    }

    frame.draw_text(ui, frame.left(false), &lines, keybind.as_deref());
}

fn draw_area(ui: &Ui, candidate: &AreaCandidate, frame: &RowFrame) {
    let tiles = format!("{} tiles", candidate.count);

    frame.draw_text(
        ui,
        frame.left(false),
        &[
            (candidate.name.as_str(), frame.text),
            (candidate.path.as_str(), frame.dim),
        ],
        Some(&tiles),
    );
}
