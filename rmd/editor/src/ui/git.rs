use std::collections::HashSet;

use dear_imgui_rs::{
    ItemHoveredFlags,
    ListClipper,
    SelectableFlags,
    TabItemFlags,
    TableColumnFlags,
    TableFlags,
    TableSizingPolicy,
    Ui,
    WindowKey,
    WindowKeyError,
};
use dmm::Coord;
use editor::{
    blame::{self, BOUNDARY, UNCOMMITTED},
    conflict::{OURS_COLOR, OURS_LABEL, Side, THEIRS_COLOR, UNRESOLVED_COLOR, describe_tile},
    document::DocumentId,
    icons::materialdesignicons::{
        ICON_ALERT,
        ICON_ALERT_CIRCLE,
        ICON_CHECK,
        ICON_CLOSE_THICK,
        ICON_REFRESH,
        ICON_SOURCE_BRANCH,
        ICON_SOURCE_COMMIT,
        ICON_SOURCE_MERGE,
    },
};

use crate::{
    session::{self, BlameState, GitDocState, Session},
    settings::Settings,
};

const STALE_COLOR: [f32; 4] = [1.0, 0.8, 0.3, 1.0];
const LEGEND_STEPS: usize = 32;
const LEGEND_WIDTH: f32 = 140.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GitTab {
    Conflicts,
    Blame,
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
}

pub(super) struct GitPanel {
    window: WindowKey,
    focus_requested: bool,
    select_tab: Option<GitTab>,
    selection: HashSet<Coord>,
    anchor: Option<Coord>,
    selection_document: Option<DocumentId>,
    unresolved_only: bool,
    current_level_only: bool,
    show_all_commits: bool,
    /// Whether the window's contents ran last frame, false while another tab covers it
    visible: bool,
}

