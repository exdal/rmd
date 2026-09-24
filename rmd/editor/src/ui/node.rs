use dear_imgui_rs::Ui;
use dmm::Coord;
use editor::{node, tool::Tool};

use crate::{camera::Controller, session::NodeOverlay};

const NODE_HANDLE_RADIUS: f32 = 6.0;

const NODE_CONNECTION_TOLERANCE: f32 = 6.0;

#[derive(Default)]
pub(super) struct NodeOverlayHit {
    pub(super) handle: Option<Coord>,
    pub(super) standalone: Option<Coord>,
    pub(super) connection: Option<Vec<Coord>>,
}

pub(super) enum NodeRightClick<'a> {
    DeleteStandalone(Coord),
    DeleteConnection(&'a [Coord]),
    Menu,
}

pub(super) fn node_right_click(tool: Tool, shift: bool, hit: &NodeOverlayHit) -> NodeRightClick<'_> {
    if tool == Tool::Node && !shift {
        if let Some(coord) = hit.standalone {
            return NodeRightClick::DeleteStandalone(coord);
        }

        if let Some(connection) = hit.connection.as_deref() {
            return NodeRightClick::DeleteConnection(connection);
        }
    }

    NodeRightClick::Menu
}

pub(super) struct NodeOverlayView<'a> {
    pub(super) camera: &'a Controller,
    pub(super) viewport_min: [f32; 2],
    pub(super) viewport_max: [f32; 2],
    pub(super) tile_size: u32,
    pub(super) hovered_tile: Option<Coord>,
    pub(super) interactive: bool,
}

fn point_segment_distance_squared(point: [f32; 2], from: [f32; 2], to: [f32; 2]) -> f32 {
    let delta = [to[0] - from[0], to[1] - from[1]];
    let length_squared = delta[0].powi(2) + delta[1].powi(2);
    if length_squared == 0.0 {
        return (point[0] - from[0]).powi(2) + (point[1] - from[1]).powi(2);
    }

    let offset = [point[0] - from[0], point[1] - from[1]];
    let t = ((offset[0] * delta[0] + offset[1] * delta[1]) / length_squared).clamp(0.0, 1.0);
    let closest = [from[0] + delta[0] * t, from[1] + delta[1] * t];

    (point[0] - closest[0]).powi(2) + (point[1] - closest[1]).powi(2)
}

fn closest_node_connection(
    mouse: [f32; 2], connections: &[Vec<Coord>], center: impl Fn(Coord) -> [f32; 2], tolerance: f32,
) -> Option<usize> {
    let mut nearest = None;
    for (index, connection) in connections.iter().enumerate() {
        for segment in connection.windows(2) {
            let distance = point_segment_distance_squared(mouse, center(segment[0]), center(segment[1]));
            if distance <= tolerance.powi(2)
                && nearest.is_none_or(|(best, best_index)| distance < best || distance == best && index < best_index)
            {
                nearest = Some((distance, index));
            }
        }
    }

    nearest.map(|(_, index)| index)
}

fn hit_node_overlay(
    overlay: &NodeOverlay, mouse: [f32; 2], hovered_tile: Option<Coord>, center: impl Fn(Coord) -> [f32; 2],
    interactive: bool,
) -> NodeOverlayHit {
    if !interactive {
        return NodeOverlayHit::default();
    }

    let hovered = overlay
        .nodes
        .iter()
        .copied()
        .filter_map(|coord| {
            let position = center(coord);
            let distance = (position[0] - mouse[0]).powi(2) + (position[1] - mouse[1]).powi(2);

            (distance <= (NODE_HANDLE_RADIUS + 3.0).powi(2)).then_some((distance, coord))
        })
        .min_by(|left, right| left.0.total_cmp(&right.0))
        .map(|(_, coord)| coord);
    let hovered_connection = if let Some(handle) = hovered {
        let mut attached = overlay
            .connections
            .iter()
            .enumerate()
            .filter(|(_, connection)| connection.first() == Some(&handle) || connection.last() == Some(&handle));
        let first = attached.next().map(|(index, _)| index);
        if attached.next().is_none() { first } else { None }
    } else {
        closest_node_connection(mouse, &overlay.connections, &center, NODE_CONNECTION_TOLERANCE).or_else(|| {
            hovered_tile
                .and_then(|coord| node::connection_at_tile(&overlay.connections, coord))
                .and_then(|connection| overlay.connections.iter().position(|current| current == connection))
        })
    };
    let standalone = hovered.filter(|coord| !overlay.segments.iter().any(|(from, to)| from == coord || to == coord));

    NodeOverlayHit {
        handle: hovered,
        standalone,
        connection: hovered_connection.and_then(|index| overlay.connections.get(index).cloned()),
    }
}

