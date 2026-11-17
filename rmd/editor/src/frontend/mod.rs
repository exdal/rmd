use base::path::TreePath;
use std::{
    cell::Cell,
    collections::{HashMap, VecDeque},
    mem,
    path::{Path, PathBuf},
};

use dear_imgui_rs::{
    DockLayout, DockLayoutApply, DockNodeFlags, DockSplit, DockspaceError, Id, Ui, WindowKey, WindowKeyError,
};
use dmm::{Coord, PrefabInstanceId};
use render::{Camera, GuideLine, MapViewInteraction, MapViewRect};
use ui::theme::Themes;

use crate::{
    app::{external_editor::SourceLocation, pacing::FrameDemand, update::UpdateCheck},
    bake::panel::RuntimesPanel,
    coop::panel::{CoopDialog, comment::CommentDraft, draw_activity, notice::CoopNotice, presence::FollowedView},
    dm_ui::panel::DmUi,
    document::{
        DocumentId,
        panel::{
            DeleteLevelDialog, GoToDialog, NewLevelDialog, NewMapDialog, ResizeMapDialog, SaveDialog,
            SaveDialogOutcome, TileFillPaths, draw_save_dialog,
        },
    },
    environment::{
        panel::{DiagnosticsState, LoadNotice},
        session::LoadReport,
        worker::LoadView,
    },
    frame::session::TypeLayer,
    git::panel::{GitPanel, GitTab},
    gizmo::panel::GizmoState,
    inspector::panel::InspectorPanel,
    issues::panel::IssuesPanel,
    object_tree::panel::ObjectTreePanel,
    region::panel::RegionSelectionOptions,
    search::panel::{FindPanel, SimilarMatchKind},
    session::Session,
    settings::{
        Settings,
        keybind::{KeybindAction, KeybindPreset},
        panel::SettingsWindow,
    },
    stash::panel::StashView,
    tool::{
        FillMode, Tool,
        panel::{DEFAULT_CUSTOM_FILL_BOUNDARY, fill_warning::PendingFillWarning},
    },
    viewport::panel::{
        MapViewState,
        edit::EditCommand,
        shortcuts::MomentaryTool,
        stroke::{ActivePlacementFlash, MoveStroke, PickStroke, PlacementStroke},
    },
};

mod actions;
pub(crate) mod dialog;
#[cfg(test)]
pub(crate) mod fixtures;
mod menu;
pub(crate) mod welcome;

use self::{
    dialog::{SAVE_ERROR_COLOR, draw_keybind_preset_dialog},
    welcome::{ForgetRequest, WelcomeOutput},
};

const DOCKSPACE_ID: &str = "rmd-main-dockspace-v4";

fn dock_layout<'a>(
    left: [&WindowKey; 2], right: [&WindowKey; 3], center: impl IntoIterator<Item = &'a WindowKey>,
) -> DockLayout {
    DockLayout::split(
        DockSplit::Left,
        0.25,
        DockLayout::tabs(left),
        DockLayout::split(
            DockSplit::Right,
            0.20 / 0.75,
            DockLayout::tabs(right),
            DockLayout::tabs(center),
        ),
    )
}

pub(crate) const LAYER_KEYS: [(KeybindAction, TypeLayer); 4] = [
    (KeybindAction::ToggleAreaLayer, TypeLayer::Area),
    (KeybindAction::ToggleTurfLayer, TypeLayer::Turf),
    (KeybindAction::ToggleObjLayer, TypeLayer::Obj),
    (KeybindAction::ToggleMobLayer, TypeLayer::Mob),
];

const SCREENSHOT_KEYS: [(KeybindAction, ScreenshotArea, bool); 4] = [
    (KeybindAction::ScreenshotMap, ScreenshotArea::Map, false),
    (KeybindAction::ScreenshotSelection, ScreenshotArea::Selection, false),
    (KeybindAction::CopyMapScreenshot, ScreenshotArea::Map, true),
    (KeybindAction::CopySelectionScreenshot, ScreenshotArea::Selection, true),
];

