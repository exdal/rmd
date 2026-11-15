use base::{location::Location, path::TreePath, types::Value};

use dmm::{Map, Size};
use vm::bake::NodeOrientation;

use super::*;

fn tree() -> ObjectTree {
    let mut tree = ObjectTree::new();
    for path in [
        "/atom",
        "/atom/movable",
        "/obj",
        "/obj/cable",
        "/obj/cable/heavy",
        "/obj/pipe",
        "/obj/link",
        "/obj/link/segment",
        "/obj/link/junction",
        "/obj/link/endpoint",
        "/obj/link/broken",
        "/turf",
        "/turf/open",
        "/turf/closed",
        "/turf/closed/wall",
        "/area",
    ] {
        tree.register(&TreePath::parse(path), Location::default());
    }
    for (path, parent) in [
        ("/atom/movable", "/atom"),
        ("/obj", "/atom/movable"),
        ("/turf", "/atom"),
        ("/area", "/atom"),
    ] {
        let id = tree.id_of(&TreePath::parse(path)).unwrap();
        tree.get_mut(id).unwrap().parent_type = Some(TreePath::parse(parent));
    }
    tree.resolve_parent_types();

    tree
}

fn document(width: u32, height: u32, placements: &[(Coord, &str)]) -> MapDocument {
    let mut map = Map::new(Size {
        x: width,
        y: height,
        z: 1,
    });
    for y in 1..=height {
        for x in 1..=width {
            let coord = Coord::new(x, y, 1);
            let turf = placements
                .iter()
                .find_map(|(placed, path)| (*placed == coord && path.starts_with("/turf")).then_some(*path))
                .unwrap_or("/turf/open");
            let mut tile = vec![
                Prefab::new(TreePath::parse(turf)),
                Prefab::new(TreePath::parse("/area")),
            ];
            tile.extend(
                placements
                    .iter()
                    .filter(|(placed, path)| *placed == coord && !path.starts_with("/turf"))
                    .map(|(_, path)| Prefab::new(TreePath::parse(path))),
            );
            let key = map.intern_tile(tile);
            map.grid[0][(height - y) as usize][(x - 1) as usize] = key;
        }
    }

    MapDocument::new(map, 1)
}

fn group_definition(tree: &ObjectTree) -> NodeGroup {
    NodeGroup {
        subtype: tree.id_of(&TreePath::parse("/obj/cable")).unwrap(),
        blockers: vec![tree.id_of(&TreePath::parse("/turf/closed")).unwrap()],
        orientable_subtype: None,
        orientations: Vec::new(),
        matching: Vec::new(),
        ports: NodePortRule::Orientation,
    }
}

fn group(tree: &ObjectTree) -> ResolvedGroup {
    resolve_group(
        tree,
        &[group_definition(tree)],
        0,
        &Prefab::new(TreePath::parse("/obj/cable")),
    )
    .unwrap()
}

fn configured_group(tree: &ObjectTree) -> ResolvedGroup {
    let root = tree.id_of(&TreePath::parse("/obj/link")).unwrap();
    let segment = tree.id_of(&TreePath::parse("/obj/link/segment")).unwrap();
    let junction = tree.id_of(&TreePath::parse("/obj/link/junction")).unwrap();
    let endpoint = tree.id_of(&TreePath::parse("/obj/link/endpoint")).unwrap();
    let orientations = [
        (root, NodePorts::NORTH, NodePorts::all()),
        (segment, NodePorts::NORTH, NodePorts::VERTICAL),
        (segment, NodePorts::SOUTH, NodePorts::VERTICAL),
        (segment, NodePorts::EAST, NodePorts::HORIZONTAL),
        (segment, NodePorts::WEST, NodePorts::HORIZONTAL),
        (segment, NodePorts::NORTHEAST, NodePorts::NORTHEAST),
        (segment, NodePorts::SOUTHEAST, NodePorts::SOUTHEAST),
        (segment, NodePorts::NORTHWEST, NodePorts::NORTHWEST),
        (segment, NodePorts::SOUTHWEST, NodePorts::SOUTHWEST),
        (
            junction,
            NodePorts::NORTH,
            NodePorts::NORTH | NodePorts::EAST | NodePorts::SOUTH,
        ),
        (endpoint, NodePorts::SOUTH, NodePorts::SOUTH),
    ]
    .into_iter()
    .map(|(subtype, direction, openings)| NodeOrientation {
        subtype,
        direction: direction.bits(),
        openings: openings.bits(),
    })
    .collect::<Vec<_>>();
    resolve_group(
        tree,
        &[NodeGroup {
            subtype: root,
            blockers: Vec::new(),
            orientable_subtype: Some(segment),
            orientations,
            matching: Vec::new(),
            ports: NodePortRule::Orientation,
        }],
        0,
        &Prefab::new(TreePath::parse("/obj/link/segment")),
    )
    .unwrap()
}

