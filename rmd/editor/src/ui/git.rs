use std::collections::HashSet;

use dear_imgui_rs::{
    ItemHoveredFlags,
    ListClipper,
    SelectableFlags,
    StyleColor,
    TabItemFlags,
    TableColumnFlags,
    TableFlags,
    TableRowFlags,
    TableSizingPolicy,
    TreeNodeFlags,
    Ui,
    WindowKey,
    WindowKeyError,
};
use dmm::Coord;
use editor::{
    blame::{self, BOUNDARY, UNCOMMITTED},
    conflict::{OURS_COLOR, OURS_LABEL, Side, THEIRS_COLOR, UNRESOLVED_COLOR, describe_tile},
    diff::{ChangeKind, DiffSource},
    document::DocumentId,
    git::{CommitInfo, WebLinks},
    icons::materialdesignicons::{
        ICON_ALERT,
        ICON_ALERT_CIRCLE,
        ICON_ARROW_RIGHT,
        ICON_CHECK,
        ICON_CIRCLE_SMALL,
        ICON_CLOSE_THICK,
        ICON_FILE_COMPARE,
        ICON_PENCIL,
        ICON_REFRESH,
        ICON_RESTORE,
        ICON_SOURCE_BRANCH,
        ICON_SOURCE_COMMIT,
        ICON_SOURCE_MERGE,
    },
};

use super::{
    SAVE_ERROR_COLOR,
    common::{
        align_right,
        button_width,
        centered_note,
        checkbox_width,
        focus_window_on_hover,
        label_width,
        opaque,
        overflow_scroll,
        same_line_if_fits,
        table_min_width,
        text_wrapped_colored,
    },
};
use crate::{
    session::{self, BlameState, DiffSide, GitDocState, Session},
    settings::Settings,
};

const STALE_COLOR: [f32; 4] = [1.0, 0.8, 0.3, 1.0];
const LEGEND_STEPS: usize = 32;
const LEGEND_WIDTH: f32 = 140.0;
const LEGEND_MIN_WIDTH: f32 = 48.0;
const VERSION_ROW_PADDING: f32 = 4.0;
const BADGE_PADDING: f32 = 4.0;
const COMPARED_ROW_ALPHA_SCALE: f32 = 0.5;
const VERSIONS_MIN_ROWS: f32 = 3.0;
const VERSIONS_HEIGHT_SHARE: f32 = 0.4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GitTab {
    Conflicts,
    Blame,
    Diff,
}

#[derive(Default)]
pub(super) struct GitPanelOutput {
    pub center: Option<(DocumentId, Coord)>,
    pub load_conflicts: Option<DocumentId>,
    pub copy: Option<String>,
}

/// Something the panel asked for while it was reading the session
enum Action {
    Resolve(Vec<Coord>, Side),
    Center(Coord),
    MarkResolved,
    LoadConflicts,
    Refresh,
    ClearError,
    ToggleBlame,
    RunBlame,
    PinBlame(u32),
    Copy(String),
    LoadHistory,
    RunDiff(DiffSource, DiffSource),
    ToggleDiff,
    Restore(Vec<Coord>),
}

/// Selected table rows, keyed by tile
#[derive(Default)]
struct RowSelection {
    tiles: HashSet<Coord>,
    anchor: Option<Coord>,
}

impl RowSelection {
    fn contains(&self, coord: &Coord) -> bool { self.tiles.contains(coord) }

    fn clear(&mut self) {
        self.tiles.clear();
        self.anchor = None;
    }

    /// The selected rows among `order`, in its order
    fn among(&self, order: impl IntoIterator<Item = Coord>) -> Vec<Coord> {
        order.into_iter().filter(|coord| self.tiles.contains(coord)).collect()
    }

    /// Updates the selection like a list view, returns whether the view should follow the row
    fn click(&mut self, ui: &Ui, order: &[Coord], position: usize) -> bool {
        let coord = order[position];
        let io = ui.io();
        let anchor = self
            .anchor
            .and_then(|anchor| order.iter().position(|candidate| *candidate == anchor));

        if io.key_shift()
            && let Some(anchor) = anchor
        {
            if !io.key_ctrl() {
                self.tiles.clear();
            }
            self.tiles.extend(&order[anchor.min(position)..=anchor.max(position)]);
            false
        } else if io.key_ctrl() {
            if !self.tiles.remove(&coord) {
                self.tiles.insert(coord);
            }
            self.anchor = Some(coord);
            false
        } else {
            self.tiles.clear();
            self.tiles.insert(coord);
            self.anchor = Some(coord);
            true
        }
    }
}

