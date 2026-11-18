use base::coord::Coord;
use dear_imgui_rs::{
    ItemHoveredFlags, ListClipper, SelectableFlags, TableColumnFlags, TableFlags, TableSizingPolicy, TreeNodeFlags, Ui,
};
use ui::{
    icons::materialdesignicons::{ICON_ARROW_RIGHT, ICON_FILE_COMPARE, ICON_RESTORE},
    widgets::{
        align_right, button_width, centered_note, checkbox_width, label_width, opaque, overflow_scroll,
        same_line_if_fits, table_min_width,
    },
};

use crate::{
    document::DocumentId,
    git::{
        conflict::describe_tile,
        diff::{ChangeKind, ChangedTile, DiffSource, MapDiff, session::DiffState},
        session::GitDocState,
    },
    session::Session,
};

use super::{Action, GitPanel, legend::draw_swatch, versions::draw_versions};

const VERSIONS_HEIGHT_SHARE: f32 = 0.4;

impl GitPanel {
    pub(super) fn draw_diff(
        &mut self, ui: &Ui, session: &Session, id: DocumentId, git: &GitDocState, actions: &mut Vec<Action>,
    ) {
        self.draw_revisions(ui, git, actions);

        let (Some(state), Some(diff)) = (git.diff.as_ref(), session.diff(id)) else {
            return;
        };

        ui.spacing();
        let spacing = ui.clone_style().item_spacing()[0];
        draw_diff_header(ui, state, git, actions);

        draw_change_counts(ui, &diff, spacing);

        if diff.is_empty() {
            centered_note(ui, "No differences");

            return;
        }

        let z = session.z();
        let visible = diff
            .changes()
            .iter()
            .filter(|change| !self.diff_level_only || change.coord.z == z)
            .collect::<Vec<_>>();
        let order = visible.iter().map(|change| change.coord).collect::<Vec<_>>();
        let selected = self.diff_rows.among(order.iter().copied());
        let from = state.from.label();
        let restorable = state.restorable();
        let (targets, suffix) = if selected.is_empty() {
            (order.clone(), String::from(" for all"))
        } else {
            let count = selected.len();
            (selected, format!(" ({count} selected)"))
        };

        self.draw_restore_button(ui, targets, &from, &suffix, restorable, actions);

        let tile_width = ui.calc_text_size("000, 000, 00")[0];
        let restore = "Restore";
        let restore_width = button_width(ui, restore);
        let (scroll, inner_width) = overflow_scroll(ui, table_min_width(ui, &[tile_width, restore_width], 2));

        ui.table("git-diff")
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
            .weight(0.6)
            .done()
            .column("Change")
            .weight(0.8)
            .done()
            .column("From")
            .weight(1.5)
            .flags(TableColumnFlags::DEFAULT_HIDE)
            .done()
            .column("To")
            .weight(1.5)
            .flags(TableColumnFlags::DEFAULT_HIDE)
            .done()
            .column("##restore")
            .width(restore_width)
            .flags(TableColumnFlags::NO_HIDE)
            .done()
            .build(|ui| {
                let clipper = ListClipper::new(visible.len()).begin(ui);
                for position in clipper.iter() {
                    let change = visible[position];
                    let coord = change.coord;
                    let _id = ui.push_id(position);

                    ui.table_next_row();
                    ui.table_next_column();
                    let clicked = ui
                        .selectable_config(format!("{}, {}, {}", coord.x, coord.y, coord.z))
                        .selected(self.diff_rows.contains(&coord))
                        .flags(SelectableFlags::SPAN_ALL_COLUMNS | SelectableFlags::ALLOW_OVERLAP)
                        .build();
                    if clicked && self.diff_rows.click(ui, &order, position) {
                        actions.push(Action::Center(coord));
                    }
                    self.draw_diff_menu(ui, &order, coord, &from, restorable, actions);

                    draw_change_cells(ui, session, id, change, coord);

                    draw_restore_cell(ui, restorable, restore, coord, actions);
                }
            });
    }

    fn draw_revisions(&mut self, ui: &Ui, git: &GitDocState, actions: &mut Vec<Action>) {
        if git.history.is_none() {
            actions.push(Action::LoadHistory);
        }

        self.draw_other_revisions(ui, actions);
        // the results share the panel once there are any
        let share = if git.diff.is_some() || git.diff_loading {
            VERSIONS_HEIGHT_SHARE
        } else {
            1.0
        };
        draw_versions(ui, git, share, &mut self.reveal_version, actions);

        if git.diff_loading {
            ui.text_disabled("Comparing...");
        }
    }