fn configured_document(placements: &[(Coord, &str, NodePorts)]) -> MapDocument {
    let paths = placements
        .iter()
        .map(|(coord, path, ..)| (*coord, *path))
        .collect::<Vec<_>>();
    let mut document = document(5, 5, &paths);
    for (coord, path, dir) in placements {
        let id = document
            .instance_ids_at(*coord)
            .iter()
            .copied()
            .find(|id| {
                document
                    .prefab_instance(*id)
                    .is_some_and(|instance| instance.prefab().path.to_string() == *path)
            })
            .unwrap();
        document.set_instance_var(id, "dir".into(), Value::Num(dir.bits() as f32));
    }
    document
}

fn layered_group(tree: &ObjectTree, seed: &Prefab) -> ResolvedGroup {
    resolve_group(
        tree,
        &[NodeGroup {
            subtype: tree.id_of(&TreePath::parse("/obj/pipe")).unwrap(),
            blockers: vec![tree.id_of(&TreePath::parse("/turf/closed")).unwrap()],
            orientable_subtype: None,
            orientations: Vec::new(),
            matching: vec![Identifier::from("piping_layer")],
            ports: NodePortRule::Orientation,
        }],
        0,
        seed,
    )
    .unwrap()
}

fn layered_document(width: u32, height: u32, pipes: &[(Coord, &[u32])]) -> MapDocument {
    let paths = pipes
        .iter()
        .flat_map(|(coord, layers)| layers.iter().map(|_| (*coord, "/obj/pipe")))
        .collect::<Vec<_>>();
    let mut document = document(width, height, &paths);
    for (coord, layers) in pipes {
        let ids = document
            .instance_ids_at(*coord)
            .iter()
            .copied()
            .filter(|id| {
                document
                    .prefab_instance(*id)
                    .is_some_and(|instance| instance.prefab().path.to_string() == "/obj/pipe")
            })
            .collect::<Vec<_>>();
        for (id, layer) in ids.into_iter().zip(layers.iter()) {
            document.set_instance_var(id, "piping_layer".into(), Value::Num(*layer as f32));
        }
    }

    document
}

fn layer_of(document: &MapDocument, id: PrefabInstanceId) -> Option<Value> {
    document
        .prefab_instance(id)?
        .prefab()
        .var(&Identifier::from("piping_layer"))
        .cloned()
}

#[test]
fn matching_vars_split_adjacent_and_stacked_pipes_by_layer() {
    let tree = tree();
    let west = Coord::new(1, 1, 1);
    let stacked = Coord::new(2, 1, 1);
    let east = Coord::new(3, 1, 1);
    let north = Coord::new(1, 2, 1);
    let document = layered_document(3, 2, &[(west, &[2]), (stacked, &[2, 4]), (east, &[4]), (north, &[4])]);
    let layer2 = layered_group(&tree, document.placed_tile(west).unwrap().last().unwrap().prefab());
    let layer4 = layered_group(&tree, document.placed_tile(east).unwrap().last().unwrap().prefab());

    assert_eq!(
        component(&document, &tree, &layer2, west).unwrap().tiles,
        HashSet::from([west, stacked])
    );
    assert_eq!(
        component(&document, &tree, &layer4, east).unwrap().tiles,
        HashSet::from([stacked, east])
    );
    assert_eq!(
        component(&document, &tree, &layer4, north).unwrap().tiles,
        HashSet::from([north]),
        "an adjacent pipe on another layer is a separate network"
    );
    assert!(!occupied(&document, &tree, &layer2, east));
    assert_eq!(
        layer_of(
            &document,
            eligible_instance_at(&document, &tree, &layer2, stacked).unwrap()
        ),
        Some(Value::Num(2.0))
    );
    assert_eq!(
        layer_of(
            &document,
            eligible_instance_at(&document, &tree, &layer4, stacked).unwrap()
        ),
        Some(Value::Num(4.0))
    );
}