pub(super) struct GitPanel {
    window: WindowKey,
    focus_requested: bool,
    select_tab: Option<GitTab>,
    conflict_rows: RowSelection,
    diff_rows: RowSelection,
    selection_document: Option<DocumentId>,
    unresolved_only: bool,
    current_level_only: bool,
    diff_level_only: bool,
    /// The typed revisions under "Compare other revisions"
    custom_from: String,
    custom_to: String,
    show_all_commits: bool,
    /// The version row whose comparison was just started, scrolled back into view once the results
    /// shrink the table
    reveal_version: Option<usize>,
    /// Whether the window's contents ran last frame, false while another tab covers it
    visible: bool,
}

impl GitPanel {
    pub fn new() -> Result<Self, WindowKeyError> {
        Ok(Self {
            window: WindowKey::new("git-panel", "Git")?,
            focus_requested: false,
            select_tab: None,
            conflict_rows: RowSelection::default(),
            diff_rows: RowSelection::default(),
            selection_document: None,
            unresolved_only: false,
            current_level_only: false,
            diff_level_only: false,
            custom_from: String::new(),
            custom_to: String::new(),
            show_all_commits: false,
            reveal_version: None,
            visible: false,
        })
    }

    pub const fn window(&self) -> &WindowKey { &self.window }

    /// Brings the panel to the front of its dock node
    pub fn focus(&mut self) { self.focus_requested = true; }

    pub fn request(&mut self, tab: GitTab) {
        self.focus_requested = true;
        self.select_tab = Some(tab);
    }

    pub fn has_focus_request(&self) -> bool { self.focus_requested }

    #[cfg(test)]
    pub fn visible(&self) -> bool { self.visible }

    pub fn draw(&mut self, ui: &Ui, session: &mut Session, settings: &Settings) -> GitPanelOutput {
        let mut output = GitPanelOutput::default();
        let mut actions = Vec::new();
        let active = session.state.active();

        if self.selection_document != active {
            self.conflict_rows.clear();
            self.diff_rows.clear();
            self.selection_document = active;
        }

        let unresolved = active.map_or(0, |id| session.unresolved_conflicts(id));
        let title = match unresolved {
            0 => String::from("Git"),
            count => format!("Git ({count})"),
        };
        let focus = std::mem::take(&mut self.focus_requested);
        let window = self.window.clone();

        self.visible = false;
        ui.window(window.label(title)).focused(focus).build(|| {
            self.visible = true;
            if settings.focus_windows_on_hover {
                focus_window_on_hover(ui);
            }
            self.draw_contents(ui, session, settings, &mut actions);
        });

        let Some(id) = active else {
            return output;
        };
        for action in actions {
            match action {
                Action::Resolve(coords, side) => {
                    session.resolve_conflict(id, &coords, side);
                },
                Action::Center(coord) => output.center = Some((id, coord)),
                Action::MarkResolved => {
                    session.mark_resolved(id);
                },
                Action::LoadConflicts => output.load_conflicts = Some(id),
                Action::Refresh => session.refresh_git(),
                Action::ClearError => session.clear_git_error(id),
                Action::ToggleBlame => session.toggle_blame(id, settings.blame_depth as usize),
                Action::RunBlame => session.run_blame(id, settings.blame_depth as usize),
                Action::PinBlame(cell) => session.pin_blame(id, cell),
                Action::Copy(text) => output.copy = Some(text),
                Action::LoadHistory => session.load_history(id, settings.blame_depth as usize),
                Action::RunDiff(from, to) => session.run_diff(id, from, to),
                Action::ToggleDiff => session.toggle_diff(id),
                Action::Restore(coords) => {
                    session.restore_diff(id, &coords);
                },
            }
        }

        output
    }

