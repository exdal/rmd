use core::path::TreePath;
use std::{
    collections::HashMap,
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
use dmm::PrefabInstanceId;
use editor::{
    document::{DocumentId, MapDocument},
    environment::BundledProfile,
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
    session::{LoadReport, Session},
    settings::{KeybindAction, KeybindPreset, Settings},
    update::UpdateCheck,
};

mod blame;
mod block;
mod dialog;
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

pub(crate) use self::load::LoadNotice;
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
        CLOSE_MAP_POPUP,
        FILL_LIMIT_WARNING_POPUP,
        FillWarningContext,
        NEW_MAP_POPUP,
        NewLevelDialog,
        NewMapDialog,
        PendingFillWarning,
        SAVE_ERROR_COLOR,
        SAVE_MAP_POPUP,
        SaveDialog,
        draw_fill_limit_warning,
        draw_keybind_preset_dialog,
        draw_new_level_dialog,
        draw_new_map_dialog,
        draw_save_dialog,
    },
    grid::{draw_selected_pixel_grid, draw_tile_grid},
    history::{draw_history_overlay, recent_button_size},
    load::{DIAGNOSTIC_WARNING_COLOR, DiagnosticsState, draw_load_popup},
    menu::MenuActions,
    node::{NodeOverlayView, NodeRightClick, draw_node_overlay, node_right_click},
    overlay::{
        OVERLAY_BG,
        OVERLAY_PADDING,
        OverlayRect,
        conflict_controls_layout,
        draw_conflict_controls,
        draw_guide_badges,
        draw_highlights,
        draw_overlay_underlay,
        draw_placement_preview,
    },
    paste::{PASTE_LABELS, PasteAction, PendingPaste, centered_paste_min, draw_paste_controls, paste_controls},
    search::{MAX_CUSTOM_FILL_SEARCH_RESULTS, draw_type_path_search, matching_type_paths},
    toolbar::{DEFAULT_CUSTOM_FILL_BOUNDARY, TopOverlayState, draw_top_overlay, request_level_change},
    tooltip::{draw_conflict_tooltip, draw_diff_tooltip},
    viewport::{ActivePlacementFlash, DeletionStroke, MapViewState, PlacementStroke},
    welcome::{ForgetRequest, WelcomeOutput},
};

mod common;

mod context_menu;

mod dm;

mod git;

mod inspector;

mod object_tree;

mod settings;

const DOCKSPACE_ID: &str = "rmd-main-dockspace-v4";

const RELOAD_CONFLICTS_POPUP: &str = "Load merge conflicts##reload-conflicts";

pub struct VisibleMapView {
    pub document: DocumentId,
    pub rect: MapViewRect,
    pub camera: Camera,
    pub interaction: MapViewInteraction,
    pub guide_lines: Vec<GuideLine>,
    pub connected: Vec<PrefabInstanceId>,
}