#[test]
fn configured_components_follow_facing_ports_including_junctions_and_endpoints() -> Result<(), &'static str> {
    let tree = tree();
    let group = configured_group(&tree);
    let junction = Coord::new(2, 2, 1);
    let east = Coord::new(3, 2, 1);
    let west = Coord::new(1, 2, 1);
    let north = Coord::new(2, 3, 1);
    let document = configured_document(&[
        (junction, "/obj/link/junction", NodePorts::NORTH),
        (east, "/obj/link/segment", NodePorts::EAST),
        (west, "/obj/link/segment", NodePorts::EAST),
        (north, "/obj/link/endpoint", NodePorts::SOUTH),
    ]);
    let network = component(&document, &tree, &group, junction).unwrap();
    assert_eq!(network.tiles, HashSet::from([junction, east, north]));
    assert_eq!(network.segments.len(), 2);
    assert_eq!(
        component(&document, &tree, &group, west).unwrap().tiles,
        HashSet::from([west])
    );
    let brush = document
        .prefab_instance(eligible_instance_at(&document, &tree, &group, west).ok_or("eligible instance is missing")?)
        .ok_or("prefab instance is missing")?
        .prefab();
    assert!(oriented_route_directions(&document, &tree, &group, brush, &[west, junction], &HashMap::new()).is_none());
    assert!(oriented_route_directions(&document, &tree, &group, brush, &[east, junction], &HashMap::new()).is_some());
    Ok(())
}

#[test]
fn configured_route_orients_straights_bends_and_rejects_a_third_port() -> Result<(), &'static str> {
    let tree = tree();
    let group = configured_group(&tree);
    let start = Coord::new(1, 1, 1);
    let middle = Coord::new(2, 1, 1);
    let end = Coord::new(2, 2, 1);
    let document = configured_document(&[(start, "/obj/link/segment", NodePorts::NORTH)]);
    let brush = document
        .prefab_instance(eligible_instance_at(&document, &tree, &group, start).ok_or("eligible instance is missing")?)
        .ok_or("prefab instance is missing")?
        .prefab();
    let dirs =
        oriented_route_directions(&document, &tree, &group, brush, &[start, middle, end], &HashMap::new()).unwrap();
    assert_eq!(dirs[&start], NodePorts::EAST.bits());
    assert_eq!(dirs[&middle], NodePorts::NORTHWEST.bits());
    assert_eq!(dirs[&end], NodePorts::NORTH.bits());

    let center = Coord::new(3, 3, 1);
    let north = Coord::new(3, 4, 1);
    let south = Coord::new(3, 2, 1);
    let west = Coord::new(2, 3, 1);
    let document = configured_document(&[
        (center, "/obj/link/segment", NodePorts::NORTH),
        (north, "/obj/link/segment", NodePorts::NORTH),
        (south, "/obj/link/segment", NodePorts::NORTH),
        (west, "/obj/link/segment", NodePorts::EAST),
    ]);
    let brush = document
        .prefab_instance(eligible_instance_at(&document, &tree, &group, west).ok_or("eligible instance is missing")?)
        .ok_or("prefab instance is missing")?
        .prefab();
    assert!(oriented_route_directions(&document, &tree, &group, brush, &[west, center], &HashMap::new()).is_none());
    Ok(())
}

