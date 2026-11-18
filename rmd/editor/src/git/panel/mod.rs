pub(crate) mod blame;
mod conflicts;
mod diff;
mod legend;
pub(crate) mod tooltip;
mod versions;

use base::coord::Coord;
use dear_imgui_rs::{TabItemFlags, Ui, WindowKey, WindowKeyError};
use ui::{
    icons::materialdesignicons::{
        ICON_ALERT, ICON_ALERT_CIRCLE, ICON_CLOSE_THICK, ICON_REFRESH, ICON_SOURCE_BRANCH, ICON_SOURCE_MERGE,
    },
    widgets::{
        RowSelection, align_right, button_width, centered_note, focus_window_on_hover, label_width, opaque,
        same_line_if_fits, text_wrapped_colored,
    },
};

use crate::{
    document::DocumentId,
    frontend::dialog::SAVE_ERROR_COLOR,
    git::{
        conflict::{Side, THEIRS_COLOR, UNRESOLVED_COLOR},
        diff::DiffSource,
        repo::{CommitInfo, WebLinks},
        session::GitDocState,
    },
    session::Session,
    settings::Settings,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GitTab {
    Conflicts,
    Blame,
    Diff,
}

#[derive(Default)]
pub(crate) struct GitPanelOutput {
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

pub(crate) struct GitPanel {
    window: WindowKey,
    focus_requested: bool,
    select_tab: Option<GitTab>,
    conflict_rows: RowSelection<Coord>,
    diff_rows: RowSelection<Coord>,
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

    pub const fn window(&self) -> &WindowKey {
        &self.window
    }

    /// Brings the panel to the front of its dock node
    pub fn focus(&mut self) {
        self.focus_requested = true;
    }

    pub fn request(&mut self, tab: GitTab) {
        self.focus_requested = true;
        self.select_tab = Some(tab);
    }

    pub fn has_focus_request(&self) -> bool {
        self.focus_requested
    }

    #[cfg(test)]
    pub fn visible(&self) -> bool {
        self.visible
    }

    pub fn draw(&mut self, ui: &Ui, session: &mut Session, settings: &Settings, open: &mut bool) -> GitPanelOutput {
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
        if !*open {
            return output;
        }

        ui.window(window.label(title)).opened(open).focused(focus).build(|| {
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

pub(crate) fn commit_summary(ui: &Ui, commit: &CommitInfo, web: Option<&WebLinks>, width: f32) {
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
