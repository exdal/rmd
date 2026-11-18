use base::coord::Coord;
use dear_imgui_rs::{
    ItemHoveredFlags, ListClipper, SelectableFlags, TableColumnFlags, TableFlags, TableSizingPolicy, Ui,
};
use dmm::merge::TileConflict;
use ui::{
    icons::materialdesignicons::ICON_CHECK,
    widgets::{
        align_right, button_width, centered_note, checkbox_width, opaque, overflow_scroll, same_line_if_fits,
        table_min_width,
    },
};

use crate::{
    document::{DocumentId, grid::TileView},
    git::{
        conflict::{
            ConflictRow, ConflictState, OURS_COLOR, OURS_LABEL, Side, THEIRS_COLOR, UNRESOLVED_COLOR, describe_tile,
        },
        session::GitDocState,
    },
    session::Session,
};

use super::{Action, GitPanel};

impl GitPanel {
    pub(super) fn draw_conflicts(
        &mut self, ui: &Ui, session: &Session, id: DocumentId, git: &GitDocState, actions: &mut Vec<Action>,
    ) {
        let (Some(conflicts), Some((rows, status))) = (git.conflicts.as_ref(), session.conflict_table(id)) else {
            if git.pending_load {
                centered_note(
                    ui,
                    "This map has merge conflicts on disk, load them to resolve on the map",
                );
            } else {
                centered_note(ui, "No merge conflicts in this map");
            }

            return;
        };

        let total = rows.len();
        let resolved = status.iter().filter(|side| side.is_some()).count();
        let theirs = conflicts.side_label(Side::Theirs).to_owned();

        draw_conflict_progress(ui, total, resolved);

        self.draw_take_buttons(ui, rows, conflicts, actions);

        draw_mark_resolved(ui, session, id, git, actions);

        ui.checkbox("Unresolved only", &mut self.unresolved_only);
        let label = "Current level only";
        same_line_if_fits(ui, checkbox_width(ui, label));
        ui.checkbox(label, &mut self.current_level_only);

        let z = session.z();
        let visible = (0..rows.len())
            .filter(|index| !self.unresolved_only || status[*index].is_none())
            .filter(|index| !self.current_level_only || rows[*index].coord.z == z)
            .collect::<Vec<_>>();
        let order = visible.iter().map(|index| rows[*index].coord).collect::<Vec<_>>();
        let tile_width = ui.calc_text_size("000, 000, 00")[0];
        let buttons_width = ui.calc_text_size(format!("{OURS_LABEL}{theirs}"))[0]
            + ui.clone_style().frame_padding()[0] * 4.0
            + ui.clone_style().item_spacing()[0];
        let (scroll, inner_width) = overflow_scroll(ui, table_min_width(ui, &[tile_width, buttons_width], 2));

        ui.table("git-conflicts")
            .flags(
                TableFlags::ROW_BG
                    | TableFlags::BORDERS_INNER_V
                    | TableFlags::BORDERS_OUTER
                    | TableFlags::RESIZABLE
                    | TableFlags::HIDEABLE
                    | TableFlags::SCROLL_Y
                    | scroll,
            )
            .sizing_policy(TableSizingPolicy::StretchProp)
            .inner_width(inner_width)
            .freeze(1, 1)
            .headers(true)
            .column("Tile")
            .width(tile_width)
            .flags(TableColumnFlags::NO_HIDE)
            .done()
            .column("Hunk")
            .weight(0.8)
            .done()
            .column("Base")
            .weight(1.5)
            .flags(TableColumnFlags::DEFAULT_HIDE)
            .done()
            .column("Status")
            .weight(1.0)
            .done()
            .column("##actions")
            .width(buttons_width)
            .flags(TableColumnFlags::NO_HIDE)
            .done()
            .build(|ui| {
                let clipper = ListClipper::new(visible.len()).begin(ui);
                for position in clipper.iter() {
                    let index = visible[position];
                    let row = &rows[index];
                    let coord = row.coord;
                    let conflict = conflicts.conflict_at(coord);
                    let _id = ui.push_id(index);

                    ui.table_next_row();
                    ui.table_next_column();
                    let clicked = ui
                        .selectable_config(format!("{}, {}, {}", coord.x, coord.y, coord.z))
                        .selected(self.conflict_rows.contains(&coord))
                        .flags(SelectableFlags::SPAN_ALL_COLUMNS | SelectableFlags::ALLOW_OVERLAP)
                        .build();
                    if clicked && self.conflict_rows.click(ui, &order, position) {
                        actions.push(Action::Center(coord));
                    }
                    self.draw_conflict_menu(ui, &order, coord, &theirs, actions);

                    draw_conflict_details(ui, row, conflict, &status, index, &theirs);

                    draw_resolve_buttons(ui, coord, conflicts, &theirs, actions);
                }
            });
    }