    fn draw_restore_button(
        &mut self, ui: &Ui, targets: Vec<Coord>, from: &str, suffix: &str, restorable: bool, actions: &mut Vec<Action>,
    ) {
        ui.checkbox("Current level only", &mut self.diff_level_only);
        let label = format!("{ICON_RESTORE} Restore from {from}{suffix}##restore-diff");
        align_right(ui, button_width(ui, &label));
        if ui.with_disabled_if(!restorable || targets.is_empty(), || ui.button(&label)) {
            actions.push(Action::Restore(targets));
        }
        if !restorable && ui.is_item_hovered_with_flags(ItemHoveredFlags::ALLOW_WHEN_DISABLED) {
            ui.tooltip_text("Tiles can only be restored onto the working map, pick it as To");
        }
    }

    fn draw_diff_menu(
        &self, ui: &Ui, order: &[Coord], coord: Coord, from: &str, restorable: bool, actions: &mut Vec<Action>,
    ) {
        if let Some(_menu) = ui.begin_popup_context_item() {
            let targets = if self.diff_rows.contains(&coord) {
                self.diff_rows.among(order.iter().copied())
            } else {
                vec![coord]
            };
            let count = if targets.len() > 1 {
                format!(" ({} tiles)", targets.len())
            } else {
                String::new()
            };
            if ui.menu_item_enabled_selected_no_shortcut(format!("Restore from {from}{count}"), false, restorable) {
                actions.push(Action::Restore(targets));
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

    /// Typed revisions for anything the history doesn't list, like another branch
    fn draw_other_revisions(&mut self, ui: &Ui, actions: &mut Vec<Action>) {
        if !ui.collapsing_header("Compare other revisions##diff-custom", TreeNodeFlags::empty()) {
            return;
        }

        let spacing = ui.clone_style().item_spacing()[0];
        let label_end = ui.cursor_pos()[0] + label_width(ui, "From").max(label_width(ui, "To")) + spacing;
        let mut submitted = false;
        for (label, input, hint) in [
            ("From", &mut self.custom_from, "HEAD~5, a branch, tag or hash"),
            ("To", &mut self.custom_to, "Empty for the working map"),
        ] {
            ui.align_text_to_frame_padding();
            ui.text(label);
            ui.same_line_with_pos(label_end);
            ui.set_next_item_width(-f32::MIN_POSITIVE);
            submitted |= ui
                .input_text(format!("##custom-{label}"), input)
                .hint(hint)
                .enter_returns_true(true)
                .build();
        }

        let from = self.custom_from.trim();
        let to = self.custom_to.trim();
        let label = format!("{ICON_FILE_COMPARE} Compare##custom-diff");
        align_right(ui, button_width(ui, &label));
        let clicked = ui.with_disabled_if(from.is_empty(), || ui.button(&label));
        if (clicked || submitted) && !from.is_empty() {
            let to = match to {
                "" => DiffSource::Working,
                to => DiffSource::Revision(to.to_owned()),
            };
            actions.push(Action::RunDiff(DiffSource::Revision(from.to_owned()), to));
        }

        ui.spacing();
    }
}

fn draw_diff_header(ui: &Ui, state: &DiffState, git: &GitDocState, actions: &mut Vec<Action>) {
    let comparing = format!("{} {ICON_ARROW_RIGHT} {}", state.from.label(), state.to.label());
    ui.text(&comparing);
    let label = "Show on map";
    let mut show = git.show_diff;
    align_right(ui, checkbox_width(ui, label));
    if ui.checkbox(label, &mut show) {
        actions.push(Action::ToggleDiff);
    }
}

fn draw_change_counts(ui: &Ui, diff: &MapDiff, spacing: f32) {
    let height = ui.text_line_height();
    for (index, kind) in ChangeKind::ALL.into_iter().enumerate() {
        let text = format!("{} {}", diff.count(kind), kind.label().to_lowercase());
        if index != 0 {
            same_line_if_fits(ui, height + spacing + label_width(ui, &text));
        }

        draw_swatch(ui, ui.cursor_screen_pos(), height, kind.color());
        ui.dummy([height, height]);
        ui.same_line();
        ui.text(&text);
    }
}

fn draw_change_cells(ui: &Ui, session: &Session, id: DocumentId, change: &ChangedTile, coord: Coord) {
    ui.table_next_column();
    ui.text(change.hunk.to_string());

    ui.table_next_column();
    ui.text_colored(opaque(change.kind.color()), change.kind.label());

    let (before, after) = session
        .diff_at(id, coord)
        .map_or((None, None), |(_, before, after)| (before, after));
    for tile in [before, after] {
        ui.table_next_column();
        let text = describe_tile(tile).replace('\n', ", ");
        ui.text_disabled(&text);
        ui.set_item_tooltip(&text);
    }
}

fn draw_restore_cell(ui: &Ui, restorable: bool, restore: &str, coord: Coord, actions: &mut Vec<Action>) {
    ui.table_next_column();
    if restorable && ui.small_button(restore) {
        actions.push(Action::Restore(vec![coord]));
    }
}
