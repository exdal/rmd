use base::coord::Coord;
use dear_imgui_rs::{
    Condition, ListClipper, SelectableFlags, StyleColor, TableColumnFlags, TableFlags, TableSizingPolicy, Ui,
    WindowFlags,
};
use ui::{
    icons::materialdesignicons::{ICON_ALERT, ICON_CIRCLE_SMALL, ICON_REFRESH, ICON_SOURCE_COMMIT},
    widgets::{
        button_width, centered_note, dpi, overflow_scroll, same_line_if_fits, table_min_width, text_wrapped_colored,
    },
};

use crate::{
    document::DocumentId,
    git::{
        blame::{
            BOUNDARY, UNCOMMITTED, relative_time,
            session::{BlameState, blame_color},
        },
        repo::{CommitInfo, WebLinks},
        session::GitDocState,
    },
    session::Session,
    settings::Settings,
    viewport::panel::overlay::OverlayRect,
};

use super::{
    Action, GitPanel, commit_summary,
    legend::{draw_legend, draw_swatch},
    unix_now,
};

pub(crate) struct BlamePopup {
    pub(crate) coord: Coord,
    pub(crate) position: [f32; 2],
    pub(crate) bounds: Option<OverlayRect>,
}

pub(crate) fn blame_tooltip_position(ui: &Ui, mouse: [f32; 2]) -> [f32; 2] {
    let display = ui.io().display_size();
    let width = 390.0;
    let height = ui.text_line_height_with_spacing() * 6.0;
    [
        if mouse[0] + width + 12.0 > display[0] {
            (mouse[0] - width - 12.0).max(0.0)
        } else {
            mouse[0] + 12.0
        },
        if mouse[1] + height + 12.0 > display[1] {
            (mouse[1] - height - 12.0).max(0.0)
        } else {
            mouse[1] + 12.0
        },
    ]
}

pub(crate) fn draw_blame_popup(
    ui: &Ui, document: DocumentId, popup: &BlamePopup, commit: &CommitInfo, web: Option<&WebLinks>,
) -> Option<(OverlayRect, bool)> {
    const WIDTH: f32 = 390.0;
    let width = WIDTH * dpi(ui);
    let flags = WindowFlags::NO_DECORATION
        | WindowFlags::NO_MOVE
        | WindowFlags::NO_SAVED_SETTINGS
        | WindowFlags::NO_DOCKING
        | WindowFlags::NO_FOCUS_ON_APPEARING
        | WindowFlags::ALWAYS_AUTO_RESIZE;
    let _background = ui.push_style_color(StyleColor::WindowBg, ui.clone_style().color(StyleColor::PopupBg));
    ui.window(format!("Blame##popup-{}", document.get()))
        .flags(flags)
        .position(popup.position, Condition::Always)
        .size_constraints([0.0, 0.0], [width, f32::MAX])
        .build(|| {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |time| time.as_secs() as i64);
            if let Some(web) = web {
                let url = web.commit(&commit.hash);
                ui.text_link_open_url(&commit.short, &url);
            } else {
                ui.text(&commit.short);
                ui.set_item_tooltip("No web remote is configured for this repository");
            }
            ui.same_line();
            ui.text(format!(
                "{ICON_CIRCLE_SMALL} {} {ICON_CIRCLE_SMALL} {}",
                commit.author,
                crate::git::blame::relative_time(now, commit.time)
            ));
            let padding = ui.clone_style().window_padding()[0];
            commit_summary(ui, commit, web, width - padding * 2.0);
            let close = ui.small_button("Close");
            let min = ui.window_pos();
            let size = ui.window_size();
            (
                OverlayRect {
                    min,
                    max: [min[0] + size[0], min[1] + size[1]],
                },
                close,
            )
        })
}

const STALE_COLOR: [f32; 4] = [1.0, 0.8, 0.3, 1.0];

impl GitPanel {
    pub(super) fn draw_blame(
        &mut self, ui: &Ui, session: &Session, settings: &Settings, id: DocumentId, git: &GitDocState,
        actions: &mut Vec<Action>,
    ) {
        let mut show = git.show_blame;
        if ui.checkbox("Show heatmap", &mut show) {
            actions.push(Action::ToggleBlame);
        }

        let blame = session.blame_state(id);
        draw_blame_button(ui, blame, actions);

        draw_blame_progress(ui, session, settings, id);

        let Some(blame) = blame else {
            if session.blame_progress(id).is_none() {
                centered_note(ui, "Run blame to see which commit last changed each tile");
            }
            return;
        };

        draw_boundary_note(ui, blame);
        draw_legend(ui, blame);
        ui.checkbox("Show all commits", &mut self.show_all_commits);

        let cells = self.blame_cells(blame);
        let web = git.web.as_ref();
        let now = unix_now();
        let swatch = ui.text_line_height();
        let commit_width = ui.calc_text_size("Uncommitted")[0];
        let tiles_width = ui.calc_text_size("000000")[0];
        let z = session.z();
        let (scroll, inner_width) = overflow_scroll(ui, table_min_width(ui, &[swatch, commit_width, tiles_width], 3));

        ui.table("git-blame-commits")
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
            .freeze(2, 1)
            .headers(true)
            .column("##heat")
            .width(swatch)
            .flags(TableColumnFlags::NO_HIDE)
            .done()
            .column("Commit")
            .width(commit_width)
            .flags(TableColumnFlags::NO_HIDE)
            .done()
            .column("Author")
            .weight(0.6)
            .done()
            .column("Date")
            .weight(0.5)
            .done()
            .column("Summary")
            .weight(1.4)
            .done()
            .column("Tiles")
            .width(tiles_width)
            .done()
            .build(|ui| {
                let clipper = ListClipper::new(cells.len()).begin(ui);
                for position in clipper.iter() {
                    let cell = cells[position];
                    let commit = blame.result.commits.get(cell as usize);
                    let tiles = match cell {
                        UNCOMMITTED => blame.counts.uncommitted,
                        BOUNDARY => blame.counts.boundary,
                        index => blame.counts.commits.get(index as usize).copied().unwrap_or(0),
                    };
                    let _id = ui.push_id(position);

                    ui.table_next_row();
                    ui.table_next_column();
                    let origin = ui.cursor_screen_pos();
                    if ui
                        .selectable_config("##blame-row")
                        .selected(blame.pinned == Some(cell))
                        .flags(SelectableFlags::SPAN_ALL_COLUMNS | SelectableFlags::ALLOW_OVERLAP)
                        .build()
                    {
                        actions.push(Action::PinBlame(cell));
                    }
                    ui.set_item_tooltip(match blame.pinned == Some(cell) {
                        true => "Click to stop outlining these tiles",
                        false => "Click to outline these tiles on the map",
                    });
                    draw_swatch(ui, origin, swatch, blame_color(&blame.result, cell));

                    draw_blame_menu(ui, blame, cell, tiles, z, commit, actions);

                    draw_blame_cells(ui, blame, cell, commit, web, now, tiles);
                }
            });
    }