    fn draw_contents(&mut self, ui: &Ui, session: &Session, settings: &Settings, actions: &mut Vec<Action>) {
        if !settings.git_enabled {
            centered_note(ui, "Git integration is disabled in Settings");
            return;
        }
        let Some(id) = session.state.active() else {
            centered_note(ui, "No map selected");
            return;
        };
        let Some(git) = session.git_state(id) else {
            centered_note(ui, "This map is outside a Git worktree");
            return;
        };

        draw_header(ui, git, actions);

        let unresolved = session.unresolved_conflicts(id);
        let select = self.select_tab.take();
        let flags = |tab| {
            if select == Some(tab) {
                TabItemFlags::SET_SELECTED
            } else {
                TabItemFlags::NONE
            }
        };

        if let Some(_bar) = ui.tab_bar("git-panel-tabs") {
            let label = match unresolved {
                0 => String::from("Conflicts###conflicts"),
                count => format!("Conflicts ({count})###conflicts"),
            };
            if let Some(_tab) = ui.tab_item_with_flags(label, None, flags(GitTab::Conflicts)) {
                self.draw_conflicts(ui, session, id, git, actions);
            }
            if let Some(_tab) = ui.tab_item_with_flags("Blame###blame", None, flags(GitTab::Blame)) {
                self.draw_blame(ui, session, settings, id, git, actions);
            }
            if let Some(_tab) = ui.tab_item_with_flags("Diff###diff", None, flags(GitTab::Diff)) {
                self.draw_diff(ui, session, id, git, actions);
            }
        }
    }

    fn draw_conflicts(
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

        ui.progress_bar(if total == 0 {
            1.0
        } else {
            resolved as f32 / total as f32
        })
        .overlay_text(format!("{resolved} / {total} resolved"))
        .size([-f32::MIN_POSITIVE, 0.0])
        .build();

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
            .column("Region")
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

                    ui.table_next_column();
                    ui.text(row.region.to_string());

                    ui.table_next_column();
                    if let Some(conflict) = conflict {
                        let base = describe_tile(conflict.base.as_ref()).replace('\n', ", ");
                        ui.text_disabled(&base);
                        ui.set_item_tooltip(&base);
                    }

                    ui.table_next_column();
                    let (label, color) = match status[index] {
                        None => ("Unresolved", UNRESOLVED_COLOR),
                        Some(Side::Ours) => (OURS_LABEL, OURS_COLOR),
                        Some(Side::Theirs) => (theirs.as_str(), THEIRS_COLOR),
                    };
                    ui.text_colored(opaque(color), label);

                    ui.table_next_column();
                    if ui.small_button(OURS_LABEL) {
                        actions.push(Action::Resolve(vec![coord], Side::Ours));
                    }
                    ui.set_item_tooltip(conflicts.side_detail(Side::Ours));
                    ui.same_line();
                    if ui.small_button(&theirs) {
                        actions.push(Action::Resolve(vec![coord], Side::Theirs));
                    }
                    ui.set_item_tooltip(conflicts.side_detail(Side::Theirs));
                }
            });
    }

    fn draw_blame(
        &mut self, ui: &Ui, session: &Session, settings: &Settings, id: DocumentId, git: &GitDocState,
        actions: &mut Vec<Action>,
    ) {
        let mut show = git.show_blame;
        if ui.checkbox("Show heatmap", &mut show) {
            actions.push(Action::ToggleBlame);
        }

        let blame = session.blame_state(id);
        let label = if blame.is_some() {
            format!("{ICON_REFRESH} Refresh##run-blame")
        } else {
            String::from("Run blame##run-blame")
        };
        same_line_if_fits(ui, button_width(ui, &label));
        if ui.button(label) {
            actions.push(Action::RunBlame);
        }

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

        let Some(blame) = blame else {
            if session.blame_progress(id).is_none() {
                centered_note(ui, "Run blame to see which commit last changed each tile");
            }
            return;
        };

        if blame.counts.boundary != 0 || blame.result.parse_boundary.is_some() {
            text_wrapped_colored(
                ui,
                ui.style_color(StyleColor::TextDisabled),
                &blame.result.boundary_label(),
            );
        }
        draw_legend(ui, blame);
        ui.checkbox("Show all commits", &mut self.show_all_commits);

        let cells = std::iter::once(UNCOMMITTED)
            .filter(|_| blame.counts.uncommitted != 0)
            .chain(
                (0..blame.result.commits.len())
                    .filter(|index| self.show_all_commits || blame.counts.commits[*index] != 0)
                    .map(|index| index as u32),
            )
            .chain(std::iter::once(BOUNDARY).filter(|_| blame.counts.boundary != 0))
            .collect::<Vec<_>>();
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
                    draw_swatch(ui, origin, swatch, session::blame_color(&blame.result, cell));

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
                        ui.text(blame::relative_time(now, commit.time));
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
            });
    }

    fn draw_diff(&mut self, ui: &Ui, session: &Session, id: DocumentId, git: &GitDocState, actions: &mut Vec<Action>) {
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

        let (Some(state), Some(diff)) = (git.diff.as_ref(), session.diff(id)) else {
            return;
        };

        ui.spacing();
        let spacing = ui.clone_style().item_spacing()[0];
        let comparing = format!("{} {ICON_ARROW_RIGHT} {}", state.from.label(), state.to.label());
        ui.text(&comparing);
        let label = "Show on map";
        let mut show = git.show_diff;
        align_right(ui, checkbox_width(ui, label));
        if ui.checkbox(label, &mut show) {
            actions.push(Action::ToggleDiff);
        }

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

        ui.checkbox("Current level only", &mut self.diff_level_only);
        let label = format!("{ICON_RESTORE} Restore from {from}{suffix}##restore-diff");
        align_right(ui, button_width(ui, &label));
        if ui.with_disabled_if(!restorable || targets.is_empty(), || ui.button(&label)) {
            actions.push(Action::Restore(targets));
        }
        if !restorable && ui.is_item_hovered_with_flags(ItemHoveredFlags::ALLOW_WHEN_DISABLED) {
            ui.tooltip_text("Tiles can only be restored onto the working map, pick it as To");
        }

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
            .column("Region")
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
                        if ui.menu_item_enabled_selected_no_shortcut(
                            format!("Restore from {from}{count}"),
                            false,
                            restorable,
                        ) {
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

                    ui.table_next_column();
                    ui.text(change.region.to_string());

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

                    ui.table_next_column();
                    if restorable && ui.small_button(restore) {
                        actions.push(Action::Restore(vec![coord]));
                    }
                }
            });
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