impl GitPanel {
    pub fn new() -> Result<Self, WindowKeyError> {
        Ok(Self {
            window: WindowKey::new("git-panel", "Git")?,
            focus_requested: false,
            select_tab: None,
            selection: HashSet::new(),
            anchor: None,
            selection_document: None,
            unresolved_only: false,
            current_level_only: false,
            show_all_commits: false,
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
            self.selection.clear();
            self.anchor = None;
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
                super::focus_window_on_hover(ui);
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
            .filter(|coord| self.selection.contains(coord))
            .collect::<Vec<_>>();
        let (targets, suffix) = if selected.is_empty() {
            (conflicts.coords().to_vec(), String::from(" for all"))
        } else {
            let count = selected.len();
            (selected, format!(" ({count} selected)"))
        };
        for side in [Side::Ours, Side::Theirs] {
            if side == Side::Theirs {
                ui.same_line();
            }
            if ui.button(format!("Take {}{suffix}##take-{side:?}", conflicts.side_label(side))) {
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
        align_right(
            ui,
            ui.calc_text_size(&label)[0] + ui.clone_style().frame_padding()[0] * 2.0,
        );
        if ui.with_disabled_if(blocker.is_some(), || ui.button(&label)) {
            actions.push(Action::MarkResolved);
        }
        if ui.is_item_hovered_with_flags(ItemHoveredFlags::ALLOW_WHEN_DISABLED) {
            ui.tooltip_text(blocker.as_deref().unwrap_or("Stage the map with git add"));
        }

        ui.checkbox("Unresolved only", &mut self.unresolved_only);
        ui.same_line();
        ui.checkbox("Current level only", &mut self.current_level_only);

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

        ui.table("git-conflicts")
            .flags(
                TableFlags::ROW_BG
                    | TableFlags::BORDERS_INNER_V
                    | TableFlags::BORDERS_OUTER
                    | TableFlags::RESIZABLE
                    | TableFlags::HIDEABLE
                    | TableFlags::SCROLL_Y,
            )
            .sizing_policy(TableSizingPolicy::StretchProp)
            .freeze(0, 1)
            .headers(true)
            .column("Tile")
            .width(tile_width)
            .flags(TableColumnFlags::NO_HIDE)
            .done()
            .column("Region")
            .width(ui.calc_text_size("Region")[0])
            .done()
            .column("Base")
            .weight(1.0)
            .flags(TableColumnFlags::DEFAULT_HIDE)
            .done()
            .column("Status")
            .width(ui.calc_text_size(format!("Unresolved{theirs}"))[0].max(tile_width) * 0.6)
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
                        .selected(self.selection.contains(&coord))
                        .flags(SelectableFlags::SPAN_ALL_COLUMNS | SelectableFlags::ALLOW_OVERLAP)
                        .build();
                    if clicked && self.click_row(ui, &order, position) {
                        actions.push(Action::Center(coord));
                    }
                    if let Some(_menu) = ui.begin_popup_context_item() {
                        let targets = if self.selection.contains(&coord) {
                            order
                                .iter()
                                .copied()
                                .filter(|coord| self.selection.contains(coord))
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

    /// Updates the selection like a list view, returns whether the view should follow the row
    fn click_row(&mut self, ui: &Ui, order: &[Coord], position: usize) -> bool {
        let coord = order[position];
        let io = ui.io();
        let anchor = self
            .anchor
            .and_then(|anchor| order.iter().position(|candidate| *candidate == anchor));

        if io.key_shift()
            && let Some(anchor) = anchor
        {
            if !io.key_ctrl() {
                self.selection.clear();
            }
            self.selection
                .extend(&order[anchor.min(position)..=anchor.max(position)]);
            false
        } else if io.key_ctrl() {
            if !self.selection.remove(&coord) {
                self.selection.insert(coord);
            }
            self.anchor = Some(coord);
            false
        } else {
            self.selection.clear();
            self.selection.insert(coord);
            self.anchor = Some(coord);
            true
        }
    }

    fn draw_blame(
        &mut self, ui: &Ui, session: &Session, settings: &Settings, id: DocumentId, git: &GitDocState,
        actions: &mut Vec<Action>,
    ) {
        let mut show = git.show_blame;
        if ui.checkbox("Show heatmap", &mut show) {
            actions.push(Action::ToggleBlame);
        }
        ui.same_line();
        let blame = session.blame_state(id);
        let label = if blame.is_some() {
            format!("{ICON_REFRESH} Refresh##run-blame")
        } else {
            String::from("Run blame##run-blame")
        };
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
            ui.text_colored(STALE_COLOR, format!("{ICON_ALERT} Map changed since blame ran"));
        }

        let Some(blame) = blame else {
            if session.blame_progress(id).is_none() {
                centered_note(ui, "Run blame to see which commit last changed each tile");
            }
            return;
        };

        if blame.counts.boundary != 0 || blame.result.parse_boundary.is_some() {
            ui.text_disabled(blame.result.boundary_label());
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
        let web = git.web_commit_base.as_deref();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |time| time.as_secs() as i64);
        let swatch = ui.text_line_height();
        let z = session.z();

        ui.table("git-blame-commits")
            .flags(
                TableFlags::ROW_BG
                    | TableFlags::BORDERS_INNER_V
                    | TableFlags::BORDERS_OUTER
                    | TableFlags::RESIZABLE
                    | TableFlags::HIDEABLE
                    | TableFlags::SCROLL_Y,
            )
            .sizing_policy(TableSizingPolicy::StretchProp)
            .freeze(0, 1)
            .headers(true)
            .column("##heat")
            .width(swatch)
            .flags(TableColumnFlags::NO_HIDE)
            .done()
            .column("Commit")
            .width(ui.calc_text_size("Uncommitted")[0])
            .flags(TableColumnFlags::NO_HIDE)
            .done()
            .column("Author")
            .weight(0.6)
            .done()
            .column("Date")
            .width(ui.calc_text_size("11 months ago")[0])
            .done()
            .column("Summary")
            .weight(1.4)
            .done()
            .column("Tiles")
            .width(ui.calc_text_size("000000")[0])
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
                            let url = format!("{web}/{}", commit.hash);
                            ui.text_link_open_url(&commit.short, &url);
                            ui.set_item_tooltip(&url);
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
}

fn draw_header(ui: &Ui, git: &GitDocState, actions: &mut Vec<Action>) {
    let branch = git.branch.as_deref().unwrap_or("Reading repository...");
    ui.text(format!("{ICON_SOURCE_BRANCH} {branch}"));
    if let Some(head) = &git.head {
        ui.same_line();
        ui.text_disabled(format!("HEAD {}", head.short));
        ui.set_item_tooltip(&head.subject);
    }
    ui.same_line();
    ui.text_disabled(&git.repo.rel);

    let refresh = format!("{ICON_REFRESH}##refresh-git");
    align_right(ui, ui.frame_height());
    if ui.button_with_size(&refresh, [ui.frame_height(), ui.frame_height()]) {
        actions.push(Action::Refresh);
    }
    ui.set_item_tooltip("Refresh Git status");

    if let Some(operation) = &git.operation {
        ui.text_colored(
            opaque(THEIRS_COLOR),
            format!(
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
        ui.same_line();
        if ui.small_button("Load conflicts") {
            actions.push(Action::LoadConflicts);
        }
    }
    if let Some(error) = &git.error {
        if ui.small_button(format!("{ICON_CLOSE_THICK}##dismiss-git-error")) {
            actions.push(Action::ClearError);
        }
        ui.set_item_tooltip("Dismiss");
        ui.same_line();
        ui.text_colored(super::SAVE_ERROR_COLOR, format!("{ICON_ALERT_CIRCLE} {error}"));
    }
    ui.spacing();
}

fn draw_legend(ui: &Ui, blame: &BlameState) {
    let height = ui.text_line_height();
    let limit = blame.result.history_limit;

    if limit != 0 {
        ui.text_disabled("Newest");
        ui.same_line();
        let origin = ui.cursor_screen_pos();
        let step = LEGEND_WIDTH / LEGEND_STEPS as f32;
        for index in 0..LEGEND_STEPS {
            let cell = (index * limit.saturating_sub(1) / (LEGEND_STEPS - 1)) as u32;
            let color = session::blame_color(&blame.result, cell);
            let min = [origin[0] + step * index as f32, origin[1]];
            ui.get_window_draw_list()
                .add_rect(min, [min[0] + step + 0.5, min[1] + height], opaque(color))
                .filled(true)
                .build();
        }
        ui.dummy([LEGEND_WIDTH, height]);
        ui.same_line();
        ui.text_disabled("Oldest");
        ui.same_line();
    }

    for (cell, label) in [(UNCOMMITTED, "Uncommitted"), (BOUNDARY, "Older")] {
        let origin = ui.cursor_screen_pos();
        draw_swatch(ui, origin, height, session::blame_color(&blame.result, cell));
        ui.dummy([height, height]);
        ui.same_line();
        ui.text_disabled(label);
        if cell == UNCOMMITTED {
            ui.same_line();
        }
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

/// Moves the next item on this line against the right edge
fn align_right(ui: &Ui, width: f32) {
    ui.same_line();
    let available = ui.content_region_avail()[0];
    if available > width {
        let [x, y] = ui.cursor_pos();
        ui.set_cursor_pos([x + available - width, y]);
    }
}

fn centered_note(ui: &Ui, text: &str) {
    let width = ui.content_region_avail()[0];
    let size = ui.calc_text_size(text);
    let [x, y] = ui.cursor_pos();
    if width > size[0] {
        ui.set_cursor_pos([x + (width - size[0]) * 0.5, y + ui.text_line_height()]);
        ui.text_disabled(text);
    } else {
        ui.set_cursor_pos([x, y + ui.text_line_height()]);
        ui.text_wrapped(text);
    }
}

const fn opaque(color: [f32; 3]) -> [f32; 4] { [color[0], color[1], color[2], 1.0] }
