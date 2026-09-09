use core::path::TreePath;
use std::collections::HashSet;

use dear_imgui_rs::{
    DockLayout,
    DockLayoutApply,
    DockNodeFlags,
    DockSplit,
    DockspaceError,
    Key,
    MouseButton,
    StyleColor,
    StyleVar,
    Ui,
    WindowFlags,
    WindowKey,
    WindowKeyError,
};
use dmm::{Coord, Prefab};
use editor::{
    command::EditGroupId,
    icons::materialdesignicons::{
        ICON_ERASER,
        ICON_EYEDROPPER,
        ICON_FORMAT_COLOR_FILL,
        ICON_IMAGE_BROKEN,
        ICON_MENU_DOWN,
        ICON_PENCIL,
    },
    tool::{FillMode, Tool, is_placeable},
};
use objtree::{ObjectTree, TypeId};
use render::{InteractionMode, PickRequest, PlacementFlash, Renderer, ViewportInteraction};

use crate::{
    camera::Controller,
    gizmo::{GizmoState, GizmoViewport},
    inspector::InspectorState,
    session::{FillOutcome, PlacementPreview, Session},
    settings::{BINDABLE_KEYS, KeyBinding, KeyBindings, KeybindAction, Settings},
};

/// How far down the "Blur below" menu goes. The option itself takes any depth.
const MAX_UNDERLAY_DEPTH: u32 = 3;

const DOCKSPACE_ID: &str = "dmed-main-dockspace";
const OVERLAY_PADDING: f32 = 4.0;
const OVERLAY_BG: [f32; 4] = [0.0, 0.0, 0.0, 0.55];
const RECENT_ICON_SIZE: f32 = 48.0;
const RECENT_BADGE_BG: [f32; 4] = [0.0, 0.0, 0.0, 0.8];
const RECENT_BADGE_TEXT: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const PLACEMENT_FLASH_DURATION: f64 = 0.25;
const PLACEMENT_PREVIEW_PERIOD: f64 = 1.5;
const DEFAULT_CUSTOM_FILL_BOUNDARY: &str = "/turf/closed/wall";
const MAX_CUSTOM_FILL_SEARCH_RESULTS: usize = 50;
const FILL_LIMIT_WARNING_POPUP: &str = "Large fill##fill-limit-warning";

#[derive(Debug, Clone, Copy, PartialEq)]
struct ActivePlacementFlash {
    owner: dmm::PrefabInstanceId,
    coord: Coord,
    started_at: f64,
}

impl ActivePlacementFlash {
    fn sample(self, now: f64) -> Option<PlacementFlash> {
        let elapsed = (now - self.started_at).max(0.0);
        if !elapsed.is_finite() || elapsed >= PLACEMENT_FLASH_DURATION {
            return None;
        }

        Some(PlacementFlash {
            owner: self.owner,
            strength: (1.0 - elapsed / PLACEMENT_FLASH_DURATION) as f32,
        })
    }
}

#[derive(Debug)]
struct PlacementStroke {
    prefab: Prefab,
    z: u32,
    group: EditGroupId,
    visited: HashSet<Coord>,
}

impl PlacementStroke {
    fn new(prefab: Prefab, z: u32) -> Self {
        Self {
            prefab,
            z,
            group: EditGroupId::new(),
            visited: HashSet::new(),
        }
    }

    fn matches_context(&self, tool: Tool, prefab: Option<&Prefab>, z: u32) -> bool {
        tool == Tool::Place && prefab == Some(&self.prefab) && z == self.z
    }

    fn visit(&mut self, coord: Coord) -> Option<EditGroupId> {
        (coord.z == self.z && self.visited.insert(coord)).then_some(self.group)
    }
}

#[derive(Debug)]
struct DeletionStroke {
    cursor: [u32; 2],
}

impl DeletionStroke {
    const fn new(cursor: [u32; 2]) -> Self { Self { cursor } }

