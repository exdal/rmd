use core::path::TreePath;
use std::{
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

use dear_imgui_rs::{Condition, DockLayout, Key, MouseButton};
use dmm::{Coord, Prefab, PrefabInstanceId, Size};
use editor::{
    conflict::Side,
    document::{DocumentId, MapDocument, Selection},
    tool::{BlockSelectionMode, SelectionMask, SelectionRotation, Tool},
};
use render::{HighlightStyle, InteractionMode, MapViewInteraction, MapViewRect, PickRequest, PlacementFlash};

use super::{
    ActivePlacementFlash,
    CoopPresence,
    EditCommand,
    MapViewState,
    PickStroke,
    PlacementStroke,
    cursor_in_map_view,
    framebuffer_rect,
    panel_extent,
    stroke::active_placement_flash,
    tools::{configure_tool_interaction, request_pick},
};
use crate::{
    camera::Controller,
    gizmo::{BlockGizmoKind, BlockGizmoTarget, GizmoMapView, GizmoState},
    session::{
        LevelChange,
        Session,
        fixtures::{install_blame, node_map, node_session, node_tile_has_group},
    },
    settings::{KeyBinding, KeybindAction, KeybindPreset, Settings},
    ui::{
        IMGUI_CONTEXT,
        RectangleGesture,
        UiState,
        block::block_selection_bounds,
        context_menu::NodeContext,
        fixtures::{RectangleUiHarness, rectangle_context},
        inspector::TransformMode,
        restore_rectangle_gesture,
    },
};

mod hover;
mod navigation;
mod presence;
mod selection;
mod tools;

#[test]
fn a_map_view_rect_is_measured_in_framebuffer_pixels() {
    // ImGui lays out in logical units; the render target is physical pixels.
    let rect = framebuffer_rect([100.0, 50.0], [400.0, 300.0], [1.5, 1.5], [1000.0, 800.0]);

    assert_eq!(rect.x, 150);
    assert_eq!(rect.y, 75);
    assert_eq!(rect.width, 600);
    assert_eq!(rect.height, 450);
}

#[test]
fn a_map_view_rect_is_the_same_at_unit_scale() {
    let rect = framebuffer_rect([0.0, 22.0], [640.0, 480.0], [1.0, 1.0], [1280.0, 720.0]);

    assert_eq!(
        rect,
        MapViewRect {
            x: 0,
            y: 22,
            width: 640,
            height: 480,
        }
    );
}

#[test]
fn a_map_view_rect_is_clamped_to_the_target() {
    // Position stays inside the main framebuffer, while the independent map
    // attachment keeps its full size when the ImGui window is clipped.
    let rect = framebuffer_rect([-40.0, -10.0], [200.0, 100.0], [1.0, 1.0], [120.0, 60.0]);

    assert_eq!(rect.x, 0);
    assert_eq!(rect.y, 0);
    assert_eq!(rect.width, 200);
    assert_eq!(rect.height, 100);

    let collapsed = framebuffer_rect([10.0, 10.0], [0.0, 0.0], [1.0, 1.0], [100.0, 100.0]);
    assert!(collapsed.is_empty(), "a zero-size split contributes no draw");
}

#[test]
fn a_cursor_is_local_to_its_map_view() {
    // Picking chooses a visibility attachment by hovered ImGui window, so
    // its cursor stays local regardless of where that window is placed.
    assert_eq!(cursor_in_map_view([10.0, 20.0], [1.0, 1.0]), [10, 20]);
}

#[test]
fn a_cursor_scales_with_the_display() {
    assert_eq!(cursor_in_map_view([30.0, 40.0], [2.0, 2.0]), [60, 80]);
}

#[test]
fn every_document_gets_its_own_map_view_window_key() {
    let first = DocumentId::new();
    let second = DocumentId::new();
    let first = MapViewState::new(first).expect("valid window key");
    let second = MapViewState::new(second).expect("valid window key");

    // Two windows sharing a key would be one window, and the dock layout
    // compiler rejects the duplicate outright.
    assert_ne!(first.window.stable_id(), second.window.stable_id());
    assert!(DockLayout::tabs([&first.window, &second.window]).validate().is_ok());
}

#[test]
fn a_new_map_view_starts_out_wanting_to_frame_its_map() {
    let view = MapViewState::new(DocumentId::new()).expect("valid window key");

    assert!(view.refit);
    assert!(!view.focus);
    assert!(view.gestures.block_selection_anchor.is_none());
    assert!(view.gestures.block_placement.is_none());
}

#[test]
fn panel_extent_rejects_non_finite_or_empty_sizes() {
    assert_eq!(panel_extent([0.0, f32::NAN]), ([1.0, 1.0], (1, 1)));
    assert_eq!(panel_extent([320.75, 200.25]), ([320.75, 200.25], (320, 200)));
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
fn tool_interaction_configures_highlights_and_pick_requests() {
    let owner = PrefabInstanceId::from_raw(7).unwrap();
    let interaction = MapViewInteraction {
        cursor: Some([10, 20]),
        selected: Some(owner),
        placement_flash: Some(PlacementFlash { owner, strength: 0.5 }),
        highlight: HighlightStyle::Tint,
        mode: InteractionMode::Select {
            pick: Some(PickRequest::Select),
        },
    };
    let mut selected = interaction;

    configure_tool_interaction(Tool::Select, &mut selected);
    assert_eq!(selected, interaction);

    let mut delete = interaction;
    configure_tool_interaction(Tool::Delete, &mut delete);
    assert_eq!(
        delete,
        MapViewInteraction {
            selected: None,
            mode: InteractionMode::Delete { pick: None },
            ..interaction
        }
    );

    configure_tool_interaction(Tool::Place, &mut selected);
    assert_eq!(
        selected,
        MapViewInteraction {
            placement_flash: interaction.placement_flash,
            highlight: interaction.highlight,
            ..Default::default()
        }
    );

    let mut node = interaction;
    configure_tool_interaction(Tool::Node, &mut node);
    assert_eq!(
        node,
        MapViewInteraction {
            placement_flash: interaction.placement_flash,
            highlight: interaction.highlight,
            mode: InteractionMode::Select { pick: None },
            ..Default::default()
        }
    );
    let coord = Coord::new(3, 4, 1);
    request_pick(&mut node, PickRequest::NodeSeed(coord));
    assert_eq!(
        node.mode,
        InteractionMode::Select {
            pick: Some(PickRequest::NodeSeed(coord)),
        }
    );
    request_pick(&mut node, PickRequest::NodeDelete(coord));
    assert_eq!(
        node.mode,
        InteractionMode::Select {
            pick: Some(PickRequest::NodeDelete(coord)),
        }
    );

    let mut fill = interaction;
    configure_tool_interaction(Tool::Fill, &mut fill);
    assert_eq!(
        fill,
        MapViewInteraction {
            placement_flash: interaction.placement_flash,
            highlight: interaction.highlight,
            ..Default::default()
        }
    );

    let mut block = interaction;
    configure_tool_interaction(Tool::BlockSelect, &mut block);
    assert_eq!(
        block,
        MapViewInteraction {
            placement_flash: interaction.placement_flash,
            highlight: interaction.highlight,
            ..Default::default()
        }
    );
}

#[test]
fn pick_strokes_only_continue_when_the_cursor_moves() {
    let mut stroke = PickStroke::new([10, 20], 1);

    assert!(!stroke.move_to([10, 20]));
    assert!(stroke.move_to([11, 20]));
    assert!(!stroke.move_to([11, 20]));
    assert!(stroke.move_to([10, 20]));
}