#[test]
fn configured_bends_use_each_matching_diagonal_icon_direction() {
    let tree = tree();
    let group = configured_group(&tree);
    let center = Coord::new(3, 3, 1);
    let document = document(5, 5, &[]);
    let brush = Prefab::new(TreePath::parse("/obj/link/segment"));
    for (first, last, expected) in [
        (Coord::new(3, 4, 1), Coord::new(4, 3, 1), NodePorts::NORTHEAST),
        (Coord::new(3, 2, 1), Coord::new(4, 3, 1), NodePorts::SOUTHEAST),
        (Coord::new(3, 4, 1), Coord::new(2, 3, 1), NodePorts::NORTHWEST),
        (Coord::new(3, 2, 1), Coord::new(2, 3, 1), NodePorts::SOUTHWEST),
    ] {
        let directions = oriented_route_directions(
            &document,
            &tree,
            &group,
            &brush,
            &[first, center, last],
            &HashMap::new(),
        )
        .unwrap();
        assert_eq!(directions[&center], expected.bits());
    }
}

#[test]
fn configured_orientations_can_map_icon_directions_independently_of_openings() {
    let tree = tree();
    let mut group = configured_group(&tree);
    let segment = tree.id_of(&TreePath::parse("/obj/link/segment")).unwrap();
    for rule in group
        .definition
        .orientations
        .iter_mut()
        .filter(|rule| rule.subtype == segment)
    {
        if rule.direction == NodePorts::NORTHEAST.bits() {
            rule.direction = NodePorts::NORTHWEST.bits();
        } else if rule.direction == NodePorts::NORTHWEST.bits() {
            rule.direction = NodePorts::NORTHEAST.bits();
        }
    }
    let document = document(5, 5, &[]);
    let brush = Prefab::new(TreePath::parse("/obj/link/segment"));
    let center = Coord::new(3, 3, 1);
    let directions = oriented_route_directions(
        &document,
        &tree,
        &group,
        &brush,
        &[Coord::new(3, 4, 1), center, Coord::new(2, 3, 1)],
        &HashMap::new(),
    )
    .unwrap();
    assert_eq!(directions[&center], NodePorts::NORTHEAST.bits());
}

fn all_sides_group(tree: &ObjectTree) -> ResolvedGroup {
    let mut group = configured_group(tree);
    group.definition.ports = NodePortRule::AllSides;

    group
}

fn segment_brush() -> Prefab {
    Prefab::new(TreePath::parse("/obj/link/segment"))
}

#[test]
fn all_sides_routes_pick_icon_directions_for_straights_and_bends() {
    let tree = tree();
    let group = all_sides_group(&tree);
    let document = document(5, 5, &[]);
    let brush = segment_brush();

    let east_west = [Coord::new(1, 1, 1), Coord::new(2, 1, 1), Coord::new(3, 1, 1)];
    let directions = oriented_route_directions(&document, &tree, &group, &brush, &east_west, &HashMap::new()).unwrap();
    assert!(
        east_west
            .iter()
            .all(|coord| directions[coord] == NodePorts::EAST.bits())
    );

    let north_south = [Coord::new(1, 1, 1), Coord::new(1, 2, 1), Coord::new(1, 3, 1)];
    let directions =
        oriented_route_directions(&document, &tree, &group, &brush, &north_south, &HashMap::new()).unwrap();
    assert!(
        north_south
            .iter()
            .all(|coord| directions[coord] == NodePorts::NORTH.bits())
    );

    let center = Coord::new(3, 3, 1);
    let bend = [Coord::new(3, 4, 1), center, Coord::new(2, 3, 1)];
    let directions = oriented_route_directions(&document, &tree, &group, &brush, &bend, &HashMap::new()).unwrap();
    assert_eq!(directions[&center], NodePorts::NORTHWEST.bits());
}

#[test]
fn all_sides_misdirected_segments_still_join_the_component() {
    let tree = tree();
    let group = all_sides_group(&tree);
    let left = Coord::new(1, 1, 1);
    let right = Coord::new(2, 1, 1);
    let document = configured_document(&[
        (left, "/obj/link/segment", NodePorts::NORTH),
        (right, "/obj/link/segment", NodePorts::NORTH),
    ]);

    let network = component(&document, &tree, &group, left).unwrap();
    assert_eq!(network.tiles, HashSet::from([left, right]));
}