pub struct VisibleMapView {
    pub document: DocumentId,
    pub rect: MapViewRect,
    pub camera: Camera,
    pub interaction: MapViewInteraction,
    pub guide_lines: Vec<GuideLine>,
    pub connected: Vec<PrefabInstanceId>,
    pub picked: Vec<PrefabInstanceId>,
}

#[derive(Default)]
pub struct UiOutput {
    pub exit: bool,
    pub map_views: Vec<VisibleMapView>,
    pub picking: Option<usize>,
    pub open: Option<OpenRequest>,
    pub open_source: Option<SourceLocation>,
    pub pick_new_map_path: bool,
    pub screenshot: Option<ScreenshotRequest>,
    pub cancel_load: bool,
    pub copy_to_clipboard: Option<String>,
    pub reload_profile: Option<ProfileReload>,
    pub load_conflicts: Option<DocumentId>,
    pub(crate) keybind_preset: Option<KeybindPreset>,
    pub demand: FrameDemand,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ProfileReload {
    Select(String),
    Force(Option<String>),
    Current,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenshotArea {
    Map,
    Selection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenshotRequest {
    pub area: ScreenshotArea,
    /// To the clipboard instead of a PNG file
    pub copy: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenRequest {
    PickCodebase,
    PickMap,
    Codebase(PathBuf),
    Map(PathBuf),
    ShareMap(PathBuf),
    ReloadCoop,
}

struct StartupPanelFocus {
    remaining_draws: u8,
}

impl StartupPanelFocus {
    const fn new() -> Self {
        Self { remaining_draws: 3 }
    }

    const fn requested(&self) -> bool {
        self.remaining_draws > 0
    }

    fn cancel(&mut self) {
        self.remaining_draws = 0;
    }

    fn after_draw(&mut self, docked: bool) {
        if docked {
            self.cancel();
        } else {
            self.remaining_draws = self.remaining_draws.saturating_sub(1);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CloseFocus {
    Map(DocumentId),
    Welcome,
}

pub struct UiState {
    pub(crate) object_tree: ObjectTreePanel,
    pub(crate) map_views: HashMap<DocumentId, MapViewState>,
    pub(crate) central_node: Option<Id>,
    pub(crate) dockspace_root: Option<Id>,
    pub(crate) dm_ui: DmUi,
    pub(crate) welcome_window: WindowKey,
    pub(crate) inspector: InspectorPanel,
    pub(crate) find: FindPanel,
    pub(crate) issues: IssuesPanel,
    pub(crate) runtimes: RuntimesPanel,
    pub(crate) git_panel: GitPanel,
    /// Keeps the object tree in front of the Git tab while its saved layout settles
    select_object_tree: StartupPanelFocus,
    /// Keeps the inspector in front of the Search tab while its saved layout settles
    select_inspector: StartupPanelFocus,
    /// A docked tab only comes forward if it still holds the focus when its tab bar updates next frame
    pub(crate) panel_focus_requested: bool,
    pub(crate) settings_window: SettingsWindow,
    pub(crate) layout: DockLayout,
    pub(crate) reset_layout: bool,
    pub(crate) gizmo: GizmoState,
    pub(crate) gizmo_context: Option<(DocumentId, Tool, u32)>,
    pub(crate) move_stroke: Option<MoveStroke>,
    pub(crate) placement_flash: Option<ActivePlacementFlash>,
    pub(crate) placement_stroke: Option<PlacementStroke>,
    pub(crate) pick_stroke: Option<PickStroke>,
    pub(crate) momentary_tool: Option<MomentaryTool>,
    pub(crate) region_selection_options: RegionSelectionOptions,
    pub(crate) fill_mode: FillMode,
    pub(crate) custom_fill_boundaries: Vec<TreePath>,
    pub(crate) custom_fill_search: String,
    pub(crate) pending_fill_warning: Option<PendingFillWarning>,
    pub(crate) popup_was_open: bool,
    pub(crate) new_map_dialog: Option<NewMapDialog>,
    pub(crate) new_level_dialog: Option<NewLevelDialog>,
    pub(crate) delete_level_dialog: Option<DeleteLevelDialog>,
    pub(crate) resize_map_dialog: Option<ResizeMapDialog>,
    pub(crate) go_to_dialog: Option<GoToDialog>,
    pub(crate) coop_dialog: Option<CoopDialog>,
    pub(crate) coop_notice: CoopNotice,
    pub(crate) comment_draft: Option<CommentDraft>,
    pub(crate) followed_view: Option<FollowedView>,
    pub(crate) last_go_to: Option<Coord>,
    pub(crate) tile_fill: Option<TileFillPaths>,
    pub(crate) save_dialog: Option<SaveDialog>,
    pub(crate) close_queue: VecDeque<DocumentId>,
    pub(crate) close_focus: Option<CloseFocus>,
    pub(crate) edit_command: Option<EditCommand>,
    pub(crate) pending_conflict_reload: Option<DocumentId>,
    pub(crate) save_requested: bool,
    pub(crate) exit_requested: bool,
    pub(crate) is_stash_open_requested: bool,
    pub(crate) stash_view: StashView,
    pub(crate) show_welcome: bool,
    pub(crate) welcome_map_filter: String,
    pub(crate) welcome_maps_expanded: bool,
    pub(crate) open_error: Option<String>,
    pub(crate) load_window: WindowKey,
    pub(crate) load_notice: Option<LoadNotice>,
    pub(crate) diagnostics: DiagnosticsState,
    pub(crate) keybind_preset_prompt: bool,
    pub(crate) copy_to_clipboard: Option<String>,
    pub(crate) open_map: Option<PathBuf>,
    pub(crate) update_check: UpdateCheck,
    pub(crate) demand: Cell<FrameDemand>,
}

impl UiState {
    pub fn new(keybind_preset_prompt: bool) -> Result<Self, WindowKeyError> {
        let object_tree = ObjectTreePanel::new()?;
        let welcome_window = WindowKey::new("welcome", "Welcome")?;
        let inspector = InspectorPanel::new()?;
        let find = FindPanel::new()?;
        let issues = IssuesPanel::new()?;
        let runtimes = RuntimesPanel::new()?;
        let git_panel = GitPanel::new()?;
        let settings_window = SettingsWindow::new()?;
        let load_window = WindowKey::new("load", "Loading")?;
        let layout = dock_layout(
            [object_tree.window(), git_panel.window()],
            [inspector.window(), find.window(), issues.window()],
            [&welcome_window],
        );

        Ok(Self {
            object_tree,
            map_views: HashMap::new(),
            central_node: None,
            dockspace_root: None,
            dm_ui: DmUi::default(),
            welcome_window,
            inspector,
            find,
            issues,
            runtimes,
            git_panel,
            select_object_tree: StartupPanelFocus::new(),
            select_inspector: StartupPanelFocus::new(),
            panel_focus_requested: false,
            settings_window,
            layout,
            reset_layout: false,
            gizmo: GizmoState::default(),
            gizmo_context: None,
            move_stroke: None,
            placement_flash: None,
            placement_stroke: None,
            pick_stroke: None,
            momentary_tool: None,
            region_selection_options: RegionSelectionOptions::default(),
            fill_mode: FillMode::default(),
            custom_fill_boundaries: vec![TreePath::parse(DEFAULT_CUSTOM_FILL_BOUNDARY)],
            custom_fill_search: String::new(),
            pending_fill_warning: None,
            popup_was_open: false,
            new_map_dialog: None,
            new_level_dialog: None,
            delete_level_dialog: None,
            resize_map_dialog: None,
            go_to_dialog: None,
            coop_dialog: None,
            coop_notice: CoopNotice::default(),
            comment_draft: None,
            followed_view: None,
            last_go_to: None,
            tile_fill: None,
            save_dialog: None,
            close_queue: VecDeque::new(),
            close_focus: None,
            edit_command: None,
            pending_conflict_reload: None,
            save_requested: false,
            exit_requested: false,
            is_stash_open_requested: false,
            stash_view: StashView::new()?,
            show_welcome: true,
            welcome_map_filter: String::new(),
            welcome_maps_expanded: false,
            open_error: None,
            load_window,
            load_notice: None,
            diagnostics: DiagnosticsState::default(),
            keybind_preset_prompt,
            copy_to_clipboard: None,
            open_map: None,
            update_check: UpdateCheck::default(),
            demand: Cell::new(FrameDemand::Idle),
        })
    }

    pub fn is_checking_for_updates(&self) -> bool {
        self.update_check.is_checking()
    }

    #[cfg(test)]
    pub(crate) fn map_issues_visible(&self) -> bool {
        self.issues.visible()
    }

    pub(crate) fn raise_demand(&self, demand: FrameDemand) {
        let mut raised = self.demand.get();
        raised.raise(demand);
        self.demand.set(raised);
    }

    pub fn set_open_error(&mut self, error: Option<String>) {
        self.open_error = error;
    }

    pub fn reset_coop_notice(&mut self) {
        self.coop_notice.reset();
    }

    pub fn reveal_picked_instance(&mut self, session: &Session) {
        self.object_tree.reveal_picked_instance(session);
    }

    pub fn request_mouse_popup(&mut self) {
        self.dm_ui.request_mouse_popup();
    }

    pub fn set_load_notice(&mut self, notice: Option<LoadNotice>) {
        self.load_notice = notice;
    }

    pub fn set_profiles(&mut self, profiles: Vec<String>) {
        self.settings_window.set_profiles(profiles);
    }

    pub fn set_codebase_report(&mut self, report: LoadReport) {
        self.diagnostics.set_codebase(report);
    }

    pub fn set_map_report(&mut self, path: PathBuf, report: LoadReport) {
        self.diagnostics.set_map(path, report);
    }

    pub fn set_failed_codebase_report(&mut self, report: LoadReport) {
        self.diagnostics.set_failed_codebase(report);
    }

    pub fn request_exit(&mut self) {
        self.exit_requested = true;
    }

    pub fn request_refit(&mut self, id: Option<DocumentId>) {
        if let Some(view) = id.and_then(|id| self.map_views.get_mut(&id)) {
            view.refit = true;
        }
    }

    pub(crate) fn cancel_tool_gestures(&mut self) {
        self.gizmo.cancel();
        self.move_stroke = None;
        self.placement_flash = None;
        self.placement_stroke = None;
        self.pick_stroke = None;
    }

    pub(crate) fn cancel_edit_gestures(&mut self, session: &mut Session, id: Option<DocumentId>) {
        self.cancel_tool_gestures();
        session.cancel_node_drag();

        if let Some(id) = id
            && let Some(view) = self.map_views.get_mut(&id)
        {
            view.gestures.cancel(session, id);
        }
    }

    pub fn set_new_map_path(&mut self, path: PathBuf, codebase_dir: &Path) {
        let Some(dialog) = self.new_map_dialog.as_mut() else {
            return;
        };

        dialog.path = path.strip_prefix(codebase_dir).unwrap_or(&path).display().to_string();
        dialog.error = None;
    }

    pub fn draw(
        &mut self, ui: &Ui, session: &mut Session, settings: &mut Settings, themes: &mut Themes,
        load: Option<&LoadView>,
    ) -> Result<UiOutput, DockspaceError> {
        let loading = load.is_some() || self.load_notice.is_some();
        self.demand.set(FrameDemand::Idle);
        if load.is_some() {
            self.raise_demand(FrameDemand::Full);
        }

        if settings.check_for_updates {
            self.update_check.start();
        }

        let root = self.draw_dockspace(ui, session)?;

        if self.keybind_preset_prompt {
            let keybind_preset = draw_keybind_preset_dialog(ui, &mut self.keybind_preset_prompt);
            let exit = self.draw_exit_confirmation(ui, session);

            return Ok(UiOutput {
                exit,
                keybind_preset,
                ..UiOutput::default()
            });
        }

        let mut menu = self.draw_menu_bar(ui, session, settings, loading);
        self.read_edit_keys(ui, session, settings, &mut menu);
        self.apply_window_actions(settings, &menu);
        settings.mirror_camera ^= menu.toggle_mirror_camera;
        self.edit_command = menu.edit;

        self.apply_file_actions(ui, session, settings, &mut menu);
        self.apply_search_actions(ui, session, settings, &mut menu);

        self.apply_history_actions(session, &menu);
        self.apply_view_toggles(session, settings, &menu);

        if menu.open_save_dialog {
            self.open_save_dialog(ui, session, None);
        }

        match draw_save_dialog(ui, session, &mut self.save_dialog) {
            Some(SaveDialogOutcome::Saved {
                document,
                close_after_save: true,
            }) => {
                self.close_map_view(session, document);
                self.close_queue.pop_front();
            },
            Some(SaveDialogOutcome::Cancelled { close_after_save: true }) => self.close_queue.clear(),
            _ => {},
        }

        let reload_profile = self.draw_settings_window(ui, session, settings, themes, load.is_some());
        if menu.show_welcome {
            self.show_welcome = true;
            self.close_focus = Some(CloseFocus::Welcome);
        }

        let open_source = self.draw_panels(ui, session, settings);
        let load_conflicts = self.draw_reload_conflicts_popup(ui, session);

        let mut welcome = WelcomeOutput::default();
        self.draw_welcome(ui, session, settings, loading, &mut welcome);
        match welcome.forget {
            Some(ForgetRequest::Codebase(path)) => settings.forget_codebase(&path),
            Some(ForgetRequest::Map(path)) => settings.forget_recent(&path),
            None => {},
        }

        let pick_new_map_path = self.draw_new_map(ui, session, welcome.new_map_dialog || menu.new_map);
        let load_popup = self.draw_load_window(ui, load);
        let (map_views, picking) = self.draw_open_maps(ui, session, settings, menu.refit);
        if settings.show_dm_ui {
            self.dm_ui.draw(ui, session, root.raw());
        } else {
            self.dm_ui.skip();
        }

        self.draw_map_dialogs(ui, session, &menu);
        let coop_open = self.draw_coop_dialogs(ui, session, settings, loading, menu.coop_dialog);
        if let Some(coop) = session.coop()
            && draw_activity(ui, coop)
        {
            self.raise_demand(FrameDemand::Full);
        }

        self.settings_window
            .finish_keybind_capture(ui, &mut settings.keybindings);

        let exit = self.draw_exit_confirmation(ui, session);
        self.popup_was_open = ui.is_popup_open_with_flags("", dear_imgui_rs::PopupQueryFlags::ANY_POPUP);

        Ok(UiOutput {
            exit,
            map_views,
            picking,
            open: coop_open
                .or(welcome.open)
                .or(menu.open)
                .or(self.open_map.take().map(OpenRequest::Map)),
            open_source,
            pick_new_map_path,
            screenshot: menu.screenshot,
            cancel_load: load_popup.cancel,
            copy_to_clipboard: load_popup.copy.or_else(|| self.copy_to_clipboard.take()),
            reload_profile,
            load_conflicts,
            keybind_preset: None,
            demand: self.demand.get(),
        })
    }

    fn draw_dockspace(&mut self, ui: &Ui, session: &Session) -> Result<Id, DockspaceError> {
        let root = ui.get_id(DOCKSPACE_ID);
        // map views only exist at runtime, so a reset has to name them to keep them docked
        let reset = mem::take(&mut self.reset_layout).then(|| {
            let map_views = session
                .state
                .document_ids()
                .into_iter()
                .filter_map(|id| self.map_views.get(&id))
                .map(MapViewState::window);
            dock_layout(
                [self.object_tree.window(), self.git_panel.window()],
                [self.inspector.window(), self.find.window(), self.issues.window()],
                std::iter::once(&self.welcome_window).chain(map_views),
            )
        });
        ui.dockspace()
            .main_viewport()
            .root_id(root)
            .flags(DockNodeFlags::PASSTHRU_CENTRAL_NODE)
            .layout(
                reset.as_ref().unwrap_or(&self.layout),
                if reset.is_some() {
                    DockLayoutApply::Replace
                } else {
                    DockLayoutApply::IfMissing
                },
            )
            .build()?;
        self.dockspace_root = Some(root);

        Ok(root)
    }

    fn draw_settings_window(
        &mut self, ui: &Ui, session: &mut Session, settings: &mut Settings, themes: &mut Themes, loading: bool,
    ) -> Option<ProfileReload> {
        let output = self.settings_window.draw(ui, session, settings, themes, loading);
        session.sync_git_enabled(settings.git_enabled);
        session.set_sanitize_vars_on_save(settings.sanitize_vars_on_save);
        if output.object_tree_changed {
            self.object_tree.invalidate_filter();
        }

        output.reload_profile
    }

    fn draw_panels(&mut self, ui: &Ui, session: &mut Session, settings: &mut Settings) -> Option<SourceLocation> {
        if session.take_loaded_conflicts().is_some() {
            self.git_panel.request(GitTab::Conflicts);
        }
        let loaded_issues = session.take_loaded_issues();

        self.panel_focus_requested = self.find.has_focus_request()
            || self.issues.has_focus_request()
            || loaded_issues.is_some()
            || self.git_panel.has_focus_request()
            || self.close_focus.is_some();
        let mut panels = settings.panels;
        panels.git |= self.git_panel.has_focus_request();
        // The Git tab shares the object tree's dock node, a request to show it wins
        if self.git_panel.has_focus_request() || !panels.object_tree {
            self.select_object_tree.cancel();
        }

        let object_tree = self.object_tree.draw(
            ui,
            session,
            settings,
            self.select_object_tree.requested(),
            &mut panels.object_tree,
        );
        self.select_object_tree.after_draw(object_tree.docked);
        if let Some(path) = &object_tree.find {
            self.find.open_for_path(session, path);
        }

        if self.find.has_focus_request()
            || self.issues.has_focus_request()
            || loaded_issues.is_some()
            || !panels.inspector
        {
            self.select_inspector.cancel();
        }

        let inspector = self.inspector.draw(
            ui,
            session,
            settings,
            self.select_inspector.requested(),
            &mut panels.inspector,
        );
        self.select_inspector.after_draw(inspector.docked);
        self.copy_to_clipboard = inspector.copy.or(object_tree.copy_path);
        if let Some((document, instance)) = inspector.find_similar {
            self.find
                .open_for(session, document, instance, SimilarMatchKind::Prefab);
        }

        panels.search |= self.find.has_focus_request();
        if let Some(target) = self.find.draw(ui, session, settings, &mut panels.search) {
            self.jump_to_instance(session, target);
        }

        panels.map_issues |= self.issues.has_focus_request() || loaded_issues.is_some();
        let issues = self.issues.draw(ui, session, settings, &mut panels.map_issues);
        if let Some(id) = loaded_issues {
            self.issues.request(Some(id));
        }
        if let Some(target) = issues.jump {
            self.jump_to_instance(session, target);
        }

        if let Some((document, coord)) = issues.tile {
            self.go_to_tile(session, document, coord, ui.time());
        }
        if issues.copy.is_some() {
            self.copy_to_clipboard = issues.copy;
        }

        let runtimes = self.runtimes.draw(ui, session, settings, &mut panels.runtimes);
        let git_output = self.git_panel.draw(ui, session, settings, &mut panels.git);
        settings.panels = panels;
        if git_output.copy.is_some() {
            self.copy_to_clipboard = git_output.copy;
        }

        if runtimes.copy.is_some() {
            self.copy_to_clipboard = runtimes.copy;
        }

        if runtimes.clear {
            session.clear_runtimes();
        }

        if let Some((id, coord)) = git_output.center {
            self.center_view_on(session, id, coord);
        }

        if let Some(id) = git_output.load_conflicts {
            self.pending_conflict_reload = Some(id);
        }

        inspector
            .open_source
            .or(object_tree.open_source)
            .or(runtimes.open_source)
    }
}

#[cfg(test)]
mod tests;