    fn move_to(&mut self, cursor: [u32; 2]) -> bool {
        let moved = self.cursor != cursor;
        self.cursor = cursor;

        moved
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingFillWarning {
    coord: Coord,
    fill_mode: FillMode,
    custom_fill_boundaries: Vec<TreePath>,
    limit: usize,
}

fn draw_overlay_underlay(ui: &Ui, bounds: OverlayRect) {
    ui.get_window_draw_list()
        .add_rect(bounds.min, bounds.max, OVERLAY_BG)
        .filled(true)
        .build();
}

pub struct UiOutput {
    pub exit: bool,
    pub viewport: (u32, u32),
    pub interaction: ViewportInteraction,
}

pub struct UiState {
    object_tree: WindowKey,
    viewport_window: WindowKey,
    inspector_window: WindowKey,
    settings_window: WindowKey,
    layout: DockLayout,
    selected: Option<TypeId>,
    inspector: InspectorState,
    gizmo: GizmoState,
    placement_flash: Option<ActivePlacementFlash>,
    placement_stroke: Option<PlacementStroke>,
    deletion_stroke: Option<DeletionStroke>,
    fill_mode: FillMode,
    custom_fill_boundaries: Vec<TreePath>,
    custom_fill_search: String,
    pending_fill_warning: Option<PendingFillWarning>,
    viewport: (u32, u32),
    initial_refit: bool,
    show_settings: bool,
    capturing_keybind: Option<KeybindAction>,
}

impl UiState {
    pub fn new() -> Result<Self, WindowKeyError> {
        let object_tree = WindowKey::new("object-tree", "Object tree")?;
        let viewport_window = WindowKey::new("viewport", "Viewport")?;
        let inspector_window = WindowKey::new("inspector", "Inspector")?;
        let settings_window = WindowKey::new("settings", "Settings")?;
        let layout = DockLayout::split(
            DockSplit::Left,
            0.25,
            DockLayout::tabs([&object_tree]),
            DockLayout::split(
                DockSplit::Right,
                0.20 / 0.75,
                DockLayout::tabs([&inspector_window]),
                DockLayout::tabs([&viewport_window]),
            ),
        );

        Ok(Self {
            object_tree,
            viewport_window,
            inspector_window,
            settings_window,
            layout,
            selected: None,
            inspector: InspectorState::default(),
            gizmo: GizmoState::default(),
            placement_flash: None,
            placement_stroke: None,
            deletion_stroke: None,
            fill_mode: FillMode::default(),
            custom_fill_boundaries: vec![TreePath::parse(DEFAULT_CUSTOM_FILL_BOUNDARY)],
            custom_fill_search: String::new(),
            pending_fill_warning: None,
            viewport: (1, 1),
            initial_refit: true,
            show_settings: false,
            capturing_keybind: None,
        })
    }

    pub fn draw(
        &mut self, ui: &Ui, session: &mut Session, settings: &mut Settings, camera: &mut Controller,
    ) -> Result<UiOutput, DockspaceError> {
        ui.dockspace()
            .main_viewport()
            .root_id(ui.get_id(DOCKSPACE_ID))
            .flags(DockNodeFlags::PASSTHRU_CENTRAL_NODE)
            .layout(&self.layout, DockLayoutApply::IfMissing)
            .build()?;

        let mut exit = false;
        let mut toggle_areas = false;
        let mut toggle_area_outlines = false;
        let mut level_delta = 0;
        let mut underlay_depth = None;
        let mut refit = false;
        let mut interaction = ViewportInteraction {
            selected: session.selected_instance(),
            selection_guide: settings
                .selection_guide_line
                .then(|| session.selected_offset_guide())
                .flatten(),
            mode: interaction_mode(session.tool()),
            ..Default::default()
        };

        ui.main_menu_bar(|| {
            ui.menu("File", || {
                if ui.menu_item("Settings...") {
                    self.show_settings = true;
                }
                ui.separator();
                if ui.menu_item_with_shortcut("Exit", "Esc") {
                    exit = true;
                }
            });
            ui.menu("View", || {
                if ui.menu_item_enabled_selected_with_shortcut(
                    "Show areas",
                    settings.keybindings.get(KeybindAction::ShowAreas).label(ui),
                    session.options.show_areas,
                    true,
                ) {
                    toggle_areas = true;
                }
                if ui.menu_item_enabled_selected_with_shortcut(
                    "Show area outlines",
                    settings.keybindings.get(KeybindAction::ShowAreaOutlines).label(ui),
                    session.options.show_area_outlines,
                    true,
                ) {
                    toggle_area_outlines = true;
                }
                if ui.menu_item_with_shortcut("Z up", settings.keybindings.get(KeybindAction::LevelUp).label(ui)) {
                    level_delta += 1;
                }
                if ui.menu_item_with_shortcut("Z down", settings.keybindings.get(KeybindAction::LevelDown).label(ui)) {
                    level_delta -= 1;
                }
                ui.menu("Blur below", || {
                    for depth in 0..=MAX_UNDERLAY_DEPTH {
                        let label = match depth {
                            0 => String::from("Off"),
                            depth => format!("{depth} level(s)"),
                        };

                        if ui.menu_item_enabled_selected(
                            label,
                            None::<&str>,
                            session.options.underlay_depth == depth,
                            true,
                        ) {
                            underlay_depth = Some(depth);
                        }
                    }
                });
                if ui.menu_item_with_shortcut("Refit", settings.keybindings.get(KeybindAction::Refit).label(ui)) {
                    refit = true;
                }
            });
        });

        if toggle_areas {
            session.toggle_areas();
        }
        if toggle_area_outlines {
            session.toggle_area_outlines();
        }
        if level_delta != 0 {
            session.change_level(level_delta);
        }
        if let Some(depth) = underlay_depth {
            session.set_underlay_depth(depth);
        }

        draw_settings_window(
            ui,
            &self.settings_window,
            &mut self.show_settings,
            &mut self.capturing_keybind,
            session,
            settings,
        );
        self.draw_object_tree(ui, session);
        self.draw_inspector(ui, session);
        exit |= self.draw_viewport(ui, session, settings, camera, &mut interaction, &mut refit);
        finish_keybind_capture(ui, &mut self.capturing_keybind, &mut settings.keybindings);
        interaction.selection_guide = settings
            .selection_guide_line
            .then(|| interaction.selected.and_then(|_| session.selected_offset_guide()))
            .flatten();

        Ok(UiOutput {
            exit,
            viewport: self.viewport,
            interaction,
        })
    }

    fn draw_object_tree(&mut self, ui: &Ui, session: &mut Session) {
        ui.window(&self.object_tree).build(|| {
            let mut chosen = None;
            let Some(tree) = session.tree() else {
                ui.text_disabled("No environment loaded");

                return;
            };

            for child in sorted_children(tree, TypeId::ROOT) {
                draw_type(ui, tree, child, &mut self.selected, &mut chosen);
            }

            ui.separator();
            let Some(selected) = self.selected.and_then(|id| tree.get(id)) else {
                ui.text_disabled("No type selected");

                return;
            };

            ui.text(selected.path.to_string());
            ui.text(format!("{} variables", selected.vars.len()));
            ui.text(format!("{} procedures", selected.procs.len()));
            if !is_placeable(tree, selected.id) {
                ui.text_disabled("This type cannot be placed on a map");
            }

            if let Some(chosen) = chosen {
                session.choose_type(chosen);
            }
        });
    }

    fn draw_viewport(
        &mut self, ui: &Ui, session: &mut Session, settings: &Settings, camera: &mut Controller,
        interaction: &mut ViewportInteraction, refit: &mut bool,
    ) -> bool {
        let mut exit = false;
        ui.window(&self.viewport_window).build(|| {
            let (image_size, viewport) = panel_extent(ui.content_region_avail());
            self.viewport = viewport;
            camera.resize(viewport.0, viewport.1);
            if self.initial_refit || *refit {
                let (width, height) = session.extent_px();
                camera.frame_map(width, height);
                self.initial_refit = false;
                *refit = false;
            }
            ui.image(Renderer::VIEWPORT_TEXTURE, image_size);

            let image_hovered = ui.is_item_hovered();
            let viewport_min = ui.item_rect_min();
            let viewport_max = ui.item_rect_max();
            let top_overlay_height = ui.frame_height() + OVERLAY_PADDING * 2.0;
            let history_overlay_height = recent_button_size(ui) + OVERLAY_PADDING * 2.0;
            let top_overlay = OverlayRect {
                min: viewport_min,
                max: [
                    viewport_max[0],
                    (viewport_min[1] + top_overlay_height).min(viewport_max[1]),
                ],
            };
            let bottom_overlay = OverlayRect {
                min: [
                    viewport_min[0],
                    (viewport_max[1] - history_overlay_height).max(viewport_min[1]),
                ],
                max: viewport_max,
            };

            let mouse = ui.io().mouse_pos();
            let over_overlay = top_overlay.contains(mouse) || bottom_overlay.contains(mouse);
            let hovered = image_hovered && !over_overlay;
            let focused = ui.is_window_focused();
            if focused
                && !ui.io().want_text_input()
                && !has_modifiers(ui)
                && (!hovered || !settings.keybindings.is_any_pressed(ui))
                && let Some(index) = pressed_recent(ui)
            {
                session.choose_recent(index);
            }

            if hovered {
                let io = ui.io();
                if !self.gizmo.is_interacting() && ui.is_mouse_down(MouseButton::Middle) {
                    camera.pan_by(io.mouse_delta());
                }

                let wheel = io.mouse_wheel();
                if !self.gizmo.is_interacting() && wheel != 0.0 {
                    let mouse = io.mouse_pos();
                    camera.zoom_by(wheel, [mouse[0] - viewport_min[0], mouse[1] - viewport_min[1]]);
                }

                if settings.keybindings.get(KeybindAction::ShowAreas).is_pressed(ui) {
                    session.toggle_areas();
                }
                if settings.keybindings.get(KeybindAction::ShowAreaOutlines).is_pressed(ui) {
                    session.toggle_area_outlines();
                }
                if settings.keybindings.get(KeybindAction::LevelUp).is_pressed(ui) {
                    session.change_level(1);
                }
                if settings.keybindings.get(KeybindAction::LevelDown).is_pressed(ui) {
                    session.change_level(-1);
                }
                if settings.keybindings.get(KeybindAction::Refit).is_pressed(ui) {
                    *refit = true;
                }
                if settings.keybindings.get(KeybindAction::PlaceTool).is_pressed(ui) {
                    session.set_tool(Tool::Place);
                }
                if settings.keybindings.get(KeybindAction::SelectTool).is_pressed(ui) {
                    session.set_tool(Tool::Select);
                }
                if settings.keybindings.get(KeybindAction::DeleteTool).is_pressed(ui) {
                    session.set_tool(Tool::Delete);
                }
                if settings.keybindings.get(KeybindAction::FillTool).is_pressed(ui) {
                    session.set_tool(Tool::Fill);
                }

                if *refit {
                    let (width, height) = session.extent_px();
                    camera.frame_map(width, height);
                    *refit = false;
                }
            }
            configure_tool_interaction(session.tool(), interaction);

            let in_viewport = |point: [f32; 2]| {
                let local = [point[0] - viewport_min[0], point[1] - viewport_min[1]];

                local
                    .iter()
                    .all(|value| value.is_finite() && *value >= 0.0)
                    .then_some(local)
                    .filter(|local| local[0] < viewport.0 as f32 && local[1] < viewport.1 as f32)
            };
            let cursor = hovered.then(|| ui.io().mouse_pos()).and_then(in_viewport);
            let pointed_coord = cursor.and_then(|cursor| {
                let size = session.map()?.size;

                camera.screen_to_tile(cursor, size, session.options.tile_size, session.z())
            });
            if hovered
                && !ui.io().want_text_input()
                && !has_modifiers(ui)
                && !settings.keybindings.is_any_pressed(ui)
                && ui.is_key_pressed(Key::F)
            {
                session.toggle_focus_at(pointed_coord);
                self.placement_stroke = None;
            }
            let tool = session.tool();
            let preview_coord = (tool == Tool::Place)
                .then(|| self.gizmo.placement_coord().or(pointed_coord))
                .flatten();
            let active_flash = active_placement_flash(&mut self.placement_flash, ui.time(), settings.tile_place_flash);
            interaction.placement_flash = active_flash.map(|(_, flash)| flash);
            if let Some(coord) = preview_coord
                && session.can_edit_at(coord)
                && active_flash.is_none_or(|(flash_coord, _)| flash_coord != coord)
            {
                draw_placement_preview(ui, session, camera, coord, viewport_min, viewport_max);
            }

            let gizmo_viewport = GizmoViewport {
                min: viewport_min,
                max: viewport_max,
                hovered,
            };
            let gizmo_captures_mouse = match tool {
                Tool::Select => {
                    self.gizmo
                        .draw(ui, session, camera, self.inspector.transform_mode(), gizmo_viewport)
                        .captures_mouse
                },
                Tool::Place => {
                    self.gizmo
                        .draw_placement_direction(ui, session, camera, pointed_coord, gizmo_viewport)
                        .captures_mouse
                },
                Tool::Delete => {
                    self.gizmo.cancel();

                    false
                },
                Tool::Fill => {
                    self.gizmo.cancel();

                    false
                },
            };

            let left_clicked = ui.is_mouse_clicked(MouseButton::Left);
            let left_down = ui.is_mouse_down(MouseButton::Left);
            if left_clicked {
                self.placement_stroke = None;
                self.deletion_stroke = None;
            }
            if !left_down || session.tool() != Tool::Delete {
                self.deletion_stroke = None;
            }

            if self.placement_stroke.as_ref().is_some_and(|stroke| {
                !left_down
                    || gizmo_captures_mouse
                    || !stroke.matches_context(session.tool(), session.palette(), session.z())
            }) {
                self.placement_stroke = None;
            }
            if !gizmo_captures_mouse && let Some(cursor) = cursor {
                let pixel = [cursor[0].floor() as u32, cursor[1].floor() as u32];
                interaction.cursor = Some(pixel);

                if let Some(coord) = pointed_coord {
                    interaction.hovered_area = session.area_at(coord);
                    match session.tool() {
                        Tool::Place => {
                            if left_clicked {
                                self.placement_stroke = session
                                    .palette()
                                    .cloned()
                                    .map(|prefab| PlacementStroke::new(prefab, session.z()));
                            }
                            let group = session
                                .can_edit_at(coord)
                                .then(|| self.placement_stroke.as_mut().and_then(|stroke| stroke.visit(coord)))
                                .flatten();
                            let placed = group.and_then(|group| session.place_at(coord, Some(group)));
                            if settings.tile_place_flash
                                && let Some(owner) = placed
                            {
                                let flash = ActivePlacementFlash {
                                    owner,
                                    coord,
                                    started_at: ui.time(),
                                };
                                self.placement_flash = Some(flash);
                                interaction.placement_flash = flash.sample(ui.time());
                            }
                        },
                        Tool::Select => {
                            self.placement_stroke = None;
                            if left_clicked {
                                request_pick(interaction, PickRequest::Cursor);
                            }
                        },
                        Tool::Delete => {
                            self.placement_stroke = None;
                            let requests_pick = if left_clicked {
                                self.deletion_stroke = Some(DeletionStroke::new(pixel));

                                true
                            } else {
                                left_down
                                    && self
                                        .deletion_stroke
                                        .as_mut()
                                        .is_some_and(|stroke| stroke.move_to(pixel))
                            };
                            if requests_pick {
                                request_pick(interaction, PickRequest::Cursor);
                            }
                        },
                        Tool::Fill => {
                            self.placement_stroke = None;
                            if left_clicked
                                && let FillOutcome::TooLarge { limit } =
                                    session.fill_at(coord, self.fill_mode, &self.custom_fill_boundaries)
                            {
                                self.pending_fill_warning = Some(PendingFillWarning {
                                    coord,
                                    fill_mode: self.fill_mode,
                                    custom_fill_boundaries: self.custom_fill_boundaries.clone(),
                                    limit,
                                });
                                ui.open_popup(FILL_LIMIT_WARNING_POPUP);
                            }
                        },
                    }
                }
            }

            draw_top_overlay(
                ui,
                session,
                top_overlay,
                &mut self.fill_mode,
                &mut self.custom_fill_boundaries,
                &mut self.custom_fill_search,
            );
            let recent_prefabs = session.recent_prefabs();
            if !recent_prefabs.is_empty() {
                draw_history_overlay(ui, session, bottom_overlay, recent_prefabs.to_vec());
            }
            configure_tool_interaction(session.tool(), interaction);
            if session.focused_area().is_some() {
                interaction.hovered_area = None;
            }

            let fill_warning_handles_escape = draw_fill_limit_warning(ui, session, &mut self.pending_fill_warning);
            if ui.is_key_pressed(Key::Escape) && !fill_warning_handles_escape && self.capturing_keybind.is_none() {
                exit = true;
            }
        });

        exit
    }

    fn draw_inspector(&mut self, ui: &Ui, session: &mut Session) {
        ui.window(&self.inspector_window).build(|| {
            self.inspector.draw(ui, session);
        });
    }
}

fn draw_settings_window(
    ui: &Ui, window: &WindowKey, open: &mut bool, capturing: &mut Option<KeybindAction>, session: &mut Session,
    settings: &mut Settings,
) {
    if !*open {
        *capturing = None;

        return;
    }

    let flags = WindowFlags::ALWAYS_AUTO_RESIZE | WindowFlags::NO_COLLAPSE | WindowFlags::NO_DOCKING;
    ui.window(window).opened(open).flags(flags).build(|| {
        ui.checkbox("Show areas", &mut session.options.show_areas);
        ui.checkbox("Show area outlines", &mut session.options.show_area_outlines);
        ui.checkbox("Tile placement flash", &mut settings.tile_place_flash);
        ui.checkbox("Selection guide line", &mut settings.selection_guide_line);

        ui.separator();
        ui.text("Keybindings");
        for action in KeybindAction::ALL {
            ui.text(action.label());
            ui.same_line_with_pos(180.0);

            let binding = settings.keybindings.get(action).label(ui);
            let visible = if *capturing == Some(action) {
                "Press a key..."
            } else {
                binding.as_str()
            };
            if ui.button_with_size(format!("{visible}##keybind-{}", action.id()), [140.0, 0.0]) {
                *capturing = (*capturing != Some(action)).then_some(action);
            }
        }

        if ui.button("Reset keybindings") {
            settings.keybindings = KeyBindings::default();
            *capturing = None;
        }
    });

    if !*open {
        *capturing = None;
    }
}

fn finish_keybind_capture(ui: &Ui, capturing: &mut Option<KeybindAction>, keybindings: &mut KeyBindings) {
    let Some(action) = *capturing else {
        return;
    };
    if ui.is_key_pressed_with_repeat(Key::Escape, false) {
        *capturing = None;

        return;
    }
    let Some(key) = BINDABLE_KEYS
        .iter()
        .copied()
        .find(|key| ui.is_key_pressed_with_repeat(*key, false))
    else {
        return;
    };

    keybindings.rebind(action, KeyBinding::from_input(ui, key));
    *capturing = None;
}

fn active_placement_flash(
    active: &mut Option<ActivePlacementFlash>, now: f64, enabled: bool,
) -> Option<(Coord, PlacementFlash)> {
    if !enabled {
        *active = None;

        return None;
    }

    let placement = (*active)?;
    let Some(flash) = placement.sample(now) else {
        *active = None;

        return None;
    };

    Some((placement.coord, flash))
}

fn draw_z_levels(ui: &Ui, session: &mut Session) {
    let current = session.z();
    let levels = session.level_count();
    ui.align_text_to_frame_padding();
    ui.text("Z");

    let mut selected = None;
    for z in 1..=levels {
        ui.same_line();

        if ui.radio_button(z.to_string(), current == z) {
            selected = Some(z);
        }
    }
    if let Some(z) = selected {
        session.set_level(z);
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct OverlayRect {
    min: [f32; 2],
    max: [f32; 2],
}

impl OverlayRect {
    fn contains(self, point: [f32; 2]) -> bool {
        point[0] >= self.min[0] && point[0] < self.max[0] && point[1] >= self.min[1] && point[1] < self.max[1]
    }
}

fn draw_placement_preview(
    ui: &Ui, session: &Session, camera: &Controller, coord: Coord, viewport_min: [f32; 2], viewport_max: [f32; 2],
) {
    let Some(preview) = session.placement_preview() else {
        return;
    };
    let bounds = placement_preview_bounds(camera, viewport_min, coord, session.options.tile_size, preview);
    let mut tint = preview.thumbnail.tint;
    tint[3] *= placement_preview_opacity(ui.time());
    let draw = ui.get_window_draw_list();

    draw.with_clip_rect(viewport_min, viewport_max, || {
        draw.add_image(
            Renderer::sprite_texture(preview.thumbnail.texture.index),
            bounds.min,
            bounds.max,
            preview.thumbnail.uv0,
            preview.thumbnail.uv1,
            tint,
        );
    });
}

fn placement_preview_bounds(
    camera: &Controller, viewport_min: [f32; 2], coord: Coord, tile_size: u32, preview: PlacementPreview,
) -> OverlayRect {
    let tile_size = tile_size.max(1);
    let x = coord.x.saturating_sub(1).saturating_mul(tile_size) as f32 + preview.offset[0] as f32;
    let y = coord.y.saturating_sub(1).saturating_mul(tile_size) as f32 + preview.offset[1] as f32;
    let width = preview.thumbnail.texture.width as f32;
    let height = preview.thumbnail.texture.height as f32;
    let top_left = camera.map_to_screen([x, y + height]);
    let bottom_right = camera.map_to_screen([x + width, y]);

    OverlayRect {
        min: [viewport_min[0] + top_left[0], viewport_min[1] + top_left[1]],
        max: [viewport_min[0] + bottom_right[0], viewport_min[1] + bottom_right[1]],
    }
}

fn placement_preview_opacity(time: f64) -> f32 {
    let phase = time.rem_euclid(PLACEMENT_PREVIEW_PERIOD) / PLACEMENT_PREVIEW_PERIOD * std::f64::consts::TAU;

    (0.9 + 0.1 * phase.cos()) as f32
}

fn configure_tool_interaction(tool: Tool, interaction: &mut ViewportInteraction) {
    interaction.mode = match (tool, interaction.mode) {
        (Tool::Place | Tool::Fill, _) => InteractionMode::Place,
        (Tool::Select, InteractionMode::Select { pick }) => InteractionMode::Select { pick },
        (Tool::Delete, InteractionMode::Delete { pick }) => InteractionMode::Delete { pick },
        (Tool::Select, _) => InteractionMode::Select { pick: None },
        (Tool::Delete, _) => InteractionMode::Delete { pick: None },
    };
    match tool {
        Tool::Place | Tool::Fill => {
            interaction.cursor = None;
            interaction.hovered_area = None;
            interaction.selected = None;
            interaction.selection_guide = None;
        },
        Tool::Delete => {
            interaction.selected = None;
            interaction.selection_guide = None;
        },
        Tool::Select => {},
    }
}

fn interaction_mode(tool: Tool) -> InteractionMode {
    match tool {
        Tool::Place | Tool::Fill => InteractionMode::Place,
        Tool::Select => InteractionMode::Select { pick: None },
        Tool::Delete => InteractionMode::Delete { pick: None },
    }
}

fn request_pick(interaction: &mut ViewportInteraction, request: PickRequest) {
    match &mut interaction.mode {
        InteractionMode::Place => {},
        InteractionMode::Select { pick } | InteractionMode::Delete { pick } => *pick = Some(request),
    }
}

fn draw_top_overlay(
    ui: &Ui, session: &mut Session, bounds: OverlayRect, fill_mode: &mut FillMode,
    custom_fill_boundaries: &mut Vec<TreePath>, custom_fill_search: &mut String,
) {
    draw_overlay_underlay(ui, bounds);

    ui.set_cursor_screen_pos([bounds.min[0] + OVERLAY_PADDING, bounds.min[1] + OVERLAY_PADDING]);

    draw_tool_button(ui, session, Tool::Place, ICON_PENCIL);
    ui.same_line();
    draw_tool_button(ui, session, Tool::Select, ICON_EYEDROPPER);
    ui.same_line();
    draw_tool_button(ui, session, Tool::Delete, ICON_ERASER);
    ui.same_line();
    let tools_end = draw_fill_tool_button(ui, session, fill_mode, custom_fill_boundaries, custom_fill_search);

    let levels_width = z_level_width(ui, session.level_count());
    let levels_x = (bounds.max[0] - OVERLAY_PADDING - levels_width).max(tools_end + OVERLAY_PADDING);
    ui.set_cursor_screen_pos([levels_x, bounds.min[1] + OVERLAY_PADDING]);
    draw_z_levels(ui, session);
}

fn draw_fill_limit_warning(ui: &Ui, session: &mut Session, pending: &mut Option<PendingFillWarning>) -> bool {
    let was_pending = pending.is_some();
    let mut fill_anyway = false;
    let mut dismiss = false;

    let flags = WindowFlags::ALWAYS_AUTO_RESIZE
        | WindowFlags::NO_RESIZE
        | WindowFlags::NO_MOVE
        | WindowFlags::NO_COLLAPSE
        | WindowFlags::NO_SAVED_SETTINGS
        | WindowFlags::NO_DOCKING;
    if let Some(warning) = pending.as_ref()
        && let Some(_modal) = ui
            .begin_modal_popup_config(FILL_LIMIT_WARNING_POPUP)
            .flags(flags)
            .begin()
    {
        ui.text(format!("This fill would change more than {} tiles.", warning.limit));
        ui.text("The operation may make the editor unresponsive.");
        ui.text("Do you want to fill it anyway?");
        ui.separator();

        if ui.button("Cancel") || ui.is_key_pressed(Key::Escape) {
            dismiss = true;
            ui.close_current_popup();
        }
        ui.same_line();
        if ui.button("Fill Anyway") {
            fill_anyway = true;
            dismiss = true;
            ui.close_current_popup();
        }
    }

    if dismiss
        && let Some(warning) = pending.take()
        && fill_anyway
    {
        session.fill_at_unlimited(warning.coord, warning.fill_mode, &warning.custom_fill_boundaries);
    }

    was_pending
}

fn draw_fill_tool_button(
    ui: &Ui, session: &mut Session, fill_mode: &mut FillMode, custom_fill_boundaries: &mut Vec<TreePath>,
    custom_fill_search: &mut String,
) -> f32 {
    const POPUP: &str = "fill-mode-popup";

    let active_color = ui.style_color(StyleColor::PlotHistogramHovered);
    let active = (session.tool() == Tool::Fill).then(|| ui.push_style_color(StyleColor::Button, active_color));
    let spacing = ui.clone_style().item_spacing();
    let connected = ui.push_style_var(StyleVar::ItemSpacing([0.0, spacing[1]]));

    if ui.button(format!("{ICON_FORMAT_COLOR_FILL}##fill-tool")) {
        session.set_tool(Tool::Fill);
    }
    let separator_x = ui.item_rect_max()[0];
    let separator_min_y = ui.item_rect_min()[1];
    let separator_max_y = ui.item_rect_max()[1];
    ui.set_item_tooltip(format!("Fill ({})", fill_mode.label()));
    ui.same_line();

    let button_size = ui.frame_height();
    let arrow_width = (button_size * 0.65)
        .ceil()
        .max((ui.calc_text_size(ICON_MENU_DOWN.to_string())[0] + 2.0).ceil());
    let frame_padding = ui.clone_style().frame_padding();
    let arrow_clicked = {
        let _padding = ui.push_style_var(StyleVar::FramePadding([0.0, frame_padding[1]]));
        let _alignment = ui.push_style_var(StyleVar::ButtonTextAlign([0.5, 0.5]));
        ui.button_with_size(format!("{ICON_MENU_DOWN}##fill-mode"), [arrow_width, button_size])
    };
    if arrow_clicked {
        ui.open_popup(POPUP);
    }
    let tools_end = ui.item_rect_max()[0];
    ui.set_item_tooltip(format!("Fill mode: {}", fill_mode.label()));
    ui.get_window_draw_list().add_line_v(
        separator_x,
        separator_min_y + frame_padding[1],
        separator_max_y - frame_padding[1],
        [0.0, 0.0, 0.0, 0.7],
        1.0,
    );

    drop(connected);
    drop(active);

    if let Some(_popup) = ui.begin_popup(POPUP) {
        for mode in [FillMode::Wall, FillMode::EntireArea, FillMode::Custom] {
            if ui.menu_item_enabled_selected_no_shortcut(mode.label(), *fill_mode == mode, true) {
                *fill_mode = mode;
                ui.close_current_popup();
            }
        }

        ui.separator();
        let selected = session.palette().map(|prefab| prefab.path.clone());
        let can_add = selected
            .as_ref()
            .is_some_and(|path| !custom_fill_boundaries.contains(path));
        let add_label = selected
            .as_ref()
            .map_or_else(|| "Add selected type".to_owned(), |path| format!("Add {path}"));
        if ui.menu_item_enabled_selected_no_shortcut(add_label, false, can_add)
            && let Some(path) = selected
        {
            custom_fill_boundaries.push(path);
            *fill_mode = FillMode::Custom;
        }

        let mut searched_path = None;
        if let Some(_menu) = ui.begin_menu("Search type paths") {
            ui.set_next_item_width(320.0);
            ui.input_text("##custom-fill-boundary-search", custom_fill_search)
                .build();

            if custom_fill_search.trim().is_empty() {
                ui.text_disabled("Type a path to search");
            } else if let Some(tree) = session.tree() {
                let matches = matching_type_paths(tree, custom_fill_search);
                if matches.is_empty() {
                    ui.text_disabled("No matching types");
                } else {
                    for path in matches {
                        let enabled = !custom_fill_boundaries.contains(&path);
                        if ui.menu_item_enabled_selected_no_shortcut(path.to_string(), false, enabled) {
                            searched_path = Some(path);
                        }
                    }
                }
            } else {
                ui.text_disabled("No environment loaded");
            }
        }
        if let Some(path) = searched_path {
            custom_fill_boundaries.push(path);
            *fill_mode = FillMode::Custom;
        }

        let mut remove = None;
        if let Some(_menu) = ui.begin_menu_with_enabled(
            format!("Remove boundary ({})", custom_fill_boundaries.len()),
            !custom_fill_boundaries.is_empty(),
        ) {
            for (index, path) in custom_fill_boundaries.iter().enumerate() {
                if ui.menu_item(format!("{path}##custom-fill-boundary-{index}")) {
                    remove = Some(index);
                }
            }
        }
        if let Some(index) = remove {
            custom_fill_boundaries.remove(index);
        }

        let default = TreePath::parse(DEFAULT_CUSTOM_FILL_BOUNDARY);
        let is_default = custom_fill_boundaries.as_slice() == [default.clone()];
        if ui.menu_item_enabled_selected_no_shortcut("Reset custom boundaries", false, !is_default) {
            custom_fill_boundaries.clear();
            custom_fill_boundaries.push(default);
        }
    }

    tools_end
}

fn matching_type_paths(tree: &ObjectTree, query: &str) -> Vec<TreePath> {
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return Vec::new();
    }

    let mut matches = tree
        .iter()
        .filter(|decl| decl.path.to_string().to_ascii_lowercase().contains(&query))
        .map(|decl| decl.path.clone())
        .take(MAX_CUSTOM_FILL_SEARCH_RESULTS)
        .collect::<Vec<_>>();
    matches.sort_by_key(ToString::to_string);

    matches
}

fn draw_tool_button(ui: &Ui, session: &mut Session, tool: Tool, icon: char) {
    let color = match tool {
        Tool::Delete => [1.0, 0.0, 0.0, 1.0],
        _ => ui.style_color(StyleColor::PlotHistogramHovered),
    };

    let _color = (session.tool() == tool).then(|| ui.push_style_color(StyleColor::Button, color));
    if ui.button(icon.to_string()) {
        session.set_tool(tool);
    }
}

fn draw_history_overlay(ui: &Ui, session: &mut Session, bounds: OverlayRect, recent_prefabs: Vec<Prefab>) {
    draw_overlay_underlay(ui, bounds);

    let button_size = recent_button_size(ui);
    let palette = session.palette().cloned();
    let mut chosen = None;
    ui.set_cursor_screen_pos([bounds.min[0] + OVERLAY_PADDING, bounds.min[1] + OVERLAY_PADDING]);

    for (index, prefab) in recent_prefabs.iter().enumerate() {
        if index > 0 {
            ui.same_line();
        }
        let key = recent_key_label(index);
        let _selected = (palette.as_ref() == Some(prefab))
            .then(|| ui.push_style_color(StyleColor::Button, ui.style_color(StyleColor::ButtonActive)));
        let clicked = match session.prefab_thumbnail(prefab) {
            Some(thumbnail) => {
                let image_size = fit_recent_icon(thumbnail.texture.width, thumbnail.texture.height);
                let padding = [(button_size - image_size[0]) * 0.5, (button_size - image_size[1]) * 0.5];
                let _padding = ui.push_style_var(StyleVar::FramePadding(padding));

                ui.image_button_config(
                    format!("recent-{index}"),
                    Renderer::sprite_texture(thumbnail.texture.index),
                    image_size,
                )
                .uv0(thumbnail.uv0)
                .uv1(thumbnail.uv1)
                .tint_color(thumbnail.tint)
                .build()
            },
            None => ui.button_with_size(format!("{ICON_IMAGE_BROKEN}##recent-{index}"), [button_size; 2]),
        };
        draw_recent_badge(ui, key);
        if clicked {
            chosen = Some(index);
        }
        ui.set_item_tooltip(prefab_tooltip(prefab));
    }

    if let Some(index) = chosen {
        session.choose_recent(index);
    }
}

fn has_modifiers(ui: &Ui) -> bool {
    let io = ui.io();

    io.key_ctrl() || io.key_shift() || io.key_alt() || io.key_super()
}

fn pressed_recent(ui: &Ui) -> Option<usize> {
    RECENT_KEYS
        .iter()
        .position(|(number, keypad)| ui.is_key_pressed(*number) || ui.is_key_pressed(*keypad))
}

const RECENT_KEYS: [(Key, Key); 10] = [
    (Key::Key1, Key::Keypad1),
    (Key::Key2, Key::Keypad2),
    (Key::Key3, Key::Keypad3),
    (Key::Key4, Key::Keypad4),
    (Key::Key5, Key::Keypad5),
    (Key::Key6, Key::Keypad6),
    (Key::Key7, Key::Keypad7),
    (Key::Key8, Key::Keypad8),
    (Key::Key9, Key::Keypad9),
    (Key::Key0, Key::Keypad0),
];

fn recent_key_label(index: usize) -> char {
    const LABELS: [char; 10] = ['1', '2', '3', '4', '5', '6', '7', '8', '9', '0'];

    LABELS.get(index).copied().unwrap_or('?')
}

fn recent_button_size(ui: &Ui) -> f32 {
    let padding = ui.clone_style().frame_padding();

    RECENT_ICON_SIZE + padding[0].max(padding[1]) * 2.0
}

fn fit_recent_icon(width: u32, height: u32) -> [f32; 2] {
    let width = width.max(1) as f32;
    let height = height.max(1) as f32;
    let scale = RECENT_ICON_SIZE / width.max(height);

    [width * scale, height * scale]
}

fn draw_recent_badge(ui: &Ui, key: char) {
    let key = key.to_string();
    let item_min = ui.item_rect_min();
    let text_size = ui.calc_text_size(&key);
    let badge_min = [item_min[0] + 2.0, item_min[1] + 2.0];
    let badge_max = [badge_min[0] + text_size[0] + 4.0, badge_min[1] + text_size[1] + 2.0];
    let draw = ui.get_window_draw_list();
    draw.add_rect(badge_min, badge_max, RECENT_BADGE_BG)
        .rounding(2.0)
        .filled(true)
        .build();
    draw.add_text([badge_min[0] + 2.0, badge_min[1] + 1.0], RECENT_BADGE_TEXT, key);
}

fn prefab_tooltip(prefab: &Prefab) -> String {
    let mut tooltip = prefab.path.to_string();
    for (name, value) in &prefab.vars {
        tooltip.push_str(&format!("\n{name} = {}", dmm::writer::format_value(&value.value)));
    }

    tooltip
}

fn z_level_width(ui: &Ui, levels: u32) -> f32 {
    let style = ui.clone_style();
    let item_spacing = style.item_spacing()[0];
    let inner_spacing = style.item_inner_spacing()[0];
    let radio = ui.frame_height();
    let mut width = ui.calc_text_size("Z")[0];

    for z in 1..=levels {
        width += item_spacing + radio + inner_spacing + ui.calc_text_size(z.to_string())[0];
    }

    width
}

fn draw_type(ui: &Ui, tree: &ObjectTree, id: TypeId, selected: &mut Option<TypeId>, chosen: &mut Option<TypeId>) {
    let Some(decl) = tree.get(id) else {
        return;
    };

    let label = decl
        .path
        .segments
        .last()
        .map_or_else(|| decl.path.to_string(), ToString::to_string);
    let node_id = decl.path.to_string();
    let leaf = decl.children.is_empty();
    let token = ui
        .tree_node_config(node_id)
        .label(label)
        .selected(*selected == Some(id))
        .leaf(leaf)
        .no_tree_push_on_open(leaf)
        .span_avail_width(true)
        .push();

    if ui.is_item_clicked() {
        *selected = Some(id);
        *chosen = Some(id);
    }

    if !leaf && let Some(token) = token {
        for child in sorted_children(tree, id) {
            draw_type(ui, tree, child, selected, chosen);
        }

        token.pop();
    }
}

fn sorted_children(tree: &ObjectTree, parent: TypeId) -> Vec<TypeId> {
    let mut children = tree
        .get(parent)
        .into_iter()
        .flat_map(|decl| decl.children.iter().copied())
        .filter(|id| tree.get(*id).is_some())
        .collect::<Vec<_>>();

    children.sort_by(|left, right| {
        let left = tree.get(*left).map(|decl| decl.path.to_string()).unwrap_or_default();
        let right = tree.get(*right).map(|decl| decl.path.to_string()).unwrap_or_default();

        left.cmp(&right)
    });

    children
}

fn panel_extent(available: [f32; 2]) -> ([f32; 2], (u32, u32)) {
    let width = if available[0].is_finite() {
        available[0].max(1.0)
    } else {
        1.0
    };
    let height = if available[1].is_finite() {
        available[1].max(1.0)
    } else {
        1.0
    };

    ([width, height], (width.floor() as u32, height.floor() as u32))
}

#[cfg(test)]
mod tests {
    use core::{location::Location, path::TreePath};

    use dmm::PrefabInstanceId;
    use render::SpriteTexture;

    use super::*;

    #[test]
    fn the_default_dock_layout_is_valid() {
        let state = UiState::new().expect("valid window keys");

        assert_eq!(state.layout.validate(), Ok(()));
        assert_eq!(state.fill_mode, FillMode::Wall);
        assert_eq!(
            state.custom_fill_boundaries,
            [TreePath::parse(DEFAULT_CUSTOM_FILL_BOUNDARY)]
        );
        assert!(state.custom_fill_search.is_empty());
        assert!(state.pending_fill_warning.is_none());
    }

    #[test]
    fn custom_fill_search_includes_parent_types_and_ignores_case() {
        let mut tree = ObjectTree::new();
        tree.register(&TreePath::parse("/obj/structure/table"), Location::default());
        tree.register(&TreePath::parse("/turf/closed/wall"), Location::default());

        assert_eq!(
            matching_type_paths(&tree, "STRUCT")
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ["/obj/structure", "/obj/structure/table"]
        );
        assert_eq!(
            matching_type_paths(&tree, "wall")
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ["/turf/closed/wall"]
        );
        assert!(matching_type_paths(&tree, "  ").is_empty());
    }

    #[test]
    fn object_tree_children_are_sorted_by_path() {
        let mut tree = ObjectTree::new();
        tree.register(&TreePath::parse("/obj/zeta"), Location::default());
        tree.register(&TreePath::parse("/obj/alpha"), Location::default());
        let obj = tree.id_of(&TreePath::parse("/obj")).expect("registered parent");
        let paths = sorted_children(&tree, obj)
            .into_iter()
            .filter_map(|id| tree.get(id))
            .map(|decl| decl.path.to_string())
            .collect::<Vec<_>>();

        assert_eq!(paths, ["/obj/alpha", "/obj/zeta"]);
    }

    #[test]
    fn panel_extent_rejects_non_finite_or_empty_sizes() {
        assert_eq!(panel_extent([0.0, f32::NAN]), ([1.0, 1.0], (1, 1)));
        assert_eq!(panel_extent([320.75, 200.25]), ([320.75, 200.25], (320, 200)));
    }

    #[test]
    fn recent_slots_follow_the_number_row_order() {
        assert_eq!((0..10).map(recent_key_label).collect::<String>(), "1234567890");
    }

    #[test]
    fn recent_icons_fit_inside_a_square_without_changing_aspect_ratio() {
        assert_eq!(fit_recent_icon(32, 32), [RECENT_ICON_SIZE, RECENT_ICON_SIZE]);
        assert_eq!(fit_recent_icon(64, 32), [RECENT_ICON_SIZE, RECENT_ICON_SIZE / 2.0]);
        assert_eq!(fit_recent_icon(16, 32), [RECENT_ICON_SIZE / 2.0, RECENT_ICON_SIZE]);
        assert_eq!(fit_recent_icon(0, 0), [RECENT_ICON_SIZE, RECENT_ICON_SIZE]);
    }

    #[test]
    fn placement_preview_breathes_between_full_and_eighty_percent_opacity() {
        assert!((placement_preview_opacity(0.0) - 1.0).abs() < f32::EPSILON);
        assert!((placement_preview_opacity(0.75) - 0.8).abs() < f32::EPSILON);
        assert!((placement_preview_opacity(1.5) - 1.0).abs() < f32::EPSILON);
        assert!((placement_preview_opacity(3.75) - 0.8).abs() < f32::EPSILON);
    }

    #[test]
    fn placement_flash_fades_once_over_a_quarter_second() {
        let owner = PrefabInstanceId::from_raw(7).unwrap();
        let flash = ActivePlacementFlash {
            owner,
            coord: Coord::new(2, 3, 1),
            started_at: 10.0,
        };

        assert_eq!(flash.sample(10.0), Some(PlacementFlash { owner, strength: 1.0 }));
        assert_eq!(flash.sample(10.125), Some(PlacementFlash { owner, strength: 0.5 }));
        assert_eq!(flash.sample(10.25), None);
        assert_eq!(flash.sample(11.0), None);
    }

    #[test]
    fn disabling_placement_flash_clears_an_active_effect() {
        let owner = PrefabInstanceId::from_raw(7).unwrap();
        let mut active = Some(ActivePlacementFlash {
            owner,
            coord: Coord::new(2, 3, 1),
            started_at: 10.0,
        });

        assert_eq!(active_placement_flash(&mut active, 10.0, false), None);
        assert_eq!(active, None);
    }

    #[test]
    fn placement_strokes_process_each_tile_once_until_a_new_stroke_begins() {
        let prefab = Prefab::new(TreePath::parse("/obj/table"));
        let first = Coord::new(2, 3, 1);
        let second = Coord::new(3, 3, 1);
        let mut stroke = PlacementStroke::new(prefab.clone(), 1);

        let group = stroke.visit(first).expect("first tile in stroke");
        assert_eq!(stroke.visit(first), None);
        assert_eq!(stroke.visit(second), Some(group));
        assert_eq!(stroke.visit(first), None);
        assert_eq!(stroke.visit(Coord::new(2, 3, 2)), None);

        let mut next_stroke = PlacementStroke::new(prefab, 1);
        assert!(next_stroke.visit(first).is_some());
    }

    #[test]
    fn placement_strokes_only_match_their_starting_context() {
        let prefab = Prefab::new(TreePath::parse("/obj/table"));
        let other = Prefab::new(TreePath::parse("/obj/chair"));
        let stroke = PlacementStroke::new(prefab.clone(), 2);

        assert!(stroke.matches_context(Tool::Place, Some(&prefab), 2));
        assert!(!stroke.matches_context(Tool::Select, Some(&prefab), 2));
        assert!(!stroke.matches_context(Tool::Place, Some(&other), 2));
        assert!(!stroke.matches_context(Tool::Place, Some(&prefab), 1));
        assert!(!stroke.matches_context(Tool::Place, None, 2));
    }

    #[test]
    fn placement_preview_bounds_preserve_anchor_offsets_dimensions_and_zoom() {
        let mut camera = Controller::new();
        camera.resize(100, 80);
        camera.camera.x = 50.0;
        camera.camera.y = 40.0;
        camera.camera.zoom = 2.0;
        let preview = PlacementPreview {
            thumbnail: crate::session::PrefabThumbnail {
                texture: SpriteTexture {
                    width: 64,
                    height: 32,
                    ..Default::default()
                },
                uv0: [0.0; 2],
                uv1: [1.0; 2],
                tint: [1.0; 4],
            },
            offset: [-16, 4],
        };

        let bounds = placement_preview_bounds(&camera, [10.0, 20.0], Coord::new(2, 2, 1), 32, preview);

        assert_eq!(bounds.min, [-8.0, 4.0]);
        assert_eq!(bounds.max, [120.0, 68.0]);
    }

    #[test]
    fn tool_interaction_configures_highlights_and_pick_requests() {
        let owner = PrefabInstanceId::from_raw(7).unwrap();
        let interaction = ViewportInteraction {
            cursor: Some([10, 20]),
            hovered_area: Some(owner),
            selected: Some(owner),
            selection_guide: None,
            placement_flash: Some(PlacementFlash { owner, strength: 0.5 }),
            mode: InteractionMode::Select {
                pick: Some(PickRequest::Cursor),
            },
        };
        let mut selected = interaction;

        configure_tool_interaction(Tool::Select, &mut selected);
        assert_eq!(selected, interaction);

        let mut delete = interaction;
        configure_tool_interaction(Tool::Delete, &mut delete);
        assert_eq!(
            delete,
            ViewportInteraction {
                selected: None,
                selection_guide: None,
                mode: InteractionMode::Delete { pick: None },
                ..interaction
            }
        );

        configure_tool_interaction(Tool::Place, &mut selected);
        assert_eq!(
            selected,
            ViewportInteraction {
                placement_flash: interaction.placement_flash,
                ..Default::default()
            }
        );

        let mut fill = interaction;
        configure_tool_interaction(Tool::Fill, &mut fill);
        assert_eq!(
            fill,
            ViewportInteraction {
                placement_flash: interaction.placement_flash,
                ..Default::default()
            }
        );
    }

    #[test]
    fn deletion_strokes_only_continue_when_the_cursor_moves() {
        let mut stroke = DeletionStroke::new([10, 20]);

        assert!(!stroke.move_to([10, 20]));
        assert!(stroke.move_to([11, 20]));
        assert!(!stroke.move_to([11, 20]));
        assert!(stroke.move_to([10, 20]));
    }

    #[test]
    fn prefab_tooltips_include_the_path_and_exact_overrides() {
        let mut prefab = Prefab::new(TreePath::parse("/obj/table"));
        prefab.set_var("name".into(), core::types::Value::Text("custom".into()));

        assert_eq!(prefab_tooltip(&prefab), "/obj/table\nname = \"custom\"");
    }

    #[test]
    fn overlay_bounds_exclude_their_far_edges() {
        let bounds = OverlayRect {
            min: [10.0, 20.0],
            max: [30.0, 40.0],
        };

        assert!(bounds.contains([10.0, 20.0]));
        assert!(bounds.contains([29.0, 39.0]));
        assert!(!bounds.contains([30.0, 39.0]));
        assert!(!bounds.contains([29.0, 40.0]));
    }
}