#[test]
fn all_sides_t_branch_leaves_the_existing_segment_direction() {
    let tree = tree();
    let group = all_sides_group(&tree);
    let west = Coord::new(1, 2, 1);
    let middle = Coord::new(2, 2, 1);
    let east = Coord::new(3, 2, 1);
    let branch = Coord::new(2, 1, 1);
    let document = configured_document(&[
        (west, "/obj/link/segment", NodePorts::EAST),
        (middle, "/obj/link/segment", NodePorts::EAST),
        (east, "/obj/link/segment", NodePorts::EAST),
    ]);

    let directions = oriented_route_directions(
        &document,
        &tree,
        &group,
        &segment_brush(),
        &[branch, middle],
        &HashMap::new(),
    )
    .unwrap();
    assert_eq!(directions[&branch], NodePorts::NORTH.bits());
    assert!(!directions.contains_key(&middle));
}

#[test]
fn all_sides_route_beside_a_parallel_line_orients_along_the_route() {
    let tree = tree();
    let group = all_sides_group(&tree);
    let document = configured_document(&[
        (Coord::new(1, 2, 1), "/obj/link/segment", NodePorts::EAST),
        (Coord::new(2, 2, 1), "/obj/link/segment", NodePorts::EAST),
        (Coord::new(3, 2, 1), "/obj/link/segment", NodePorts::EAST),
    ]);
    let route = [Coord::new(1, 1, 1), Coord::new(2, 1, 1), Coord::new(3, 1, 1)];

    let directions =
        oriented_route_directions(&document, &tree, &group, &segment_brush(), &route, &HashMap::new()).unwrap();
    assert_eq!(directions.len(), 3);
    assert!(route.iter().all(|coord| directions[coord] == NodePorts::EAST.bits()));
}

#[test]
fn most_specific_registered_group_wins() {
    let tree = tree();
    let groups = [
        NodeGroup {
            subtype: tree.id_of(&TreePath::parse("/obj")).unwrap(),
            blockers: Vec::new(),
            orientable_subtype: None,
            orientations: Vec::new(),
            matching: Vec::new(),
            ports: NodePortRule::Orientation,
        },
        group_definition(&tree),
    ];
    let prefab = Prefab::new(TreePath::parse("/obj/cable/heavy"));

    assert_eq!(group_for_prefab(&tree, &groups, &prefab), Some(1));
}

#[test]
fn a_broad_group_excludes_types_claimed_by_a_more_specific_group() {
    let tree = tree();
    let groups = [
        NodeGroup {
            subtype: tree.id_of(&TreePath::parse("/obj")).unwrap(),
            blockers: Vec::new(),
            orientable_subtype: None,
            orientations: Vec::new(),
            matching: Vec::new(),
            ports: NodePortRule::Orientation,
        },
        group_definition(&tree),
    ];
    let broad = resolve_group(&tree, &groups, 0, &Prefab::new(TreePath::parse("/obj/pipe"))).unwrap();
    let cable = resolve_group(&tree, &groups, 1, &Prefab::new(TreePath::parse("/obj/cable/heavy"))).unwrap();
    let pipe_coord = Coord::new(1, 1, 1);
    let cable_coord = Coord::new(2, 1, 1);
    let document = document(3, 1, &[(pipe_coord, "/obj/pipe"), (cable_coord, "/obj/cable/heavy")]);

    let broad_component = component(&document, &tree, &broad, pipe_coord).unwrap();
    assert_eq!(broad_component.tiles, HashSet::from([pipe_coord]));
    let cable_component = component(&document, &tree, &cable, cable_coord).unwrap();
    assert_eq!(cable_component.tiles, HashSet::from([cable_coord]));
}