pub(super) fn draw_node_overlay(ui: &Ui, overlay: &NodeOverlay, view: NodeOverlayView<'_>) -> NodeOverlayHit {
    const EDGE: [f32; 4] = [0.15, 0.78, 1.0, 0.9];
    const CONNECTION_HOVER: [f32; 4] = [1.0, 0.25, 0.2, 1.0];
    const ROUTE: [f32; 4] = [0.25, 1.0, 0.45, 1.0];
    const INVALID_ROUTE: [f32; 4] = [1.0, 0.25, 0.2, 1.0];
    const HANDLE: [f32; 4] = [1.0, 0.58, 0.08, 1.0];
    const HANDLE_HOVER: [f32; 4] = [1.0, 0.88, 0.45, 1.0];
    let NodeOverlayView {
        camera,
        viewport_min,
        viewport_max,
        tile_size,
        hovered_tile,
        interactive,
    } = view;

    let center = |coord: Coord| {
        let tile_size = tile_size.max(1) as f32;
        let local = camera.map_to_screen([(coord.x as f32 - 0.5) * tile_size, (coord.y as f32 - 0.5) * tile_size]);

        [viewport_min[0] + local[0], viewport_min[1] + local[1]]
    };
    let hit = hit_node_overlay(overlay, ui.io().mouse_pos(), hovered_tile, center, interactive);

    let draw = ui.get_window_draw_list();
    draw.with_clip_rect(viewport_min, viewport_max, || {
        for (from, to) in &overlay.segments {
            draw.add_line(center(*from), center(*to), EDGE).thickness(2.0).build();
        }
        if let Some(connection) = hit.connection.as_ref() {
            for segment in connection.windows(2) {
                draw.add_line(center(segment[0]), center(segment[1]), CONNECTION_HOVER)
                    .thickness(4.0)
                    .build();
            }
        }
        let route_color = if overlay.route_valid { ROUTE } else { INVALID_ROUTE };
        for segment in overlay.route.windows(2) {
            draw.add_line(center(segment[0]), center(segment[1]), route_color)
                .thickness(4.0)
                .build();
        }
        for coord in &overlay.nodes {
            let color = if hit.standalone == Some(*coord) {
                CONNECTION_HOVER
            } else if hit.handle == Some(*coord) {
                HANDLE_HOVER
            } else {
                HANDLE
            };
            draw.add_circle(center(*coord), NODE_HANDLE_RADIUS, [0.05, 0.05, 0.05, 0.95])
                .filled(true)
                .build();
            draw.add_circle(center(*coord), NODE_HANDLE_RADIUS - 2.0, color)
                .filled(true)
                .build();
        }
    });

    hit
}

#[cfg(test)]
mod tests {
    use dmm::Coord;
    use editor::tool::Tool;

    use super::{NodeOverlayHit, NodeRightClick, closest_node_connection, hit_node_overlay, node_right_click};
    use crate::session::NodeOverlay;

    #[test]
    fn node_connection_hit_test_uses_screen_distance_and_includes_adjacent_nodes() {
        let connections = vec![
            vec![Coord::new(1, 1, 1), Coord::new(2, 1, 1)],
            vec![Coord::new(1, 2, 1), Coord::new(2, 2, 1), Coord::new(3, 2, 1)],
            vec![Coord::new(4, 1, 1), Coord::new(4, 2, 1), Coord::new(4, 3, 1)],
        ];
        let center = |coord: Coord| [coord.x as f32 * 10.0, coord.y as f32 * 10.0];

        assert_eq!(
            closest_node_connection([20.0, 24.0], &connections, center, 6.0),
            Some(1)
        );
        assert_eq!(
            closest_node_connection([36.0, 20.0], &connections, center, 6.0),
            Some(2)
        );
        assert_eq!(
            closest_node_connection([15.0, 10.0], &connections, center, 6.0),
            Some(0)
        );
        assert_eq!(closest_node_connection([20.0, 27.0], &connections, center, 6.0), None);
    }

    #[test]
    fn node_overlay_hit_distinguishes_segments_endpoints_junctions_and_standalone_nodes() {
        let left = Coord::new(1, 1, 1);
        let junction = Coord::new(2, 1, 1);
        let right = Coord::new(3, 1, 1);
        let upper = Coord::new(2, 2, 1);
        let standalone = Coord::new(5, 5, 1);
        let connection = vec![left, junction];
        let overlay = NodeOverlay {
            nodes: vec![left, junction, right, upper, standalone],
            segments: vec![(left, junction), (junction, right), (junction, upper)],
            connections: vec![connection.clone(), vec![junction, right], vec![junction, upper]],
            route: Vec::new(),
            route_valid: true,
        };
        let center = |coord: Coord| [coord.x as f32 * 32.0, coord.y as f32 * 32.0];

        let segment = hit_node_overlay(&overlay, [48.0, 32.0], Some(left), center, true);
        assert_eq!(segment.handle, None);
        assert_eq!(segment.connection, Some(connection.clone()));

        let endpoint = hit_node_overlay(&overlay, center(left), Some(left), center, true);
        assert_eq!(endpoint.handle, Some(left));
        assert_eq!(endpoint.connection, Some(connection));

        let branch = hit_node_overlay(&overlay, center(junction), Some(junction), center, true);
        assert_eq!(branch.handle, Some(junction));
        assert!(branch.connection.is_none());

        let isolated = hit_node_overlay(&overlay, center(standalone), Some(standalone), center, true);
        assert_eq!(isolated.standalone, Some(standalone));
        assert!(isolated.connection.is_none());
    }

    #[test]
    fn node_right_click_deletes_known_geometry_unless_shift_opens_the_menu() {
        let node = Coord::new(2, 3, 1);
        let connection = vec![node, Coord::new(3, 3, 1)];
        let connected = NodeOverlayHit {
            connection: Some(connection.clone()),
            ..Default::default()
        };
        let standalone = NodeOverlayHit {
            standalone: Some(node),
            ..Default::default()
        };

        assert!(matches!(
            node_right_click(Tool::Node, false, &connected),
            NodeRightClick::DeleteConnection(target) if target == connection
        ));
        assert!(matches!(
            node_right_click(Tool::Node, false, &standalone),
            NodeRightClick::DeleteStandalone(target) if target == node
        ));
        assert!(matches!(
            node_right_click(Tool::Node, true, &connected),
            NodeRightClick::Menu
        ));
        assert!(matches!(
            node_right_click(Tool::Node, false, &NodeOverlayHit::default()),
            NodeRightClick::Menu
        ));
        assert!(matches!(
            node_right_click(Tool::Select, false, &connected),
            NodeRightClick::Menu
        ));
    }
}