    fn blame_cells(&self, blame: &BlameState) -> Vec<u32> {
        std::iter::once(UNCOMMITTED)
            .filter(|_| blame.counts.uncommitted != 0)
            .chain(
                (0..blame.result.commits.len())
                    .filter(|index| self.show_all_commits || blame.counts.commits[*index] != 0)
                    .map(|index| index as u32),
            )
            .chain(std::iter::once(BOUNDARY).filter(|_| blame.counts.boundary != 0))
            .collect::<Vec<_>>()
    }
}

fn draw_blame_button(ui: &Ui, blame: Option<&BlameState>, actions: &mut Vec<Action>) {
    let label = if blame.is_some() {
        format!("{ICON_REFRESH} Refresh##run-blame")
    } else {
        String::from("Run blame##run-blame")
    };
    same_line_if_fits(ui, button_width(ui, &label));
    if ui.button(label) {
        actions.push(Action::RunBlame);
    }
}

fn draw_blame_progress(ui: &Ui, session: &Session, settings: &Settings, id: DocumentId) {
    if let Some(done) = session.blame_progress(id) {
        let depth = (settings.blame_depth as usize).max(1);
        ui.progress_bar((done as f32 / depth as f32).min(1.0))
            .overlay_text(format!("Walking history: {done} revisions"))
            .size([-f32::MIN_POSITIVE, 0.0])
            .build();
    }
    if session.blame_stale(id) {
        text_wrapped_colored(ui, STALE_COLOR, &format!("{ICON_ALERT} Map changed since blame ran"));
    }
}

fn draw_boundary_note(ui: &Ui, blame: &BlameState) {
    if blame.counts.boundary != 0 || blame.result.parse_boundary.is_some() {
        text_wrapped_colored(
            ui,
            ui.style_color(StyleColor::TextDisabled),
            &blame.result.boundary_label(),
        );
    }
}

fn draw_blame_menu(
    ui: &Ui, blame: &BlameState, cell: u32, tiles: usize, z: u32, commit: Option<&CommitInfo>,
    actions: &mut Vec<Action>,
) {
    if let Some(_menu) = ui.begin_popup_context_item() {
        let pinned = blame.pinned == Some(cell);
        if ui.menu_item(if pinned { "Unpin tiles" } else { "Pin tiles" }) {
            actions.push(Action::PinBlame(cell));
        }
        if ui.menu_item_enabled_selected_no_shortcut("Jump to tiles", false, tiles != 0)
            && let Some(coord) = blame.result.first_tile(cell, z)
        {
            actions.push(Action::Center(coord));
        }
        if let Some(commit) = commit
            && ui.menu_item("Copy hash")
        {
            actions.push(Action::Copy(commit.hash.clone()));
        }
    }
}

fn draw_blame_cells(
    ui: &Ui, blame: &BlameState, cell: u32, commit: Option<&CommitInfo>, web: Option<&WebLinks>, now: i64, tiles: usize,
) {
    ui.table_next_column();
    match (commit, web) {
        (Some(commit), Some(web)) => {
            let url = web.commit(&commit.hash);
            ui.text_link_open_url(&commit.short, &url);
        },
        (Some(commit), None) => {
            ui.text(format!("{ICON_SOURCE_COMMIT} {}", commit.short));
            ui.set_item_tooltip(&commit.summary);
        },
        (None, _) if cell == UNCOMMITTED => ui.text_colored(STALE_COLOR, "Uncommitted"),
        (None, _) => ui.text_disabled("Older"),
    }

    ui.table_next_column();
    if let Some(commit) = commit {
        ui.text(&commit.author);
    }

    ui.table_next_column();
    if let Some(commit) = commit {
        ui.text(relative_time(now, commit.time));
    }

    ui.table_next_column();
    match (commit, cell) {
        (Some(commit), _) => {
            ui.text(&commit.summary);
            ui.set_item_tooltip(&commit.summary);
        },
        (None, UNCOMMITTED) => ui.text_disabled("Changed in the working tree"),
        (None, _) => ui.text_disabled(blame.result.boundary_label()),
    }

    ui.table_next_column();
    ui.text(tiles.to_string());
}
