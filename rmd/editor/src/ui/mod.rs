use core::path::TreePath;
use std::{
    cell::Cell,
    collections::{HashMap, VecDeque},
    mem,
    path::{Path, PathBuf},
};

use dear_imgui_rs::{
    DockLayout,
    DockLayoutApply,
    DockNodeFlags,
    DockSplit,
    DockspaceError,
    Id,
    Ui,
    WindowFlags,
    WindowKey,
    WindowKeyError,
};
use dmm::{Coord, PrefabInstanceId};
use editor::{
    document::{DocumentId, MapDocument},
    icons::materialdesignicons::ICON_IMAGE_BROKEN,
    tool::{FillMode, SelectionRotation, SelectionTransform, Tool},
};
pub(crate) use inspector::TransformMode;
use render::{Camera, GuideLine, MapViewInteraction, MapViewRect, Renderer};

use self::{inspector::InspectorPanel, object_tree::ObjectTreePanel, settings::SettingsWindow};
use crate::{
    external_editor::SourceLocation,
    gizmo::GizmoState,
    loader::LoadView,
    pacing::FrameDemand,
    session::{LoadReport, Session, TypeLayer},
    settings::{KeybindAction, KeybindPreset, OpenPanels, Panel, Settings},
    theme::Themes,
    update::UpdateCheck,
};

mod blame;
mod block;
mod coop;
mod dialog;
mod find;
#[cfg(test)]
mod fixtures;
mod grid;
mod history;
mod load;
mod menu;
mod node;
mod overlay;
mod paste;
mod search;
mod toolbar;
mod tooltip;
mod viewport;
mod welcome;

use self::{
    blame::{BlamePopup, blame_tooltip_position, draw_blame_popup},
    block::{
        BlockPlacementAction,
        BlockSelectionOptions,
        PendingBlockPlacement,
        PlacementControls,
        RectangleGesture,
        block_controls_placement,
        block_placement_controls_layout,
        draw_block_outline,
        draw_block_placement_controls,
        draw_marching_edge,
        draw_selection_mode_controls,
        marching_stripe_offset,
        rectangle_drag_coord,
        restore_rectangle_gesture,
    },
    dialog::{
        DeleteLevelDialog,
        FILL_LIMIT_WARNING_POPUP,
        FillWarningContext,
        GO_TO_POPUP,
        GoToDialog,
        NEW_MAP_POPUP,
        NewLevelDialog,
        NewMapDialog,
        PendingFillWarning,
        RESIZE_MAP_POPUP,
        ResizeMapDialog,
        SAVE_ERROR_COLOR,
        SAVE_MAP_POPUP,
        SaveDialog,
        SaveDialogOutcome,
        TileFillPaths,
        TileFillSearch,
        draw_delete_level_dialog,
        draw_fill_limit_warning,
        draw_go_to_dialog,
        draw_keybind_preset_dialog,
        draw_new_level_dialog,
        draw_new_map_dialog,
        draw_resize_map_dialog,
        draw_save_dialog,
    },
    grid::{draw_selected_pixel_grid, draw_tile_grid},
    history::{draw_history_overlay, recent_button_size},
    load::{DIAGNOSTIC_WARNING_COLOR, DiagnosticsState, LoadPopup, draw_load_popup},
    menu::MenuActions,
    node::{NodeOverlayView, NodeRightClick, draw_node_overlay, node_right_click},
    overlay::{
        OVERLAY_BG,
        OverlayRect,
        conflict_controls_layout,
        draw_conflict_controls,
        draw_guide_badges,
        draw_highlights,
        draw_identical_outlines,
        draw_overlay_underlay,
        draw_placement_preview,
        overlay_padding,
    },
    paste::{PASTE_LABELS, PasteAction, PendingPaste, centered_paste_min, draw_paste_controls, paste_controls},
    search::{MAX_CUSTOM_FILL_SEARCH_RESULTS, draw_type_path_search},
    toolbar::{DEFAULT_CUSTOM_FILL_BOUNDARY, TopOverlayState, draw_top_overlay, request_level_change},
    tooltip::{draw_conflict_tooltip, draw_diff_tooltip},
    viewport::{
        ActivePlacementFlash,
        EDIT_KEYS,
        EditCommand,
        MapViewState,
        MomentaryTool,
        PickStroke,
        PlacementStroke,
    },
    welcome::{ForgetRequest, WelcomeOutput},
};
pub(crate) use self::{common::dpi, load::LoadNotice};

mod common;

mod context_menu;

mod dm;

mod git;

mod inspector;

mod object_tree;

mod settings;

const DOCKSPACE_ID: &str = "rmd-main-dockspace-v4";

fn dock_layout<'a>(
    left: [&WindowKey; 2], right: [&WindowKey; 2], center: impl IntoIterator<Item = &'a WindowKey>,
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

const LAYER_KEYS: [(KeybindAction, TypeLayer); 4] = [
    (KeybindAction::ToggleAreaLayer, TypeLayer::Area),
    (KeybindAction::ToggleTurfLayer, TypeLayer::Turf),
    (KeybindAction::ToggleObjLayer, TypeLayer::Obj),
    (KeybindAction::ToggleMobLayer, TypeLayer::Mob),
];

const RELOAD_CONFLICTS_POPUP: &str = "Load merge conflicts##reload-conflicts";

pub struct VisibleMapView {
    pub document: DocumentId,
    pub rect: MapViewRect,
    pub camera: Camera,
    pub interaction: MapViewInteraction,
    pub guide_lines: Vec<GuideLine>,
    pub connected: Vec<PrefabInstanceId>,
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
    const fn new() -> Self { Self { remaining_draws: 3 } }

    const fn requested(&self) -> bool { self.remaining_draws > 0 }

    fn cancel(&mut self) { self.remaining_draws = 0; }