#[test]
fn component_nodes_include_endpoints_elbows_and_junctions() {
    let tree = tree();
    let cable = group(&tree);
    let placements = [
        (Coord::new(1, 1, 1), "/obj/cable"),
        (Coord::new(2, 1, 1), "/obj/cable"),
        (Coord::new(3, 1, 1), "/obj/cable"),
        (Coord::new(3, 2, 1), "/obj/cable"),
        (Coord::new(3, 3, 1), "/obj/cable"),
        (Coord::new(4, 2, 1), "/obj/cable/heavy"),
    ];
    let document = document(5, 4, &placements);
    let component = component(&document, &tree, &cable, Coord::new(1, 1, 1)).unwrap();

    assert_eq!(component.tiles.len(), placements.len());
    assert_eq!(component.segments.len(), placements.len() - 1);
    assert!(component.nodes.contains(&Coord::new(1, 1, 1)), "endpoint");
    assert!(!component.nodes.contains(&Coord::new(2, 1, 1)));
    assert!(component.nodes.contains(&Coord::new(3, 1, 1)), "elbow");
    assert!(component.nodes.contains(&Coord::new(3, 2, 1)), "junction");
    assert!(component.nodes.contains(&Coord::new(3, 3, 1)), "endpoint");
    assert!(component.nodes.contains(&Coord::new(4, 2, 1)), "endpoint");
}

#[test]
fn isolated_component_is_a_node() {
    let tree = tree();
    let cable = group(&tree);
    let coord = Coord::new(2, 2, 1);
    let document = document(3, 3, &[(coord, "/obj/cable")]);

    let component = component(&document, &tree, &cable, coord).unwrap();

    assert_eq!(component.nodes, vec![coord]);
    assert!(connections(&component, &HashSet::new()).is_empty());
}

#[test]
fn connections_are_bounded_by_structural_and_manual_nodes() {
    let tree = tree();
    let cable = group(&tree);
    let placements = [
        (Coord::new(1, 2, 1), "/obj/cable"),
        (Coord::new(2, 2, 1), "/obj/cable"),
        (Coord::new(3, 2, 1), "/obj/cable"),
        (Coord::new(4, 2, 1), "/obj/cable"),
        (Coord::new(5, 2, 1), "/obj/cable"),
        (Coord::new(3, 3, 1), "/obj/cable"),
    ];
    let document = document(5, 3, &placements);
    let component = component(&document, &tree, &cable, Coord::new(1, 2, 1)).unwrap();
    let manual = HashSet::from([Coord::new(2, 2, 1)]);

    let connections = connections(&component, &manual);
    assert_eq!(
        connections,
        vec![
            vec![Coord::new(1, 2, 1), Coord::new(2, 2, 1)],
            vec![Coord::new(2, 2, 1), Coord::new(3, 2, 1)],
            vec![Coord::new(3, 2, 1), Coord::new(4, 2, 1), Coord::new(5, 2, 1)],
            vec![Coord::new(3, 2, 1), Coord::new(3, 3, 1)],
        ]
    );
    assert_eq!(
        connection_at_tile(&connections, Coord::new(4, 2, 1)),
        Some(&connections[2]),
        "an interior tile identifies its connection"
    );
    assert_eq!(
        connection_at_tile(&connections, Coord::new(1, 2, 1)),
        Some(&connections[0]),
        "an endpoint belonging to one connection is unambiguous"
    );
    assert_eq!(
        connection_at_tile(&connections, Coord::new(3, 2, 1)),
        None,
        "a shared node needs the precise screen-space hit test"
    );
}

#[test]
fn route_takes_a_short_cardinal_detour_around_blockers() {
    let tree = tree();
    let cable = group(&tree);
    let start = Coord::new(1, 2, 1);
    let target = Coord::new(5, 2, 1);
    let document = document(
        5,
        3,
        &[(start, "/obj/cable"), (Coord::new(3, 2, 1), "/turf/closed/wall")],
    );
    let path = route(&document, &tree, &cable, start, target).unwrap();

    assert_eq!(path.first(), Some(&start));
    assert_eq!(path.last(), Some(&target));
    assert_eq!(path.len(), 7);
    assert!(!path.contains(&Coord::new(3, 2, 1)));
    assert!(path.windows(2).all(|step| {
        let dx = step[0].x.abs_diff(step[1].x);
        let dy = step[0].y.abs_diff(step[1].y);

        dx + dy == 1
    }));
}