/// The working map and the commits that changed the map, each with buttons to compare it
fn draw_versions(ui: &Ui, git: &GitDocState, share: f32, reveal: &mut Option<usize>, actions: &mut Vec<Action>) {
    let style = ui.clone_style();
    let history = git.history.as_deref().unwrap_or_default();
    let row_height = ui.text_line_height() * 2.0 + style.item_spacing()[1] + VERSION_ROW_PADDING * 2.0;
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
                            blame::relative_time(now, commit.time)
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
    let size = ui.calc_text_size(text);
    let origin = ui.cursor_screen_pos();
    ui.get_window_draw_list()
        .add_rect(
            origin,
            [origin[0] + size[0] + BADGE_PADDING * 2.0, origin[1] + size[1]],
            ui.style_color(StyleColor::HeaderActive),
        )
        .filled(true)
        .rounding(ui.clone_style().frame_rounding().max(2.0))
        .build();
    ui.set_cursor_screen_pos([origin[0] + BADGE_PADDING, origin[1]]);
    ui.text(text);
    // the next item starts past the pill, not the text
    ui.same_line_with_spacing(0.0, BADGE_PADDING);
    ui.dummy([0.0, size[1]]);
}

/// Keeps the two lines of a version row off its borders
fn pad_row(ui: &Ui) {
    let [x, y] = ui.cursor_pos();
    ui.set_cursor_pos([x, y + VERSION_ROW_PADDING]);
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |time| time.as_secs() as i64)
}

