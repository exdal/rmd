use dear_imgui_rs::{ListClipper, StyleColor, TableColumnFlags, TableFlags, TableRowFlags, TableSizingPolicy, Ui};
use ui::{
    icons::materialdesignicons::{ICON_CIRCLE_SMALL, ICON_FILE_COMPARE, ICON_PENCIL},
    widgets::{button_width, dpi},
};

use crate::git::{
    blame::relative_time,
    diff::{DiffSource, session::DiffSide},
    session::GitDocState,
};

use super::{Action, unix_now};

const VERSION_ROW_PADDING: f32 = 4.0;
const BADGE_PADDING: f32 = 4.0;
const COMPARED_ROW_ALPHA_SCALE: f32 = 0.5;
const VERSIONS_MIN_ROWS: f32 = 3.0;

/// The working map and the commits that changed the map, each with buttons to compare it
pub(super) fn draw_versions(
    ui: &Ui, git: &GitDocState, share: f32, reveal: &mut Option<usize>, actions: &mut Vec<Action>,
) {
    let style = ui.clone_style();
    let history = git.history.as_deref().unwrap_or_default();
    let row_height = ui.text_line_height() * 2.0 + style.item_spacing()[1] + VERSION_ROW_PADDING * dpi(ui) * 2.0;
    let header = ui.text_line_height() + style.cell_padding()[1] * 2.0;
    let rows = history.len() + 1;
    let content = header + (row_height + style.cell_padding()[1] * 2.0) * rows as f32;
    let available = ui.content_region_avail()[1];
    let height = content.min((available * share).max(header + row_height * VERSIONS_MIN_ROWS));
    let changes = format!("{ICON_FILE_COMPARE} Changes");
    let working = format!("{ICON_PENCIL} vs Working");
    let buttons_width = button_width(ui, &changes).max(button_width(ui, &working));
    let web = git.web.as_ref();
    let now = unix_now();
    // FROM or TO for the versions in the loaded comparison
    let role = |matches: &dyn Fn(&DiffSide) -> bool| {
        let diff = git.diff.as_ref()?;

        if matches(&diff.to) {
            Some("TO")
        } else if matches(&diff.from) {
            Some("FROM")
        } else {
            None
        }
    };

    ui.table("git-diff-versions")
        .flags(TableFlags::ROW_BG | TableFlags::BORDERS_OUTER | TableFlags::BORDERS_INNER_V | TableFlags::SCROLL_Y)
        .sizing_policy(TableSizingPolicy::StretchProp)
        .outer_size([0.0, height])
        .freeze(0, 1)
        .headers(true)
        .column("Version")
        .flags(TableColumnFlags::NO_HIDE)
        .done()
        .column("##compare")
        .width(buttons_width)
        .flags(TableColumnFlags::NO_HIDE)
        .done()
        .build(|ui| {
            let mut clipper = ListClipper::new(rows).begin(ui);
            let target = reveal.take().filter(|&target| target < rows);
            if let Some(target) = target {
                clipper.include_item_by_index(target);
            }

            for position in clipper.iter() {
                let _id = ui.push_id(position);
                ui.table_next_row_with_flags(TableRowFlags::NONE, row_height);

                // what the row's Changes button compares against, None for the working map
                let (role, title, commit, previous) = match position.checked_sub(1) {
                    None => (
                        role(&|side| side.is_working()),
                        String::from("Working map"),
                        None,
                        DiffSource::head(),
                    ),
                    Some(index) => {
                        let commit = &history[index];
                        let previous = history
                            .get(index + 1)
                            .map_or_else(|| format!("{}~1", commit.hash), |older| older.hash.clone());

                        (
                            role(&|side| side.commit.as_ref().is_some_and(|side| side.hash == commit.hash)),
                            commit.summary.clone(),
                            Some(commit),
                            DiffSource::Revision(previous),
                        )
                    },
                };
                if role.is_some() {
                    let mut color = ui.style_color(StyleColor::Header);
                    color[3] *= COMPARED_ROW_ALPHA_SCALE;
                    ui.table_set_row_bg0_color(color);
                    ui.table_set_row_bg1_color(color);
                }

                ui.table_next_column();
                pad_row(ui);
                if let Some(role) = role {
                    draw_badge(ui, role);
                    ui.same_line();
                }
                ui.text(&title);
                match commit {
                    Some(commit) => {
                        let details = format!(
                            "{ICON_CIRCLE_SMALL} {} {ICON_CIRCLE_SMALL} {}",
                            commit.author,
                            relative_time(now, commit.time)
                        );
                        match web {
                            Some(web) => {
                                ui.text_link_open_url(&commit.short, web.commit(&commit.hash));
                            },
                            None => ui.text_disabled(&commit.short),
                        }
                        ui.same_line();
                        ui.text_disabled(&details);
                    },
                    None => ui.text_disabled("Unsaved and uncommitted edits"),
                }

                if target == Some(position) {
                    ui.set_scroll_here_y(0.5);
                }

                ui.table_next_column();
                pad_row(ui);
                let current = commit.map_or(DiffSource::Working, |commit| DiffSource::Revision(commit.hash.clone()));
                if ui.small_button(&changes) {
                    actions.push(Action::RunDiff(previous, current.clone()));
                    *reveal = Some(position);
                }

                ui.set_item_tooltip(match commit {
                    Some(_) => "What this commit changed",
                    None => "What changed since HEAD",
                });
                if commit.is_some() {
                    if ui.small_button(&working) {
                        actions.push(Action::RunDiff(current, DiffSource::Working));
                        *reveal = Some(position);
                    }

                    ui.set_item_tooltip("Compare with the working map, tiles can be restored from it");
                }
            }
        });

    match &git.history {
        None => ui.text_disabled("Reading history..."),
        Some(history) if history.is_empty() => ui.text_disabled("No commits changed this map"),
        Some(_) => {},
    }
}

/// Text on a pill of the style's active header colour, which the style already pairs with its text
fn draw_badge(ui: &Ui, text: &str) {
    let padding = BADGE_PADDING * dpi(ui);
    let size = ui.calc_text_size(text);
    let origin = ui.cursor_screen_pos();
    ui.get_window_draw_list()
        .add_rect(
            origin,
            [origin[0] + size[0] + padding * 2.0, origin[1] + size[1]],
            ui.style_color(StyleColor::HeaderActive),
        )
        .filled(true)
        .rounding(ui.clone_style().frame_rounding().max(2.0 * dpi(ui)))
        .build();
    ui.set_cursor_screen_pos([origin[0] + padding, origin[1]]);
    ui.text(text);
    // the next item starts past the pill, not the text
    ui.same_line_with_spacing(0.0, padding);
    ui.dummy([0.0, size[1]]);
}

/// Keeps the two lines of a version row off its borders
fn pad_row(ui: &Ui) {
    let [x, y] = ui.cursor_pos();
    ui.set_cursor_pos([x, y + VERSION_ROW_PADDING * dpi(ui)]);
}