    fn draw_take_buttons(&self, ui: &Ui, rows: &[ConflictRow], conflicts: &ConflictState, actions: &mut Vec<Action>) {
        let selected = rows
            .iter()
            .map(|row| row.coord)
            .filter(|coord| self.conflict_rows.contains(coord))
            .collect::<Vec<_>>();
        let (targets, suffix) = if selected.is_empty() {
            (conflicts.coords().to_vec(), String::from(" for all"))
        } else {
            let count = selected.len();
            (selected, format!(" ({count} selected)"))
        };
        for side in [Side::Ours, Side::Theirs] {
            let label = format!("Take {}{suffix}##take-{side:?}", conflicts.side_label(side));
            if side == Side::Theirs {
                same_line_if_fits(ui, button_width(ui, &label));
            }

            if ui.button(&label) {
                actions.push(Action::Resolve(targets.clone(), side));
            }

            ui.set_item_tooltip(conflicts.side_detail(side));
        }
    }

    fn draw_conflict_menu(&self, ui: &Ui, order: &[Coord], coord: Coord, theirs: &str, actions: &mut Vec<Action>) {
        if let Some(_menu) = ui.begin_popup_context_item() {
            let targets = if self.conflict_rows.contains(&coord) {
                order
                    .iter()
                    .copied()
                    .filter(|coord| self.conflict_rows.contains(coord))
                    .collect()
            } else {
                vec![coord]
            };
            let count = if targets.len() > 1 {
                format!(" ({} tiles)", targets.len())
            } else {
                String::new()
            };
            if ui.menu_item(format!("Take {OURS_LABEL}{count}")) {
                actions.push(Action::Resolve(targets.clone(), Side::Ours));
            }
            if ui.menu_item(format!("Take {theirs}{count}")) {
                actions.push(Action::Resolve(targets, Side::Theirs));
            }
            ui.separator();
            if ui.menu_item("Jump to tile") {
                actions.push(Action::Center(coord));
            }
            if ui.menu_item("Copy coordinates") {
                actions.push(Action::Copy(format!("{}, {}, {}", coord.x, coord.y, coord.z)));
            }
        }
    }
}

fn draw_conflict_progress(ui: &Ui, total: usize, resolved: usize) {
    ui.progress_bar(if total == 0 {
        1.0
    } else {
        resolved as f32 / total as f32
    })
    .overlay_text(format!("{resolved} / {total} resolved"))
    .size([-f32::MIN_POSITIVE, 0.0])
    .build();
}

fn draw_mark_resolved(ui: &Ui, session: &Session, id: DocumentId, git: &GitDocState, actions: &mut Vec<Action>) {
    let blocker = session.mark_resolved_blocker(id);
    let label = if git.staging {
        String::from("Staging...##mark-resolved")
    } else {
        format!("{ICON_CHECK} Mark resolved##mark-resolved")
    };
    align_right(ui, button_width(ui, &label));
    if ui.with_disabled_if(blocker.is_some(), || ui.button(&label)) {
        actions.push(Action::MarkResolved);
    }
    if ui.is_item_hovered_with_flags(ItemHoveredFlags::ALLOW_WHEN_DISABLED) {
        ui.tooltip_text(blocker.as_deref().unwrap_or("Stage the map with git add"));
    }
}

fn draw_conflict_details(
    ui: &Ui, row: &ConflictRow, conflict: Option<&TileConflict>, status: &[Option<Side>], index: usize, theirs: &str,
) {
    ui.table_next_column();
    ui.text(row.hunk.to_string());

    ui.table_next_column();
    if let Some(conflict) = conflict {
        let base = describe_tile(conflict.base.as_ref().map(TileView::from)).replace('\n', ", ");
        ui.text_disabled(&base);
        ui.set_item_tooltip(&base);
    }

    ui.table_next_column();
    let (label, color) = match status[index] {
        None => ("Unresolved", UNRESOLVED_COLOR),
        Some(Side::Ours) => (OURS_LABEL, OURS_COLOR),
        Some(Side::Theirs) => (theirs, THEIRS_COLOR),
    };
    ui.text_colored(opaque(color), label);
}

fn draw_resolve_buttons(ui: &Ui, coord: Coord, conflicts: &ConflictState, theirs: &str, actions: &mut Vec<Action>) {
    ui.table_next_column();
    if ui.small_button(OURS_LABEL) {
        actions.push(Action::Resolve(vec![coord], Side::Ours));
    }
    ui.set_item_tooltip(conflicts.side_detail(Side::Ours));
    ui.same_line();
    if ui.small_button(theirs) {
        actions.push(Action::Resolve(vec![coord], Side::Theirs));
    }
    ui.set_item_tooltip(conflicts.side_detail(Side::Theirs));
}