fn draw_header(ui: &Ui, git: &GitDocState, actions: &mut Vec<Action>) {
    let refresh_size = ui.frame_height();
    let reserve = ui.clone_style().item_spacing()[0] + refresh_size;

    let branch = git.branch.as_deref().unwrap_or("Reading repository...");
    ui.text(format!("{ICON_SOURCE_BRANCH} {branch}"));
    if let Some(head) = &git.head {
        let text = format!("HEAD {}", head.short);
        same_line_if_fits(ui, label_width(ui, &text) + reserve);
        ui.text_disabled(&text);
        ui.set_item_tooltip(&head.subject);
    }

    same_line_if_fits(ui, label_width(ui, &git.repo.rel) + reserve);
    ui.text_disabled(&git.repo.rel);
    ui.set_item_tooltip(&git.repo.rel);

    let refresh = format!("{ICON_REFRESH}##refresh-git");
    align_right(ui, refresh_size);
    if ui.button_with_size(&refresh, [refresh_size, refresh_size]) {
        actions.push(Action::Refresh);
    }

    ui.set_item_tooltip("Refresh Git status");

    if let Some(operation) = &git.operation {
        text_wrapped_colored(
            ui,
            opaque(THEIRS_COLOR),
            &format!(
                "{ICON_SOURCE_MERGE} {} {}",
                operation.kind.label(),
                operation.theirs.describe()
            ),
        );
    }

    if git.pending_load {
        ui.text_colored(
            opaque(UNRESOLVED_COLOR),
            format!("{ICON_ALERT} Merge conflicts on disk"),
        );
        let label = "Load conflicts";
        same_line_if_fits(ui, button_width(ui, label));
        if ui.small_button(label) {
            actions.push(Action::LoadConflicts);
        }
    }

    if let Some(error) = &git.error {
        if ui.small_button(format!("{ICON_CLOSE_THICK}##dismiss-git-error")) {
            actions.push(Action::ClearError);
        }

        ui.set_item_tooltip("Dismiss");
        ui.same_line();
        text_wrapped_colored(ui, SAVE_ERROR_COLOR, &format!("{ICON_ALERT_CIRCLE} {error}"));
    }

    ui.spacing();
}

fn draw_legend(ui: &Ui, blame: &BlameState) {
    let height = ui.text_line_height();
    let spacing = ui.clone_style().item_spacing()[0];
    let span = blame.result.heat_span;

    if span != 0 {
        let labels = label_width(ui, "Newest") + label_width(ui, "Oldest") + spacing * 2.0;
        let width = LEGEND_WIDTH
            .min(ui.content_region_avail_width() - labels)
            .max(LEGEND_MIN_WIDTH);
        ui.text_disabled("Newest");
        ui.same_line();
        let origin = ui.cursor_screen_pos();
        let step = width / LEGEND_STEPS as f32;
        for index in 0..LEGEND_STEPS {
            let cell = (index * span.saturating_sub(1) / (LEGEND_STEPS - 1)) as u32;
            let color = session::blame_color(&blame.result, cell);
            let min = [origin[0] + step * index as f32, origin[1]];
            ui.get_window_draw_list()
                .add_rect(min, [min[0] + step + 0.5, min[1] + height], opaque(color))
                .filled(true)
                .build();
        }
        ui.dummy([width, height]);
        ui.same_line();
        ui.text_disabled("Oldest");
    }

    for (cell, label) in [(UNCOMMITTED, "Uncommitted"), (BOUNDARY, "Older")] {
        if span != 0 || cell != UNCOMMITTED {
            same_line_if_fits(ui, height + spacing + label_width(ui, label));
        }

        let origin = ui.cursor_screen_pos();
        draw_swatch(ui, origin, height, session::blame_color(&blame.result, cell));
        ui.dummy([height, height]);
        ui.same_line();
        ui.text_disabled(label);
    }
}

fn draw_swatch(ui: &Ui, origin: [f32; 2], size: f32, color: [f32; 3]) {
    let inset = (size * 0.15).round();
    ui.get_window_draw_list()
        .add_rect(
            [origin[0] + inset, origin[1] + inset],
            [origin[0] + size - inset, origin[1] + size - inset],
            opaque(color),
        )
        .filled(true)
        .rounding(2.0)
        .build();
}

pub(super) fn commit_summary(ui: &Ui, commit: &CommitInfo, web: Option<&WebLinks>, width: f32) {
    let (Some(number), Some(web)) = (commit.pull_request, web) else {
        ui.text_wrapped(&commit.summary);

        return;
    };

    let label = format!("(#{number})");
    let title = commit.summary.trim_end();
    let title = title.strip_suffix(&label).map_or(title, str::trim_end);
    let url = web.pull_request(number);
    let spacing = ui.clone_style().item_spacing()[0];
    if ui.calc_text_size(title)[0] + spacing + ui.calc_text_size(&label)[0] <= width {
        ui.text(title);
        ui.same_line();
    } else {
        // same_line after wrapped text lands beside its first line, so the link gets its own
        ui.text_wrapped(title);
    }

    ui.text_link_open_url(&label, &url);
}