pub struct UiOutput {
    pub exit: bool,
    pub map_views: Vec<VisibleMapView>,
    pub picking: Option<usize>,
    pub open: Option<OpenRequest>,
    pub open_source: Option<SourceLocation>,
    pub pick_new_map_path: bool,
    pub screenshot: bool,
    pub cancel_load: bool,
    pub copy_to_clipboard: Option<String>,
    pub reload_profile: Option<ProfileReload>,
    pub load_conflicts: Option<DocumentId>,
    pub(crate) keybind_preset: Option<KeybindPreset>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ProfileReload {
    Select(String),
    Force(Option<BundledProfile>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenRequest {
    PickCodebase,
    PickMap,
    Codebase(PathBuf),
    Map(PathBuf),
}

pub struct UiState {
    object_tree: ObjectTreePanel,
    map_views: HashMap<DocumentId, MapViewState>,
    central_node: Option<Id>,
    dockspace_root: Option<Id>,
    dm_ui: dm::DmUi,
    welcome_window: WindowKey,
    inspector: InspectorPanel,
    git_panel: git::GitPanel,
    /// Keeps the object tree in front of the Git tab until it has docked at startup
    select_object_tree: bool,
    settings_window: SettingsWindow,
    layout: DockLayout,
    gizmo: GizmoState,
    gizmo_context: Option<(DocumentId, Tool, u32)>,
    placement_flash: Option<ActivePlacementFlash>,
    placement_stroke: Option<PlacementStroke>,
    deletion_stroke: Option<DeletionStroke>,
    block_selection_options: BlockSelectionOptions,
    fill_mode: FillMode,
    custom_fill_boundaries: Vec<TreePath>,
    custom_fill_search: String,
    pending_fill_warning: Option<PendingFillWarning>,
    popup_was_open: bool,
    new_map_dialog: Option<NewMapDialog>,
    new_level_dialog: Option<NewLevelDialog>,
    new_level_type_path: String,
    save_dialog: Option<SaveDialog>,
    pending_close: Option<DocumentId>,
    pending_conflict_reload: Option<DocumentId>,
    exit_requested: bool,
    show_welcome: bool,
    welcome_map_filter: String,
    welcome_maps_expanded: bool,
    open_error: Option<String>,
    load_window: WindowKey,
    load_notice: Option<LoadNotice>,
    diagnostics: DiagnosticsState,
    load_window_size: [f32; 2],
    keybind_preset_prompt: bool,
    copy_to_clipboard: Option<String>,
    update_check: UpdateCheck,
}

#[cfg(test)]
static IMGUI_CONTEXT: std::sync::Mutex<()> = std::sync::Mutex::new(());

impl UiState {
    pub fn new(keybind_preset_prompt: bool) -> Result<Self, WindowKeyError> {
        let object_tree = ObjectTreePanel::new()?;
        let welcome_window = WindowKey::new("welcome", "Welcome")?;
        let inspector = InspectorPanel::new()?;
        let git_panel = git::GitPanel::new()?;
        let settings_window = SettingsWindow::new()?;
        let load_window = WindowKey::new("load", "Loading")?;
        let layout = DockLayout::split(
            DockSplit::Left,
            0.25,
            DockLayout::tabs([object_tree.window(), git_panel.window()]),
            DockLayout::split(
                DockSplit::Right,
                0.20 / 0.75,
                DockLayout::tabs([inspector.window()]),
                DockLayout::tabs([&welcome_window]),
            ),
        );

        Ok(Self {
            object_tree,
            map_views: HashMap::new(),
            central_node: None,
            dockspace_root: None,
            dm_ui: dm::DmUi::default(),
            welcome_window,
            inspector,
            git_panel,
            select_object_tree: true,
            settings_window,
            layout,
            gizmo: GizmoState::default(),
            gizmo_context: None,
            placement_flash: None,
            placement_stroke: None,
            deletion_stroke: None,
            block_selection_options: BlockSelectionOptions::default(),
            fill_mode: FillMode::default(),
            custom_fill_boundaries: vec![TreePath::parse(DEFAULT_CUSTOM_FILL_BOUNDARY)],
            custom_fill_search: String::new(),
            pending_fill_warning: None,
            popup_was_open: false,
            new_map_dialog: None,
            new_level_dialog: None,
            new_level_type_path: String::new(),
            save_dialog: None,
            pending_close: None,
            pending_conflict_reload: None,
            exit_requested: false,
            show_welcome: true,
            welcome_map_filter: String::new(),
            welcome_maps_expanded: false,
            open_error: None,
            load_window,
            load_notice: None,
            diagnostics: DiagnosticsState::default(),
            load_window_size: [0.0, 0.0],
            keybind_preset_prompt,
            copy_to_clipboard: None,
            update_check: UpdateCheck::default(),
        })
    }

    pub fn set_open_error(&mut self, error: Option<String>) { self.open_error = error; }

    pub fn reveal_selected_instance(&mut self, session: &Session) {
        self.object_tree.reveal_selected_instance(session);
    }

    pub fn request_mouse_popup(&mut self) { self.dm_ui.request_mouse_popup(); }

    pub fn set_load_notice(&mut self, notice: Option<LoadNotice>) { self.load_notice = notice; }

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

    fn cancel_edit_gestures(&mut self, session: &mut Session, id: Option<DocumentId>) {
        self.gizmo.cancel();
        self.placement_flash = None;
        self.placement_stroke = None;
        self.deletion_stroke = None;
        session.cancel_node_drag();

        if let Some(id) = id
            && let Some(view) = self.map_views.get_mut(&id)
        {
            restore_rectangle_gesture(session, id, &mut view.rectangle_gesture);
            view.block_selection_anchor = None;
            view.block_placement = None;
            view.paste = None;
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
        &mut self, ui: &Ui, session: &mut Session, settings: &mut Settings, load: Option<&LoadView>,
    ) -> Result<UiOutput, DockspaceError> {
        let loading = load.is_some() || self.load_notice.is_some();

        if settings.check_for_updates {
            self.update_check.start();
        }

        let root = ui.get_id(DOCKSPACE_ID);
        ui.dockspace()
            .main_viewport()
            .root_id(root)
            .flags(DockNodeFlags::PASSTHRU_CENTRAL_NODE)
            .layout(&self.layout, DockLayoutApply::IfMissing)
            .build()?;
        self.dockspace_root = Some(root);

        if self.keybind_preset_prompt {
            let keybind_preset = draw_keybind_preset_dialog(ui, &mut self.keybind_preset_prompt);
            let exit = self.draw_exit_confirmation(ui, session);

            return Ok(UiOutput {
                exit,
                map_views: Vec::new(),
                picking: None,
                open: None,
                open_source: None,
                pick_new_map_path: false,
                screenshot: false,
                cancel_load: false,
                copy_to_clipboard: None,
                reload_profile: None,
                load_conflicts: None,
                keybind_preset,
            });
        }

        let mut open_new_map_dialog = false;
        let mut pick_new_map_path = false;
        let MenuActions {
            mut open,
            show_welcome,
            mut open_save_dialog,
            mut screenshot,
            toggle_areas,
            toggle_area_outlines,
            toggle_lighting,
            toggle_tile_grid,
            toggle_pixel_grid,
            level_delta,
            underlay_depth,
            refit,
            undo,
            redo,
        } = self.draw_menu_bar(ui, session, settings, loading);

        if session.map().is_some()
            && !open_save_dialog
            && self.save_dialog.is_none()
            && !self.settings_window.is_capturing_keybind()
            && !ui.io().want_text_input()
            && settings.keybindings.get(KeybindAction::Save).is_pressed(ui)
        {
            if session.can_save_map_in_place() {
                if let Err(error) = session.save_map() {
                    self.save_dialog = Some(SaveDialog {
                        path: session
                            .map_path()
                            .map(|path| path.display().to_string())
                            .unwrap_or_default(),
                        format: session.map_format().unwrap_or_default(),
                        error: Some(error.to_string()),
                    });
                    ui.open_popup(SAVE_MAP_POPUP);
                }
            } else {
                open_save_dialog = true;
            }
        }

        screenshot |= session.map().is_some()
            && self.save_dialog.is_none()
            && !self.settings_window.is_capturing_keybind()
            && !ui.io().want_text_input()
            && settings.keybindings.get(KeybindAction::Screenshot).is_pressed(ui);

        if undo || redo {
            self.cancel_edit_gestures(session, session.state.active());
            if undo {
                session.undo();
            } else {
                session.redo();
            }
        }

        if toggle_areas {
            session.toggle_areas();
        }
        if toggle_area_outlines {
            session.toggle_area_outlines();
        }
        if toggle_lighting {
            session.toggle_lighting();
        }
        if toggle_tile_grid {
            settings.show_tile_grid = !settings.show_tile_grid;
        }
        if toggle_pixel_grid {
            settings.show_selected_pixel_grid = !settings.show_selected_pixel_grid;
        }
        if level_delta != 0 {
            request_level_change(
                session,
                level_delta,
                &mut self.new_level_dialog,
                &self.new_level_type_path,
            );
        }
        if let Some(depth) = underlay_depth {
            session.set_underlay_depth(depth);
        }

        if open_save_dialog {
            self.save_dialog = Some(SaveDialog {
                path: session
                    .map_path()
                    .map(|path| path.display().to_string())
                    .unwrap_or_default(),
                format: session.map_format().unwrap_or_default(),
                error: None,
            });
            ui.open_popup(SAVE_MAP_POPUP);
        }
        draw_save_dialog(ui, session, &mut self.save_dialog);

        let settings_output = self.settings_window.draw(ui, session, settings, load.is_some());
        session.sync_git_enabled(settings.git_enabled);
        if settings_output.object_tree_changed {
            self.object_tree.invalidate_filter();
        }
        self.show_welcome |= show_welcome;

        if session.take_loaded_conflicts().is_some() {
            self.git_panel.request(git::GitTab::Conflicts);
        }
        // The Git tab shares the object tree's dock node, a request to show it wins
        if self.git_panel.has_focus_request() {
            self.select_object_tree = false;
        }
        let (mut open_source, object_tree_docked) =
            self.object_tree.draw(ui, session, settings, self.select_object_tree);
        if object_tree_docked {
            self.select_object_tree = false;
        }
        let inspector = self.inspector.draw(ui, session, settings);
        self.copy_to_clipboard = inspector.copy_hash;
        open_source = inspector.open_source.or(open_source);
        if let Some(target) = inspector.jump {
            self.jump_to_instance(session, target);
        }
        let git_output = self.git_panel.draw(ui, session, settings);
        if git_output.copy.is_some() {
            self.copy_to_clipboard = git_output.copy;
        }
        if let Some((id, coord)) = git_output.center {
            session.set_active_document(id);
            session.set_level(coord.z);
            if let Some(view) = self.map_views.get_mut(&id) {
                view.camera.center_on_tile(coord, session.options.tile_size);
                view.refit = false;
                view.focus = true;
            }
        }
        if let Some(id) = git_output.load_conflicts {
            self.pending_conflict_reload = Some(id);
        }
        if let Some(id) = self.pending_conflict_reload
            && session.state.document(id).is_some_and(MapDocument::is_dirty)
            && !ui.is_popup_open(RELOAD_CONFLICTS_POPUP)
        {
            ui.open_popup(RELOAD_CONFLICTS_POPUP);
        }
        let mut load_conflicts = None;
        if let Some(id) = self.pending_conflict_reload {
            if session.state.document(id).is_some_and(MapDocument::is_dirty) {
                if let Some(_modal) = ui
                    .begin_modal_popup_config(RELOAD_CONFLICTS_POPUP)
                    .flags(WindowFlags::ALWAYS_AUTO_RESIZE | WindowFlags::NO_SAVED_SETTINGS)
                    .begin()
                {
                    ui.text("Discard unsaved changes and load conflicts from Git?");
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
                }
            } else {
                load_conflicts = Some(id);
                self.pending_conflict_reload = None;
            }
        }
        let mut welcome = WelcomeOutput::default();
        self.draw_welcome(ui, session, settings, loading, &mut welcome);
        open = welcome.open.or(open);
        open_new_map_dialog |= welcome.new_map_dialog;
        match welcome.forget {
            Some(ForgetRequest::Codebase(path)) => settings.forget_codebase(&path),
            Some(ForgetRequest::Map(path)) => settings.forget_recent(&path),
            None => {},
        }
        if open_new_map_dialog {
            self.new_map_dialog = Some(NewMapDialog::default());
            ui.open_popup(NEW_MAP_POPUP);
        }
        let (pick_path, created) = draw_new_map_dialog(ui, session, &mut self.new_map_dialog);
        pick_new_map_path |= pick_path;
        if created {
            self.show_welcome = false;
            self.request_refit(None);
        }
        let load_popup = draw_load_popup(
            ui,
            &self.load_window,
            &mut self.load_window_size,
            load,
            self.load_notice.as_mut(),
            &self.diagnostics,
        );
        if load_popup.dismiss {
            if self.load_notice.is_some() {
                self.load_notice = None;
            } else {
                self.diagnostics.open = None;
            }
        }

        let (map_views, picking) = if session.state.is_empty() {
            (Vec::new(), None)
        } else {
            self.draw_map_views(ui, session, settings, refit)
        };

        self.dm_ui.draw(ui, session, root.raw());

        draw_new_level_dialog(ui, session, &mut self.new_level_dialog, &mut self.new_level_type_path);

        self.settings_window
            .finish_keybind_capture(ui, &mut settings.keybindings);

        let exit = self.draw_exit_confirmation(ui, session);
        self.popup_was_open = ui.is_popup_open_with_flags("", dear_imgui_rs::PopupQueryFlags::ANY_POPUP);

        Ok(UiOutput {
            exit,
            map_views,
            picking,
            open,
            open_source,
            pick_new_map_path,
            screenshot,
            cancel_load: load_popup.cancel,
            copy_to_clipboard: load_popup.copy.or_else(|| self.copy_to_clipboard.take()),
            reload_profile: settings_output.reload_profile,
            load_conflicts,
            keybind_preset: None,
        })
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
    };
    use crate::{
        session::{DiagnosticSeverity, Session, fixtures::install_diff},
        settings::{KeyBinding, KeyBindings, KeybindAction, Settings},
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
    fn the_object_tree_tab_starts_in_front_of_the_git_tab() {
        let _guard = IMGUI_CONTEXT.lock().unwrap();
        let mut context = rectangle_context();
        let flags = context.io().config_flags() | dear_imgui_rs::ConfigFlags::DOCKING_ENABLE;
        context.io_mut().set_config_flags(flags);
        let mut state = UiState::new(false).unwrap();
        let mut session = Session::new();
        let mut settings = Settings::default();
        let mut frame = |state: &mut UiState| {
            let ui = context.frame();
            state.draw(ui, &mut session, &mut settings, None).unwrap();
            assert!(context.render_legacy().valid());
        };

        for _ in 0..4 {
            frame(&mut state);
        }
        assert!(!state.select_object_tree, "the object tree docked and took the front");
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
            state.draw(ui, session, &mut settings, None).unwrap();
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
        assert_eq!(KeybindAction::ALL.len(), 33);
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