#[test]
fn route_can_begin_away_from_the_target_to_escape_blockers() {
    let tree = tree();
    let cable = group(&tree);
    let start = Coord::new(3, 2, 1);
    let target = Coord::new(5, 2, 1);
    let document = document(
        5,
        4,
        &[
            (start, "/obj/cable"),
            (Coord::new(4, 2, 1), "/turf/closed/wall"),
            (Coord::new(3, 1, 1), "/turf/closed/wall"),
            (Coord::new(3, 3, 1), "/turf/closed/wall"),
        ],
    );

    let path = route(&document, &tree, &cable, start, target).expect("the westward escape reaches the target");

    assert_eq!(path.first(), Some(&start));
    assert_eq!(path.get(1), Some(&Coord::new(2, 2, 1)));
    assert_eq!(path.last(), Some(&target));
    assert_eq!(path.len(), 9);
    assert!(
        path.windows(2)
            .all(|step| step[0].x.abs_diff(step[1].x) + step[0].y.abs_diff(step[1].y) == 1)
    );
}

#[test]
fn route_loop_erasure_removes_a_reversed_spur() {
    let elbow = Coord::new(3, 2, 1);
    let west = Coord::new(2, 2, 1);
    let east = Coord::new(4, 2, 1);

    assert_eq!(erase_loops(vec![elbow, west, elbow, east]), vec![elbow, east]);
}

#[test]
fn route_reuses_group_tiles_and_reports_a_sealed_target_unreachable() {
    let tree = tree();
    let cable = group(&tree);
    let start = Coord::new(1, 2, 1);
    let existing = Coord::new(2, 2, 1);
    let target = Coord::new(4, 2, 1);
    let open = document(4, 3, &[(start, "/obj/cable"), (existing, "/obj/cable/heavy")]);
    let path = route(&open, &tree, &cable, start, target).unwrap();
    assert!(path.contains(&existing));

    let sealed = document(
        4,
        3,
        &[
            (start, "/obj/cable"),
            (Coord::new(2, 1, 1), "/turf/closed/wall"),
            (Coord::new(2, 2, 1), "/turf/closed/wall"),
            (Coord::new(2, 3, 1), "/turf/closed/wall"),
        ],
    );
    assert_eq!(route(&sealed, &tree, &cable, start, target), None);
}

#[test]
fn route_can_pass_beside_another_same_group_component() {
    let tree = tree();
    let cable = group(&tree);
    let start = Coord::new(3, 3, 1);
    let target = Coord::new(5, 1, 1);
    let unrelated = HashSet::from([Coord::new(1, 1, 1), Coord::new(2, 1, 1)]);
    let document = document(
        5,
        4,
        &[
            (start, "/obj/cable"),
            (Coord::new(4, 3, 1), "/turf/closed/wall"),
            (Coord::new(1, 1, 1), "/obj/cable"),
            (Coord::new(2, 1, 1), "/obj/cable"),
        ],
    );
    let path = route(&document, &tree, &cable, start, target).unwrap();

    assert_eq!(path.first(), Some(&start));
    assert_eq!(path.last(), Some(&target));
    assert!(path.iter().all(|coord| !unrelated.contains(coord)));
}

#[test]
fn route_can_intentionally_connect_to_an_occupied_target() {
    let tree = tree();
    let cable = group(&tree);
    let start = Coord::new(1, 2, 1);
    let target = Coord::new(4, 2, 1);
    let document = document(4, 3, &[(start, "/obj/cable"), (target, "/obj/cable/heavy")]);
    let path = route(&document, &tree, &cable, start, target).unwrap();

    assert_eq!(path.first(), Some(&start));
    assert_eq!(path.last(), Some(&target));
}

#[test]
fn route_merges_when_the_target_is_next_to_the_same_group() {
    let tree = tree();
    let cable = group(&tree);
    let start = Coord::new(1, 2, 1);
    let target = Coord::new(3, 2, 1);
    let neighbor = Coord::new(4, 2, 1);
    let document = document(4, 3, &[(start, "/obj/cable"), (neighbor, "/obj/cable/heavy")]);
    let path = route(&document, &tree, &cable, start, target).unwrap();

    assert_eq!(path.first(), Some(&start));
    assert_eq!(path.last(), Some(&target));
    assert!(neighbors(target, 4, 3).any(|coord| coord == neighbor));
}