    fn after_draw(&mut self, docked: bool) {
        if docked {
            self.cancel();
        } else {
            self.remaining_draws = self.remaining_draws.saturating_sub(1);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CloseFocus {
    Map(DocumentId),
    Welcome,
}

pub struct UiState {
    object_tree: ObjectTreePanel,
    map_views: HashMap<DocumentId, MapViewState>,
    central_node: Option<Id>,
    dockspace_root: Option<Id>,
    dm_ui: dm::DmUi,
    welcome_window: WindowKey,
    inspector: InspectorPanel,
    find: find::FindPanel,
    git_panel: git::GitPanel,
    /// Keeps the object tree in front of the Git tab while its saved layout settles
    select_object_tree: StartupPanelFocus,
    /// Keeps the inspector in front of the Search tab while its saved layout settles
    select_inspector: StartupPanelFocus,
    /// A docked tab only comes forward if it still holds the focus when its tab bar updates next frame
    panel_focus_requested: bool,
    settings_window: SettingsWindow,
    layout: DockLayout,
    reset_layout: bool,
    gizmo: GizmoState,
    gizmo_context: Option<(DocumentId, Tool, u32)>,
    placement_flash: Option<ActivePlacementFlash>,
    placement_stroke: Option<PlacementStroke>,
    pick_stroke: Option<PickStroke>,
    momentary_tool: Option<MomentaryTool>,
    block_selection_options: BlockSelectionOptions,
    fill_mode: FillMode,
    custom_fill_boundaries: Vec<TreePath>,
    custom_fill_search: String,
    pending_fill_warning: Option<PendingFillWarning>,
    popup_was_open: bool,
    new_map_dialog: Option<NewMapDialog>,
    new_level_dialog: Option<NewLevelDialog>,
    delete_level_dialog: Option<DeleteLevelDialog>,
    resize_map_dialog: Option<ResizeMapDialog>,
    go_to_dialog: Option<GoToDialog>,
    coop_dialog: Option<coop::CoopDialog>,
    coop_notice: coop::CoopNotice,
    comment_draft: Option<coop::CommentDraft>,
    followed_view: Option<viewport::FollowedView>,
    last_go_to: Option<Coord>,
    tile_fill: Option<TileFillPaths>,
    save_dialog: Option<SaveDialog>,
    close_queue: VecDeque<DocumentId>,
    close_focus: Option<CloseFocus>,
    edit_command: Option<EditCommand>,
    pending_conflict_reload: Option<DocumentId>,
    save_requested: bool,
    exit_requested: bool,
    show_welcome: bool,
    welcome_map_filter: String,
    welcome_maps_expanded: bool,
    open_error: Option<String>,
    load_window: WindowKey,
    load_notice: Option<LoadNotice>,
    diagnostics: DiagnosticsState,
    keybind_preset_prompt: bool,
    copy_to_clipboard: Option<String>,
    update_check: UpdateCheck,
    demand: Cell<FrameDemand>,
}

#[cfg(test)]
pub(crate) static IMGUI_CONTEXT: std::sync::Mutex<()> = std::sync::Mutex::new(());

impl UiState {
    pub fn new(keybind_preset_prompt: bool) -> Result<Self, WindowKeyError> {
        let object_tree = ObjectTreePanel::new()?;
        let welcome_window = WindowKey::new("welcome", "Welcome")?;
        let inspector = InspectorPanel::new()?;
        let find = find::FindPanel::new()?;
        let git_panel = git::GitPanel::new()?;
        let settings_window = SettingsWindow::new()?;
        let load_window = WindowKey::new("load", "Loading")?;
        let layout = dock_layout(
            [object_tree.window(), git_panel.window()],
            [inspector.window(), find.window()],
            [&welcome_window],
        );

        Ok(Self {
            object_tree,
            map_views: HashMap::new(),
            central_node: None,
            dockspace_root: None,
            dm_ui: dm::DmUi::default(),
            welcome_window,
            inspector,
            find,
            git_panel,
            select_object_tree: StartupPanelFocus::new(),
            select_inspector: StartupPanelFocus::new(),
            panel_focus_requested: false,
            settings_window,
            layout,
            reset_layout: false,
            gizmo: GizmoState::default(),
            gizmo_context: None,
            placement_flash: None,
            placement_stroke: None,
            pick_stroke: None,
            momentary_tool: None,
            block_selection_options: BlockSelectionOptions::default(),
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
            coop_notice: coop::CoopNotice::default(),
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
            show_welcome: true,
            welcome_map_filter: String::new(),
            welcome_maps_expanded: false,
            open_error: None,
            load_window,
            load_notice: None,
            diagnostics: DiagnosticsState::default(),
            keybind_preset_prompt,
            copy_to_clipboard: None,
            update_check: UpdateCheck::default(),
            demand: Cell::new(FrameDemand::Idle),
        })
    }

    pub fn is_checking_for_updates(&self) -> bool { self.update_check.is_checking() }

    fn raise_demand(&self, demand: FrameDemand) {
        let mut raised = self.demand.get();
        raised.raise(demand);
        self.demand.set(raised);
    }

    pub fn set_open_error(&mut self, error: Option<String>) { self.open_error = error; }

    pub fn reset_coop_notice(&mut self) { self.coop_notice.reset(); }

    pub fn reveal_selected_instance(&mut self, session: &Session) {
        self.object_tree.reveal_selected_instance(session);
    }

    pub fn request_mouse_popup(&mut self) { self.dm_ui.request_mouse_popup(); }

    pub fn set_load_notice(&mut self, notice: Option<LoadNotice>) { self.load_notice = notice; }

    pub fn set_profiles(&mut self, profiles: Vec<String>) { self.settings_window.set_profiles(profiles); }

    pub fn set_codebase_report(&mut self, report: LoadReport) { self.diagnostics.set_codebase(report); }

    pub fn set_map_report(&mut self, path: PathBuf, report: LoadReport) { self.diagnostics.set_map(path, report); }

    pub fn set_failed_codebase_report(&mut self, report: LoadReport) { self.diagnostics.set_failed_codebase(report); }

    pub fn request_exit(&mut self) { self.exit_requested = true; }

    pub fn request_refit(&mut self, id: Option<DocumentId>) {
        match id.and_then(|id| self.map_views.get_mut(&id)) {
            Some(view) => view.refit = true,
            None => {
                for view in self.map_views.values_mut() {
                    view.refit = true;
                }
            },
        }
    }

    fn cancel_tool_gestures(&mut self) {
        self.gizmo.cancel();
        self.placement_flash = None;
        self.placement_stroke = None;
        self.pick_stroke = None;
    }

    fn cancel_edit_gestures(&mut self, session: &mut Session, id: Option<DocumentId>) {
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
        self.show_welcome |= menu.show_welcome;

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
            && coop::draw_activity(ui, coop)
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
            open: coop_open.or(welcome.open).or(menu.open),
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

    fn apply_window_actions(&mut self, settings: &mut Settings, menu: &MenuActions) {
        settings.show_dm_ui ^= menu.toggle_dm_ui;
        self.reset_layout = menu.reset_layout;
        if menu.reset_layout {
            settings.panels = OpenPanels::default();
        }

        let Some(panel) = menu.toggle_panel else {
            return;
        };

        let open = settings.panels.get_mut(panel);
        *open = !*open;
        if !*open {
            return;
        }

        match panel {
            Panel::ObjectTree => self.select_object_tree = StartupPanelFocus::new(),
            Panel::Git => self.git_panel.focus(),
            Panel::Inspector => self.select_inspector = StartupPanelFocus::new(),
            Panel::Search => self.find.request(),
        }
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
                [self.inspector.window(), self.find.window()],
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

    fn map_keys_enabled(&self, ui: &Ui, session: &Session) -> bool {
        session.map().is_some()
            && self.save_dialog.is_none()
            && !self.settings_window.is_capturing_keybind()
            && !ui.io().want_text_input()
    }

    fn apply_history_actions(&mut self, session: &mut Session, menu: &MenuActions) {
        if !menu.undo && !menu.redo {
            return;
        }

        self.cancel_edit_gestures(session, session.state.active());
        if menu.undo {
            session.undo();
        } else {
            session.redo();
        }
    }

    fn read_edit_keys(&self, ui: &Ui, session: &Session, settings: &Settings, menu: &mut MenuActions) {
        if !self.map_keys_enabled(ui, session) {
            return;
        }

        if !unsafe { dear_imgui_rs::sys::igGetTopMostPopupModal() }.is_null() {
            return;
        }

        menu.undo |= settings.keybindings.get(KeybindAction::Undo).is_pressed_repeating(ui);
        menu.redo |= settings.keybindings.get(KeybindAction::Redo).is_pressed_repeating(ui);
        if menu.edit.is_none() {
            menu.edit = EDIT_KEYS
                .into_iter()
                .find(|(action, _)| settings.keybindings.get(*action).is_pressed(ui))
                .map(|(_, command)| command);
        }
    }

    fn apply_file_actions(&mut self, ui: &Ui, session: &mut Session, settings: &Settings, menu: &mut MenuActions) {
        // https://i.redd.it/ggq5w2aliicf1.png
        let is_save_requested = mem::take(&mut self.save_requested);
        let is_save_pressed =
            self.map_keys_enabled(ui, session) && settings.keybindings.get(KeybindAction::Save).is_pressed(ui);
        if session.map().is_some()
            && !menu.open_save_dialog
            && self.save_dialog.is_none()
            && (is_save_requested || is_save_pressed)
        {
            if session.can_save_map_in_place() {
                if let Err(error) = session.save_map() {
                    self.open_save_dialog(ui, session, Some(error.to_string()));
                }
            } else {
                menu.open_save_dialog = true;
            }
        }

        let is_map_keys_enabled = self.map_keys_enabled(ui, session);
        menu.save_all |= is_map_keys_enabled && settings.keybindings.get(KeybindAction::SaveAll).is_pressed(ui);
        menu.close_map |= is_map_keys_enabled && settings.keybindings.get(KeybindAction::CloseMap).is_pressed(ui);
        if menu.save_all {
            let outcome = session.save_all();
            if let Some(error) = outcome.error {
                self.open_error = Some(error);
            } else if let Some(id) = outcome.needs_path {
                session.set_active_document(id);
                menu.open_save_dialog = true;
            }
        }

        if menu.close_map
            && let Some(id) = session.state.active()
        {
            self.request_close(session, [id]);
        }

        if menu.close_all {
            self.request_close(session, session.state.document_ids());
        }

        if self.map_keys_enabled(ui, session) && settings.keybindings.get(KeybindAction::Screenshot).is_pressed(ui) {
            menu.screenshot = Some(ScreenshotRequest {
                area: ScreenshotArea::Map,
                copy: false,
            });
        }
    }

    fn apply_search_actions(&mut self, ui: &Ui, session: &mut Session, settings: &Settings, menu: &mut MenuActions) {
        let is_map_keys_enabled = self.map_keys_enabled(ui, session);
        menu.search |= is_map_keys_enabled && settings.keybindings.get(KeybindAction::Find).is_pressed(ui);
        menu.go_to |= is_map_keys_enabled && settings.keybindings.get(KeybindAction::GoTo).is_pressed(ui);
        if menu.search {
            match session
                .state
                .active_document()
                .and_then(|document| Some((document.id(), document.selected_instance()?)))
            {
                Some((document, instance)) => {
                    self.find
                        .open_for(session, document, instance, find::SimilarMatchKind::Prefab);
                },
                None => self.find.request(),
            }
        }

        if !is_map_keys_enabled {
            return;
        }

        let next = settings
            .keybindings
            .get(KeybindAction::FindNext)
            .is_pressed_repeating(ui);
        let previous = settings
            .keybindings
            .get(KeybindAction::FindPrevious)
            .is_pressed_repeating(ui);
        if (next || previous)
            && let Some(target) = self.find.step(session, next)
        {
            self.jump_to_instance(session, target);
        }
    }

    fn apply_view_toggles(&mut self, session: &mut Session, settings: &mut Settings, menu: &MenuActions) {
        if menu.toggle_areas {
            session.toggle_areas();
        }

        if menu.toggle_area_outlines {
            session.toggle_area_outlines();
        }

        if menu.toggle_lighting {
            session.toggle_lighting();
        }

        if menu.toggle_tile_grid {
            settings.show_tile_grid = !settings.show_tile_grid;
        }

        if menu.toggle_pixel_grid {
            settings.show_selected_pixel_grid = !settings.show_selected_pixel_grid;
        }

        if menu.level_delta != 0 {
            request_level_change(session, menu.level_delta, &mut self.new_level_dialog);
        }

        if let Some(depth) = menu.underlay_depth {
            session.set_underlay_depth(depth);
        }
    }

    fn open_save_dialog(&mut self, ui: &Ui, session: &Session, error: Option<String>) {
        let Some(document) = session.state.active() else { return };
        self.save_dialog = Some(SaveDialog {
            document,
            close_after_save: false,
            path: session
                .map_path()
                .map(|path| path.display().to_string())
                .unwrap_or_default(),
            format: session.map_format().unwrap_or_default(),
            error,
        });
        ui.open_popup(SAVE_MAP_POPUP);
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
            self.git_panel.request(git::GitTab::Conflicts);
        }

        self.panel_focus_requested =
            self.find.has_focus_request() || self.git_panel.has_focus_request() || self.close_focus.is_some();
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

        if self.find.has_focus_request() || !panels.inspector {
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
        self.copy_to_clipboard = inspector.copy_hash.or(object_tree.copy_path);
        if let Some((document, instance)) = inspector.find_similar {
            self.find
                .open_for(session, document, instance, find::SimilarMatchKind::Prefab);
        }

        panels.search |= self.find.has_focus_request();
        if let Some(target) = self.find.draw(ui, session, settings, &mut panels.search) {
            self.jump_to_instance(session, target);
        }

        let git_output = self.git_panel.draw(ui, session, settings, &mut panels.git);
        settings.panels = panels;
        if git_output.copy.is_some() {
            self.copy_to_clipboard = git_output.copy;
        }

        if let Some((id, coord)) = git_output.center {
            self.center_view_on(session, id, coord);
        }

        if let Some(id) = git_output.load_conflicts {
            self.pending_conflict_reload = Some(id);
        }

        inspector.open_source.or(object_tree.open_source)
    }

    fn draw_reload_conflicts_popup(&mut self, ui: &Ui, session: &Session) -> Option<DocumentId> {
        let id = self.pending_conflict_reload?;
        if !session.state.document(id).is_some_and(MapDocument::is_dirty) {
            self.pending_conflict_reload = None;
            return Some(id);
        }

        if !ui.is_popup_open(RELOAD_CONFLICTS_POPUP) {
            ui.open_popup(RELOAD_CONFLICTS_POPUP);
        }

        let _modal = common::begin_centered_modal(
            ui,
            RELOAD_CONFLICTS_POPUP,
            WindowFlags::ALWAYS_AUTO_RESIZE | WindowFlags::NO_SAVED_SETTINGS,
        )?;
        ui.text("Discard unsaved changes and load conflicts from Git?");
        let mut load_conflicts = None;
        if ui.button("Discard and load") {
            load_conflicts = Some(id);
            self.pending_conflict_reload = None;
            ui.close_current_popup();
        }

        ui.same_line();
        if ui.button("Cancel") {
            self.pending_conflict_reload = None;
            ui.close_current_popup();
        }

        load_conflicts
    }

    fn draw_new_map(&mut self, ui: &Ui, session: &mut Session, open: bool) -> bool {
        if open {
            self.new_map_dialog = Some(NewMapDialog::default());
            ui.open_popup(NEW_MAP_POPUP);
        }

        let (pick_path, created) = draw_new_map_dialog(ui, session, &mut self.new_map_dialog);
        if created {
            self.show_welcome = false;
            self.request_refit(None);
        }

        pick_path
    }

    fn draw_load_window(&mut self, ui: &Ui, load: Option<&LoadView>) -> LoadPopup {
        let popup = draw_load_popup(
            ui,
            &self.load_window,
            load,
            self.load_notice.as_mut(),
            &self.diagnostics,
        );
        if popup.dismiss {
            if self.load_notice.is_some() {
                self.load_notice = None;
            } else {
                self.diagnostics.open = None;
            }
        }

        popup
    }

    fn draw_open_maps(
        &mut self, ui: &Ui, session: &mut Session, settings: &mut Settings, refit: bool,
    ) -> (Vec<VisibleMapView>, Option<usize>) {
        self.follow_coop_peer(ui, session);
        let (map_views, picking, coop) = if session.state.is_empty() {
            (Vec::new(), None, viewport::CoopPresence::default())
        } else {
            self.draw_map_views(ui, session, settings, refit)
        };
        self.stop_following_when_moved(session);
        session.coop_cursor(coop.cursor);
        session.coop_view(coop.view);
        session.coop_selection(coop.selection);
        self.edit_command = None;

        (map_views, picking)
    }

    fn draw_map_dialogs(&mut self, ui: &Ui, session: &mut Session, menu: &MenuActions) {
        draw_new_level_dialog(ui, session, &mut self.new_level_dialog, &mut self.tile_fill);
        if let Some(state) = &self.delete_level_dialog
            && state.open
        {
            self.cancel_edit_gestures(session, Some(state.document));
        }

        draw_delete_level_dialog(ui, session, &mut self.delete_level_dialog);

        if menu.resize_map
            && let Some(size) = session.map().map(|map| map.size())
        {
            self.cancel_edit_gestures(session, session.state.active());
            let fill = TileFillSearch::new(session, self.tile_fill.as_ref());
            self.resize_map_dialog = Some(ResizeMapDialog::new(size, fill));
            ui.open_popup(RESIZE_MAP_POPUP);
        }

        if draw_resize_map_dialog(ui, session, &mut self.resize_map_dialog, &mut self.tile_fill) {
            self.request_refit(session.state.active());
        }

        if menu.go_to
            && let Some(size) = session.map().map(|map| map.size())
        {
            let (x, y) = self
                .last_go_to
                .map_or((size.x.div_ceil(2), size.y.div_ceil(2)), |last| (last.x, last.y));
            self.go_to_dialog = Some(GoToDialog::new(Coord::new(x, y, session.z())));
            ui.open_popup(GO_TO_POPUP);
        }

        if let Some(coord) = draw_go_to_dialog(ui, session, &mut self.go_to_dialog)
            && let Some(document) = session.state.active()
        {
            self.last_go_to = Some(coord);
            self.go_to_tile(session, document, coord, ui.time());
        }
    }

    fn draw_coop_dialogs(
        &mut self, ui: &Ui, session: &mut Session, settings: &mut Settings, loading: bool,
        kind: Option<coop::CoopDialogKind>,
    ) -> Option<OpenRequest> {
        if let Some(kind) = kind {
            let dialog = coop::CoopDialog::new(kind, settings);
            ui.open_popup(dialog.popup());
            self.coop_dialog = Some(dialog);
        }

        coop::draw_coop_dialog(ui, session, settings, &mut self.coop_dialog);

        match coop::draw_out_of_date_dialog(ui, session) {
            Some((id, coop::OutOfDateChoice::SaveMine)) => {
                session.set_active_document(id);
                self.save_requested = true;
            },
            Some((id, coop::OutOfDateChoice::DiscardMine)) => session.discard_for_coop_map(id),
            Some((_, coop::OutOfDateChoice::Leave)) => session.leave_coop(),
            None => {},
        }

        match coop::draw_coop_notice(ui, session.coop(), loading, &mut self.coop_notice) {
            Some(coop::CoopNoticeChoice::Retry) => {
                match session.retry_coop() {
                    Ok(()) => self.coop_notice.reset(),
                    Err(error) => self.open_error = Some(error),
                }

                None
            },
            Some(coop::CoopNoticeChoice::ReloadAndRetry) => Some(OpenRequest::ReloadCoop),
            Some(coop::CoopNoticeChoice::Dismiss) => {
                session.leave_coop();
                None
            },
            None => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use core::path::TreePath;
    use std::path::PathBuf;

    use dear_imgui_rs::DockLayout;
    use dmm::{Prefab, Size};
    use editor::tool::FillMode;

    use super::{
        BlockSelectionOptions,
        DEFAULT_CUSTOM_FILL_BOUNDARY,
        IMGUI_CONTEXT,
        UiState,
        fixtures::rectangle_context,
        git,
        menu::MenuActions,
    };
    use crate::{
        session::{DiagnosticSeverity, Session, fixtures::install_diff},
        settings::{KeyBinding, KeyBindings, KeybindAction, OpenPanels, Panel, Settings},
        theme::{DEFAULT_THEME, Themes},
    };

    #[test]
    fn the_default_dock_layout_is_valid() {
        let state = UiState::new(false).expect("valid window keys");

        assert_eq!(state.layout.validate(), Ok(()));
        assert_eq!(state.block_selection_options, BlockSelectionOptions::default());
        assert_eq!(state.fill_mode, FillMode::Wall);
        assert_eq!(
            state.custom_fill_boundaries,
            [TreePath::parse(DEFAULT_CUSTOM_FILL_BOUNDARY)]
        );
        assert!(state.custom_fill_search.is_empty());
        assert!(state.pending_fill_warning.is_none());
        assert!(state.new_map_dialog.is_none());
        assert!(state.load_notice.is_none());
        assert_eq!(state.diagnostics.count(DiagnosticSeverity::Error), 0);
        assert!(state.save_dialog.is_none());
        assert!(!state.exit_requested);
        assert!(state.show_welcome);
        assert!(state.open_error.is_none());
        assert!(!state.keybind_preset_prompt);
    }

    #[test]
    fn the_central_node_holds_only_the_welcome_window() {
        let state = UiState::new(false).expect("valid window keys");
        let DockLayout::Split { second, .. } = &state.layout else {
            panic!("the root is split between the object tree and everything else");
        };
        let DockLayout::Split { second, .. } = second.as_ref() else {
            panic!("the remainder is split between the inspector and the central node");
        };

        // Map views are minted per open map and dock themselves in at runtime,
        // so the declared layout cannot name them.
        assert_eq!(second.as_ref(), &DockLayout::tabs([&state.welcome_window]));
    }

    #[test]
    fn the_git_panel_shares_the_object_tree_dock_node() {
        let state = UiState::new(false).expect("valid window keys");
        let DockLayout::Split { first, .. } = &state.layout else {
            panic!("the root is split between the object tree and everything else");
        };

        assert_eq!(
            first.as_ref(),
            &DockLayout::tabs([state.object_tree.window(), state.git_panel.window()])
        );
    }

    #[test]
    fn the_search_panel_shares_the_inspector_dock_node() {
        let state = UiState::new(false).expect("valid window keys");
        let DockLayout::Split { second, .. } = &state.layout else {
            panic!("the root is split between the object tree and everything else");
        };
        let DockLayout::Split { first, .. } = second.as_ref() else {
            panic!("the remainder is split between the inspector and the central node");
        };

        assert_eq!(
            first.as_ref(),
            &DockLayout::tabs([state.inspector.window(), state.find.window()])
        );
    }

    #[test]
    fn the_inspector_tab_starts_in_front_of_the_search_tab() {
        let _guard = IMGUI_CONTEXT.lock().unwrap();
        let mut context = rectangle_context();
        let flags = context.io().config_flags() | dear_imgui_rs::ConfigFlags::DOCKING_ENABLE;
        context.io_mut().set_config_flags(flags);
        let mut state = UiState::new(false).unwrap();
        let mut session = Session::new();
        let mut themes = Themes::load(None, context.style(), DEFAULT_THEME);
        let mut settings = Settings::default();
        let mut frame = |state: &mut UiState| {
            let ui = context.frame();
            state.draw(ui, &mut session, &mut settings, &mut themes, None).unwrap();
            assert!(context.render_legacy().valid());
        };

        for _ in 0..4 {
            frame(&mut state);
        }
        assert!(
            !state.select_inspector.requested(),
            "the inspector docked and took the front"
        );
        assert!(!state.find.visible(), "the Search tab waits behind the inspector");

        state.find.request();
        for _ in 0..2 {
            frame(&mut state);
        }
        assert!(state.find.visible(), "a request brings the Search tab forward");
    }

    fn draw_frames(context: &mut dear_imgui_rs::Context, state: &mut UiState, settings: &mut Settings, frames: usize) {
        let mut session = Session::new();
        let mut themes = Themes::load(None, context.style(), DEFAULT_THEME);
        for _ in 0..frames {
            let ui = context.frame();
            state.draw(ui, &mut session, settings, &mut themes, None).unwrap();
            assert!(context.render_legacy().valid());
        }
    }

    fn docking_context() -> dear_imgui_rs::Context {
        let mut context = rectangle_context();
        let flags = context.io().config_flags() | dear_imgui_rs::ConfigFlags::DOCKING_ENABLE;
        context.io_mut().set_config_flags(flags);
        context
    }

    /// Whether ImGui drew the window last frame, and whether it sits in a dock node.
    fn window_status(key: &dear_imgui_rs::WindowKey) -> (bool, bool) {
        let name = std::ffi::CString::new(format!("###{}", key.stable_id())).unwrap();
        // SAFETY: the test's context is current between frames, and the name is NUL-terminated.
        unsafe {
            let window = dear_imgui_rs::sys::igFindWindowByID(dear_imgui_rs::sys::igImHashStr(name.as_ptr(), 0, 0));
            if window.is_null() {
                return (false, false);
            }

            let node = dear_imgui_rs::sys::igDockBuilderGetNode((*window).DockId);

            ((*window).Active, !node.is_null())
        }
    }

    fn toggle(panel: Panel) -> MenuActions {
        MenuActions {
            toggle_panel: Some(panel),
            ..MenuActions::default()
        }
    }

    #[test]
    fn a_closed_panel_reopens_from_the_window_menu_in_its_dock() {
        let _guard = IMGUI_CONTEXT.lock().unwrap();
        let mut context = docking_context();
        let mut state = UiState::new(false).unwrap();
        let mut settings = Settings::default();
        draw_frames(&mut context, &mut state, &mut settings, 4);
        assert_eq!(window_status(state.inspector.window()), (true, true));

        state.apply_window_actions(&mut settings, &toggle(Panel::Inspector));
        state.apply_window_actions(&mut settings, &toggle(Panel::Search));
        draw_frames(&mut context, &mut state, &mut settings, 2);
        assert!(!settings.panels.inspector && !settings.panels.search);
        assert!(
            !window_status(state.inspector.window()).0,
            "a closed inspector is not drawn"
        );
        assert!(!state.find.visible());

        state.apply_window_actions(&mut settings, &toggle(Panel::Inspector));
        draw_frames(&mut context, &mut state, &mut settings, 4);
        assert!(settings.panels.inspector);
        assert_eq!(
            window_status(state.inspector.window()),
            (true, true),
            "the inspector comes back docked"
        );
        assert!(!state.select_inspector.requested());
    }

    #[test]
    fn a_closed_panel_releases_startup_focus() {
        let _guard = IMGUI_CONTEXT.lock().unwrap();
        let mut context = docking_context();
        let mut state = UiState::new(false).unwrap();
        let mut settings = Settings::default();
        settings.panels.object_tree = false;
        settings.panels.inspector = false;
        draw_frames(&mut context, &mut state, &mut settings, 1);

        assert!(!state.select_object_tree.requested());
        assert!(!state.select_inspector.requested());
    }

    #[test]
    fn a_search_request_reopens_the_search_panel() {
        let _guard = IMGUI_CONTEXT.lock().unwrap();
        let mut context = docking_context();
        let mut state = UiState::new(false).unwrap();
        let mut settings = Settings::default();
        settings.panels.search = false;
        draw_frames(&mut context, &mut state, &mut settings, 4);
        assert!(!state.find.visible());

        state.find.request();
        draw_frames(&mut context, &mut state, &mut settings, 2);
        assert!(settings.panels.search);
        assert!(state.find.visible());
    }

    #[test]
    fn resetting_the_layout_reopens_every_panel() {
        let mut state = UiState::new(false).unwrap();
        let mut settings = Settings::default();
        for panel in Panel::ALL {
            state.apply_window_actions(&mut settings, &toggle(panel));
        }
        assert!(Panel::ALL.into_iter().all(|panel| !settings.panels.is_open(panel)));

        let reset = MenuActions {
            reset_layout: true,
            ..MenuActions::default()
        };
        state.apply_window_actions(&mut settings, &reset);

        assert_eq!(settings.panels, OpenPanels::default());
        assert!(state.reset_layout);
    }

    #[test]
    fn restored_floating_panels_release_startup_focus() {
        let _guard = IMGUI_CONTEXT.lock().unwrap();

        for panel in ["inspector", "object-tree"] {
            let mut saved = {
                let mut context = rectangle_context();
                let flags = context.io().config_flags() | dear_imgui_rs::ConfigFlags::DOCKING_ENABLE;
                context.io_mut().set_config_flags(flags);
                let mut state = UiState::new(false).unwrap();
                let mut session = Session::new();
                let mut themes = Themes::load(None, context.style(), DEFAULT_THEME);
                let mut settings = Settings::default();
                for _ in 0..4 {
                    let ui = context.frame();
                    state.draw(ui, &mut session, &mut settings, &mut themes, None).unwrap();
                    assert!(context.render_legacy().valid());
                }
                let mut ini = String::new();
                context.save_ini_settings(&mut ini);
                ini
            };

            // A detached panel has a saved window entry without a dock node.
            let header = format!("[Window][{panel}]");
            let start = saved.find(&header).expect("panel saved in the layout");
            let end = saved[start..].find("\n\n").map_or(saved.len(), |offset| start + offset);
            let section = &saved[start..end];
            assert!(section.contains("DockId="));
            let floating = section
                .lines()
                .filter(|line| !line.starts_with("DockId="))
                .collect::<Vec<_>>()
                .join("\n");
            saved.replace_range(start..end, &floating);

            let mut context = rectangle_context();
            let flags = context.io().config_flags() | dear_imgui_rs::ConfigFlags::DOCKING_ENABLE;
            context.io_mut().set_config_flags(flags);
            context.load_ini_settings(&saved);
            let mut state = UiState::new(false).unwrap();
            let mut session = Session::new();
            let mut themes = Themes::load(None, context.style(), DEFAULT_THEME);
            let mut settings = Settings {
                focus_windows_on_hover: false,
                ..Settings::default()
            };
            for _ in 0..4 {
                let ui = context.frame();
                state.draw(ui, &mut session, &mut settings, &mut themes, None).unwrap();
                assert!(context.render_legacy().valid());
            }
            let name = std::ffi::CString::new(format!("###{panel}")).unwrap();
            // SAFETY: the context is current between frames and the window ID is NUL-terminated.
            let window =
                unsafe { dear_imgui_rs::sys::igFindWindowByID(dear_imgui_rs::sys::igImHashStr(name.as_ptr(), 0, 0)) };
            assert!(!window.is_null(), "{panel} was not restored");
            assert_eq!(unsafe { (*window).DockId }, 0, "{panel} did not restore floating");
            let focus_pending = match panel {
                "inspector" => state.select_inspector.requested(),
                _ => state.select_object_tree.requested(),
            };
            assert!(!focus_pending, "{panel} still requests focus after restoring floating");

            let ui = context.frame();
            state.draw(ui, &mut session, &mut settings, &mut themes, None).unwrap();
            ui.window("focus-sentinel").focused(true).build(|| {});
            assert!(context.render_legacy().valid());

            let ui = context.frame();
            state.draw(ui, &mut session, &mut settings, &mut themes, None).unwrap();
            let mut retained_focus = false;
            ui.window("focus-sentinel").build(|| {
                retained_focus = ui.is_window_focused();
            });
            assert!(context.render_legacy().valid());
            assert!(retained_focus, "{panel} took focus back from another window");
        }
    }

    #[test]
    fn resetting_the_layout_keeps_open_map_views_docked() {
        let _guard = IMGUI_CONTEXT.lock().unwrap();
        let mut context = rectangle_context();
        let flags = context.io().config_flags() | dear_imgui_rs::ConfigFlags::DOCKING_ENABLE;
        context.io_mut().set_config_flags(flags);
        let mut state = UiState::new(false).unwrap();
        state.show_welcome = false;
        let mut session = Session::new();
        let mut themes = Themes::load(None, context.style(), DEFAULT_THEME);
        session.apply_map(crate::loader::LoadedMap {
            path: PathBuf::from("reset-layout-test.dmm"),
            map: dmm::Map::new(dmm::Size { x: 10, y: 10, z: 1 }),
            z: 1,
            errors: vec![],
            repo: None,
            conflict: None,
        });
        let mut settings = Settings::default();
        let mut frame = |state: &mut UiState| {
            let ui = context.frame();
            state.draw(ui, &mut session, &mut settings, &mut themes, None).unwrap();
            assert!(context.render_legacy().valid());
        };

        for _ in 0..4 {
            frame(&mut state);
        }
        assert!(state.central_node.is_some(), "the map view starts docked");

        state.reset_layout = true;
        state.central_node = None;
        for _ in 0..3 {
            frame(&mut state);
        }
        assert!(!state.reset_layout);
        assert!(
            state.central_node.is_some(),
            "the map view is docked again after the reset"
        );
    }

    /// Whether ImGui has the window docked in the dockspace's central node.
    fn docked_in_the_center(key: &dear_imgui_rs::WindowKey) -> bool {
        let name = std::ffi::CString::new(format!("###{}", key.stable_id())).unwrap();
        // SAFETY: the test's context is current between frames, and the name is NUL-terminated.
        unsafe {
            let window = dear_imgui_rs::sys::igFindWindowByID(dear_imgui_rs::sys::igImHashStr(name.as_ptr(), 0, 0));
            if window.is_null() {
                return false;
            }

            let node = dear_imgui_rs::sys::igDockBuilderGetNode((*window).DockId);

            !node.is_null() && dear_imgui_rs::sys::ImGuiDockNode_IsCentralNode(node)
        }
    }

    #[test]
    fn a_new_map_view_ignores_a_saved_dock_node_that_no_longer_exists() {
        let _guard = IMGUI_CONTEXT.lock().unwrap();
        let mut context = rectangle_context();
        let flags = context.io().config_flags() | dear_imgui_rs::ConfigFlags::DOCKING_ENABLE;
        context.io_mut().set_config_flags(flags);
        let mut state = UiState::new(false).unwrap();
        let mut session = Session::new();
        let mut themes = Themes::load(None, context.style(), DEFAULT_THEME);
        let mut settings = Settings::default();
        for _ in 0..3 {
            let ui = context.frame();
            state.draw(ui, &mut session, &mut settings, &mut themes, None).unwrap();
            assert!(context.render_legacy().valid());
        }

        state.reset_layout = true;
        session.apply_map(crate::loader::LoadedMap {
            path: PathBuf::from("saved-dock.dmm"),
            map: dmm::Map::new(dmm::Size { x: 10, y: 10, z: 1 }),
            z: 1,
            errors: vec![],
            repo: None,
            conflict: None,
        });
        let id = session.state.active().unwrap();
        // map view keys restart with the document ids every launch, so the ini remembers where an
        // earlier session's view with this number was docked, in a node the reset does not rebuild
        context.load_ini_settings(&format!(
            "[Window][###viewport-{}]\nPos=0,0\nSize=400,300\nCollapsed=0\nDockId=0x0000ABCD,0\n",
            id.get()
        ));
        for _ in 0..4 {
            let ui = context.frame();
            state.draw(ui, &mut session, &mut settings, &mut themes, None).unwrap();
            assert!(context.render_legacy().valid());
        }

        assert!(
            docked_in_the_center(state.map_views[&id].window()),
            "the new map view floats"
        );
    }

    #[test]
    fn a_search_request_brings_its_tab_forward_while_the_mouse_is_over_the_map() {
        let _guard = IMGUI_CONTEXT.lock().unwrap();
        let mut context = rectangle_context();
        let flags = context.io().config_flags() | dear_imgui_rs::ConfigFlags::DOCKING_ENABLE;
        context.io_mut().set_config_flags(flags);
        let mut state = UiState::new(false).unwrap();
        let mut session = Session::new();
        let mut themes = Themes::load(None, context.style(), DEFAULT_THEME);
        session.apply_map(crate::loader::LoadedMap {
            path: PathBuf::from("search-focus-test.dmm"),
            map: dmm::Map::new(dmm::Size { x: 10, y: 10, z: 1 }),
            z: 1,
            errors: vec![],
            repo: None,
            conflict: None,
        });
        let mut settings = Settings::default();
        assert!(settings.focus_windows_on_hover);
        let mut frame = |state: &mut UiState| {
            context.io_mut().add_mouse_pos_event([400.0, 300.0]);
            let ui = context.frame();
            state.draw(ui, &mut session, &mut settings, &mut themes, None).unwrap();
            assert!(context.render_legacy().valid());
        };

        for _ in 0..4 {
            frame(&mut state);
        }
        assert!(!state.find.visible());

        state.find.request();
        for _ in 0..3 {
            frame(&mut state);
        }
        assert!(state.find.visible(), "the map view must not take the focus back first");
    }

    #[test]
    fn the_object_tree_tab_starts_in_front_of_the_git_tab() {
        let _guard = IMGUI_CONTEXT.lock().unwrap();
        let mut context = rectangle_context();
        let flags = context.io().config_flags() | dear_imgui_rs::ConfigFlags::DOCKING_ENABLE;
        context.io_mut().set_config_flags(flags);
        let mut state = UiState::new(false).unwrap();
        let mut session = Session::new();
        let mut themes = Themes::load(None, context.style(), DEFAULT_THEME);
        let mut settings = Settings::default();
        let mut frame = |state: &mut UiState| {
            let ui = context.frame();
            state.draw(ui, &mut session, &mut settings, &mut themes, None).unwrap();
            assert!(context.render_legacy().valid());
        };

        for _ in 0..4 {
            frame(&mut state);
        }
        assert!(
            !state.select_object_tree.requested(),
            "the object tree docked and took the front"
        );
        assert!(!state.git_panel.visible(), "the Git tab waits behind the object tree");

        state.git_panel.request(git::GitTab::Conflicts);
        for _ in 0..2 {
            frame(&mut state);
        }
        assert!(state.git_panel.visible(), "a request brings the Git tab forward");
    }

    #[test]
    fn the_diff_tab_draws_the_versions_and_the_changes() {
        let _guard = IMGUI_CONTEXT.lock().unwrap();
        let mut context = rectangle_context();
        let flags = context.io().config_flags() | dear_imgui_rs::ConfigFlags::DOCKING_ENABLE;
        context.io_mut().set_config_flags(flags);
        let mut state = UiState::new(false).unwrap();
        let mut session = Session::new();
        let mut settings = Settings::default();
        let mut map = dmm::Map::new(Size { x: 4, y: 4, z: 1 });
        let floor = map.intern_tile(vec![Prefab::new(TreePath::parse("/turf/floor"))]);
        for row in &mut map.grid[0] {
            row.fill(floor);
        }
        session.apply_map(crate::loader::LoadedMap {
            path: PathBuf::from("diff-ui-test.dmm"),
            map,
            z: 1,
            errors: vec![],
            repo: Some(editor::git::RepoPath {
                root: PathBuf::from("missing-repository"),
                git_dir: PathBuf::from("missing-repository/.git"),
                rel: String::from("diff-ui-test.dmm"),
            }),
            conflict: None,
        });
        let id = session.state.active().unwrap();
        install_diff(&mut session, id);
        let mut frame = |state: &mut UiState, session: &mut Session| {
            let ui = context.frame();
            let mut themes = Themes::load(None, &ui.clone_style(), DEFAULT_THEME);
            state.draw(ui, session, &mut settings, &mut themes, None).unwrap();
            assert!(context.render_legacy().valid());
        };

        // the dock layout settles before the Git tab can be brought forward
        for _ in 0..4 {
            frame(&mut state, &mut session);
        }
        state.git_panel.request(git::GitTab::Diff);
        for _ in 0..3 {
            frame(&mut state, &mut session);
        }

        assert!(state.git_panel.visible());
        assert_eq!(session.diff(id).map(|diff| diff.len()), Some(1));
    }

    #[test]
    fn an_out_of_date_map_asks_in_a_popup_until_the_user_leaves() {
        use std::{
            thread,
            time::{Duration, Instant},
        };

        let _guard = IMGUI_CONTEXT.lock().unwrap();
        let entry = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/env/test.dme");
        let open = |width: u32| {
            let mut session = Session::new();
            session.load_environment(&entry).unwrap();
            let path = session.codebase_dir().unwrap().join("_maps/out-of-date.dmm");
            session.apply_map(crate::loader::LoadedMap {
                path,
                map: dmm::Map::new(Size { x: width, y: 1, z: 1 }),
                z: 1,
                errors: vec![],
                repo: None,
                conflict: None,
            });
            let id = session.state.active().unwrap();

            (session, id)
        };
        let poll_until = |sessions: &mut [&mut Session], done: &dyn Fn(&[&mut Session]) -> bool| {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !done(sessions) {
                assert!(Instant::now() < deadline, "co-op never got there");
                for session in sessions.iter_mut() {
                    session.poll_coop();
                }

                thread::sleep(Duration::from_millis(5));
            }
        };

        let (mut host, _) = open(2);
        host.host_coop(0, net::hash_password("hunter2"), String::from("host"))
            .unwrap();
        poll_until(&mut [&mut host], &|sessions| {
            sessions[0].coop().is_some_and(|coop| coop.is_connected())
        });
        host.share_coop_map();
        poll_until(&mut [&mut host], &|sessions| {
            sessions[0].coop().is_some_and(|coop| !coop.shared_maps.is_empty())
        });

        let (mut guest, local) = open(1);
        guest.state.document_mut(local).unwrap().mark_unsaved();
        let port = host.coop().and_then(|coop| coop.host()).unwrap().port();
        guest
            .join_coop(
                format!("127.0.0.1:{port}"),
                net::hash_password("hunter2"),
                String::from("guest"),
            )
            .unwrap();
        poll_until(&mut [&mut host, &mut guest], &|sessions| {
            sessions[1].coop_out_of_date(local).is_some()
        });

        let mut context = rectangle_context();
        let flags = context.io().config_flags() | dear_imgui_rs::ConfigFlags::DOCKING_ENABLE;
        context.io_mut().set_config_flags(flags);
        let mut state = UiState::new(false).unwrap();
        let mut settings = Settings {
            check_for_updates: false,
            ..Settings::default()
        };
        let mut frame = |state: &mut UiState, session: &mut Session| {
            let ui = context.frame();
            let mut themes = Themes::load(None, &ui.clone_style(), DEFAULT_THEME);
            state.draw(ui, session, &mut settings, &mut themes, None).unwrap();
            let is_open = ui.is_popup_open(super::coop::OUT_OF_DATE_POPUP);
            assert!(context.render_legacy().valid());
            is_open
        };
        assert!((0..3).map(|_| frame(&mut state, &mut guest)).last().unwrap());

        guest.leave_coop();
        assert!(!(0..3).map(|_| frame(&mut state, &mut guest)).last().unwrap());
        let document = guest.state.document(local).unwrap();
        assert!(document.is_dirty());
        assert!(!document.is_read_only());
        assert_eq!(document.map.size().x, 1);
    }

    #[test]
    fn the_codebase_dialog_supports_retry_close_and_escape_without_maps() {
        use dear_imgui_rs::{Key, MouseButton, sys};

        use super::OpenRequest;
        let _guard = IMGUI_CONTEXT.lock().unwrap();
        let mut context = rectangle_context();
        let flags = context.io().config_flags() | dear_imgui_rs::ConfigFlags::DOCKING_ENABLE;
        context.io_mut().set_config_flags(flags);
        let mut state = UiState::new(false).unwrap();
        let mut session = Session::new();
        let original = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/env/test.dme");
        session.load_environment(&original).unwrap();
        session
            .join_coop("not a host".into(), net::hash_password("password"), "guest".into())
            .unwrap();
        let dir = std::env::temp_dir().join(format!("rmd-codebase-notice-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("game.dme"), "/obj/different").unwrap();
        session.load_environment(&dir.join("game.dme")).unwrap();
        assert!(matches!(
            session.coop().unwrap().status,
            crate::session::CoopStatus::CodebaseMismatch { .. }
        ));
        assert!(session.state.is_empty());
        let mut settings = Settings {
            check_for_updates: false,
            ..Settings::default()
        };
        let load = crate::loader::LoadView {
            title: "Opening codebase",
            path: "game.dme".into(),
            snapshot: editor::progress::Progress::new().snapshot(),
            cancelling: false,
            cancellable: true,
        };
        let frame = |context: &mut dear_imgui_rs::Context,
                     state: &mut UiState,
                     session: &mut Session,
                     settings: &mut Settings,
                     loading: bool| {
            let ui = context.frame();
            let mut themes = Themes::load(None, &ui.clone_style(), DEFAULT_THEME);
            let output = state
                .draw(ui, session, settings, &mut themes, loading.then_some(&load))
                .unwrap();
            let is_open = ui.is_popup_open(super::coop::STATUS_POPUP);
            assert!(context.render_legacy().valid());
            (output.open, is_open)
        };
        for _ in 0..4 {
            assert_eq!(
                frame(&mut context, &mut state, &mut session, &mut settings, true),
                (None, false),
                "the dialog waits for loading to finish"
            );
        }

        let work_pos = context.main_viewport().work_pos();
        for _ in 0..4 {
            assert_eq!(
                frame(&mut context, &mut state, &mut session, &mut settings, false),
                (None, true)
            );
        }

        let (position, size) = unsafe {
            let window = sys::igFindWindowByName(c"Co-op unavailable##coop-status".as_ptr());
            assert!(!window.is_null());
            assert_ne!((*window).Flags & sys::ImGuiWindowFlags_Modal, 0);
            ([(*window).Pos.x, (*window).Pos.y], [(*window).Size.x, (*window).Size.y])
        };
        assert_eq!(
            context.main_viewport().work_pos(),
            work_pos,
            "the modal preserves the dockspace layout"
        );
        let padding = context.style().window_padding();
        let close = [position[0] + padding[0] + 8.0, position[1] + size[1] - padding[1] - 8.0];
        let retry = [close[0] + 80.0, close[1]];
        for down in [false, true, false] {
            context.io_mut().add_mouse_pos_event(retry);
            context.io_mut().add_mouse_button_event(MouseButton::Left, down);
            assert_eq!(
                frame(&mut context, &mut state, &mut session, &mut settings, true),
                (None, true),
                "retry is disabled while loading"
            );
        }

        let mut requested = None;
        for down in [false, true, false] {
            context.io_mut().add_mouse_pos_event(retry);
            context.io_mut().add_mouse_button_event(MouseButton::Left, down);
            requested = frame(&mut context, &mut state, &mut session, &mut settings, false)
                .0
                .or(requested);
        }

        assert_eq!(requested, Some(OpenRequest::ReloadCoop));
        for _ in 0..2 {
            assert_eq!(
                frame(&mut context, &mut state, &mut session, &mut settings, false),
                (None, false),
                "the previous failure must not reopen during a retry"
            );
        }

        state.reset_coop_notice();
        for _ in 0..3 {
            assert_eq!(
                frame(&mut context, &mut state, &mut session, &mut settings, false),
                (None, true),
                "a repeated mismatch opens the dialog for the new attempt"
            );
        }

        for down in [false, true, false] {
            context.io_mut().add_mouse_pos_event(close);
            context.io_mut().add_mouse_button_event(MouseButton::Left, down);
            frame(&mut context, &mut state, &mut session, &mut settings, false);
        }

        assert!(session.coop().is_none());

        session
            .join_coop("not a host".into(), net::hash_password("password"), "guest".into())
            .unwrap();
        session.load_environment(&original).unwrap();
        for _ in 0..3 {
            assert_eq!(
                frame(&mut context, &mut state, &mut session, &mut settings, false),
                (None, true)
            );
        }

        context.io_mut().add_key_event(Key::Escape, true);
        assert_eq!(
            frame(&mut context, &mut state, &mut session, &mut settings, false),
            (None, false)
        );
        assert!(session.coop().is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn editing_commands_are_bound_to_the_usual_chords() {
        let bindings = KeyBindings::default();

        assert_eq!(
            bindings.get(KeybindAction::Save),
            KeyBinding::with_ctrl(dear_imgui_rs::Key::S)
        );
        assert_eq!(
            bindings.get(KeybindAction::Undo),
            KeyBinding::with_ctrl(dear_imgui_rs::Key::Z)
        );
        assert_eq!(
            bindings.get(KeybindAction::Redo),
            KeyBinding::with_ctrl(dear_imgui_rs::Key::Y)
        );
        assert_eq!(
            bindings.get(KeybindAction::Copy),
            KeyBinding::with_ctrl(dear_imgui_rs::Key::C)
        );
        assert_eq!(
            bindings.get(KeybindAction::Paste),
            KeyBinding::with_ctrl(dear_imgui_rs::Key::V)
        );
        assert_eq!(
            bindings.get(KeybindAction::NodeTool),
            KeyBinding::new(dear_imgui_rs::Key::N)
        );
        // Every action is reachable from the settings list, or it cannot be rebound.
        assert_eq!(KeybindAction::ALL.len(), 56);
        assert_eq!(KeybindAction::RECENT.len(), 10);
        assert!(KeybindAction::ALL.contains(&KeybindAction::Save));
        assert!(KeybindAction::ALL.contains(&KeybindAction::Undo));
        assert!(KeybindAction::ALL.contains(&KeybindAction::Redo));
        assert!(KeybindAction::ALL.contains(&KeybindAction::Copy));
        assert!(KeybindAction::ALL.contains(&KeybindAction::Paste));
        assert!(KeybindAction::ALL.contains(&KeybindAction::NodeTool));
        assert!(KeybindAction::ALL.contains(&KeybindAction::ShowLighting));
        assert!(KeybindAction::ALL.contains(&KeybindAction::Recent1));
        assert!(KeybindAction::ALL.contains(&KeybindAction::Recent0));
    }
}
