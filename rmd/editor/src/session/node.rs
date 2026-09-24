use core::types::{Identifier, Value};
use std::collections::{HashMap, HashSet};

use dmm::{Coord, Prefab};
use editor::{
    command::{Edit, EditGroupId},
    document::{DocumentId, PlacedTile, PrefabInstanceId},
    node,
    tool::{FillMode, Tool, ToolContext, ToolEdit},
    visual,
};

use super::Session;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NodeOverlay {
    pub nodes: Vec<Coord>,
    pub segments: Vec<(Coord, Coord)>,
    pub connections: Vec<node::Connection>,
    pub route: Vec<Coord>,
    pub route_valid: bool,
}

#[derive(Debug)]
struct NodeDrag {
    start: Coord,
    group: EditGroupId,
    owned: HashMap<Coord, PrefabInstanceId>,
    original: HashMap<Coord, PlacedTile>,
    route: Vec<Coord>,
    valid: bool,
}

#[derive(Debug)]
pub(super) struct NodeEditState {
    pub(super) document: DocumentId,
    z: u32,
    group: node::ResolvedGroup,
    seed: Coord,
    brush: Prefab,
    manual: HashSet<Coord>,
    drag: Option<NodeDrag>,
}

impl Session {
    pub(crate) fn node_tool_available(&self) -> bool {
        let Some(id) = self.state.active() else {
            return false;
        };

        self.state
            .environment
            .as_ref()
            .and_then(|environment| environment.bake_program.as_ref())
            .is_some()
            && self
                .caches
                .get(&id)
                .and_then(|cache| cache.bake.as_ref())
                .is_some_and(|bake| !bake.node_groups().is_empty())
    }

    pub(crate) fn node_candidate_from_pick(
        &self, picked: Option<PrefabInstanceId>, coord: Coord,
    ) -> Option<PrefabInstanceId> {
        let id = self.state.active()?;
        let environment = self.state.environment.as_ref()?;
        let tree = &environment.bake_program.as_ref()?.tree;
        let groups = self.caches.get(&id)?.bake.as_ref()?.node_groups();
        let document = self.state.document(id)?;
        if let Some(picked) = picked
            && let Some((prefab, _)) = document.prefab_instance(picked)
        {
            if node::group_for_prefab(tree, groups, prefab).is_some() {
                return Some(picked);
            }

            let ty = tree.id_of(&prefab.path)?;
            let roots = tree.roots();
            let floor_or_area = roots.turf.is_some_and(|root| tree.is_subtype_of(ty, root))
                || roots.area.is_some_and(|root| tree.is_subtype_of(ty, root));
            if !floor_or_area {
                return None;
            }
        }

        document.instance_ids_at(coord).iter().rev().find_map(|instance| {
            let (prefab, _) = document.prefab_instance(*instance)?;

            node::group_for_prefab(tree, groups, prefab)
                .is_some()
                .then_some(*instance)
        })
    }

    pub(crate) fn begin_node_edit(&mut self, target: PrefabInstanceId) -> bool {
        self.cancel_node_drag();
        let Some(document_id) = self.state.active() else {
            return false;
        };
        let Some(environment) = self.state.environment.as_ref() else {
            return false;
        };
        let Some(program) = environment.bake_program.as_ref() else {
            return false;
        };
        let Some(document) = self.state.document(document_id) else {
            return false;
        };
        let Some((prefab, location)) = document.prefab_instance(target) else {
            return false;
        };
        if location.coord.z != document.z {
            return false;
        }
        let Some(groups) = self
            .caches
            .get(&document_id)
            .and_then(|cache| cache.bake.as_ref())
            .map(vm::bake::Bake::node_groups)
        else {
            return false;
        };
        let Some(group_index) = node::group_for_prefab(&program.tree, groups, prefab) else {
            return false;
        };
        let Some(group) = node::resolve_group(&program.tree, groups, group_index) else {
            return false;
        };
        let coord = location.coord;
        let brush = prefab.clone();
        let mut manual = HashSet::from([coord]);
        if let Some(previous) = self.node_edit.as_ref()
            && previous.document == document_id
            && previous.z == document.z
            && previous.group.subtype() == group.subtype()
            && let Some(component) = node::component(document, &program.tree, &group, coord)
            && component.tiles.contains(&previous.seed)
        {
            manual.extend(previous.manual.iter().copied());
        }

        self.node_edit = Some(NodeEditState {
            document: document_id,
            z: document.z,
            group,
            seed: coord,
            brush,
            manual,
            drag: None,
        });
        self.state.tool = Tool::Node;
        self.select_instance(Some(target));

        true
    }

    pub(crate) fn node_overlay(&self) -> Option<NodeOverlay> {
        let state = self.node_edit.as_ref()?;
        if self.state.tool != Tool::Node || self.state.active() != Some(state.document) || self.z() != state.z {
            return None;
        }
        let environment = self.state.environment.as_ref()?;
        let tree = &environment.bake_program.as_ref()?.tree;
        let document = self.state.document(state.document)?;
        let mut component = node::component(document, tree, &state.group, state.seed)?;
        component.nodes.extend(
            state
                .manual
                .iter()
                .copied()
                .filter(|coord| component.tiles.contains(coord)),
        );
        if let Some(endpoint) = state.drag.as_ref().and_then(|drag| drag.route.last()).copied() {
            component.nodes.push(endpoint);
        }
        component.nodes.sort_unstable_by_key(|coord| (coord.y, coord.x));
        component.nodes.dedup();
        let connections = node::connections(&component, &state.manual);

        Some(NodeOverlay {
            nodes: component.nodes,
            segments: component.segments,
            connections,
            route: state.drag.as_ref().map(|drag| drag.route.clone()).unwrap_or_default(),
            route_valid: state.drag.as_ref().is_none_or(|drag| drag.valid),
        })
    }

    pub(crate) fn start_node_drag(&mut self, coord: Coord) -> bool {
        if self
            .node_overlay()
            .is_none_or(|overlay| !overlay.nodes.contains(&coord))
        {
            return false;
        }
        let Some(state) = self.node_edit.as_mut() else {
            return false;
        };
        state.drag = Some(NodeDrag {
            start: coord,
            group: EditGroupId::new(),
            owned: HashMap::new(),
            original: HashMap::new(),
            route: vec![coord],
            valid: true,
        });

        true
    }

    pub(crate) fn update_node_drag(&mut self, target: Coord) -> bool {
        let Some(mut state) = self.node_edit.take() else {
            return false;
        };
        let Some((start, transient)) = state
            .drag
            .as_ref()
            .map(|drag| (drag.start, drag.owned.keys().copied().collect::<HashSet<_>>()))
        else {
            self.node_edit = Some(state);

            return false;
        };
        let path = self.state.active_pair().and_then(|(environment, document)| {
            let tree = &environment.bake_program.as_ref()?.tree;
            let path = node::route_with_context(document, tree, &state.group, start, target, &transient)?;
            if state.group.shapes(tree, &state.brush) {
                let original = &state.drag.as_ref()?.original;
                node::oriented_route_directions(document, tree, &state.group, &state.brush, &path, original)?;
            }

            Some(path)
        });
        let valid = path.is_some();
        let changed = self.apply_node_route(&mut state, path.as_deref());
        if let Some(drag) = state.drag.as_mut() {
            drag.route = path.unwrap_or_default();
            drag.valid = valid;
        }
        self.node_edit = Some(state);

        changed
    }

    pub(crate) fn finish_node_drag(&mut self, commit: bool) -> bool {
        let Some(mut state) = self.node_edit.take() else {
            return false;
        };
        let Some(valid) = state.drag.as_ref().map(|drag| drag.valid) else {
            self.node_edit = Some(state);

            return false;
        };
        if !commit || !valid {
            self.apply_node_route(&mut state, None);
        } else if let Some(endpoint) = state.drag.as_ref().and_then(|drag| drag.route.last()).copied() {
            state.manual.insert(endpoint);
        }
        state.drag = None;
        self.node_edit = Some(state);

        commit && valid
    }

    pub(crate) fn node_dragging(&self) -> bool { self.node_edit.as_ref().is_some_and(|state| state.drag.is_some()) }

    pub(crate) fn cancel_node_drag(&mut self) {
        let Some(mut state) = self.node_edit.take() else {
            return;
        };
        if state.drag.is_some() {
            self.apply_node_route(&mut state, None);
            state.drag = None;
        }
        self.node_edit = Some(state);
    }

    pub(crate) fn cancel_node_edit(&mut self) {
        self.cancel_node_drag();
        self.node_edit = None;
    }

    pub(crate) fn delete_node_connection(&mut self, connection: &[Coord]) -> bool {
        if connection.len() < 2 || self.node_dragging() {
            return false;
        }
        let Some(overlay) = self.node_overlay() else {
            return false;
        };
        let Some(selected) = overlay.connections.iter().position(|current| current == connection) else {
            return false;
        };

        let endpoints = [connection[0], *connection.last().unwrap()];
        let mut deleted = connection[1..connection.len() - 1].to_vec();
        let mut other_connection_counts = [0_usize; 2];
        for (endpoint_index, endpoint) in endpoints.iter().copied().enumerate() {
            other_connection_counts[endpoint_index] = overlay
                .connections
                .iter()
                .enumerate()
                .filter(|(index, current)| {
                    *index != selected && (current.first() == Some(&endpoint) || current.last() == Some(&endpoint))
                })
                .count();
            if other_connection_counts[endpoint_index] == 0 {
                deleted.push(endpoint);
            }
        }

        // Connections are derived from occupied cardinally adjacent tiles. When two
        // structural nodes are adjacent and both have other branches, there is no
        // interior placement to remove. Remove the less-connected endpoint so the
        // requested edge is actually severed on a tie, retain the active seed
        if deleted.is_empty() {
            let seed = self.node_edit.as_ref().map(|state| state.seed);
            let endpoint = (0..endpoints.len())
                .min_by_key(|index| {
                    (
                        other_connection_counts[*index],
                        seed == Some(endpoints[*index]),
                        (endpoints[*index].z, endpoints[*index].y, endpoints[*index].x),
                    )
                })
                .unwrap();
            deleted.push(endpoints[endpoint]);
        }
        deleted.sort_unstable_by_key(|coord| (coord.z, coord.y, coord.x));
        deleted.dedup();

        let replacement_seed = endpoints.into_iter().find(|coord| !deleted.contains(coord));
        let changed = self.delete_node_tiles(&deleted, "delete node connection");
        if changed
            && let Some(state) = self.node_edit.as_mut()
            && deleted.contains(&state.seed)
            && let Some(seed) = replacement_seed
        {
            state.seed = seed;
        }

        changed
    }

    pub(crate) fn node_connection_at(&self, coord: Coord) -> Option<node::Connection> {
        let overlay = self.node_overlay()?;

        node::connection_at_tile(&overlay.connections, coord).cloned()
    }

    pub(crate) fn node_connection_from_pick(
        &self, picked: Option<PrefabInstanceId>, pointed: Coord,
    ) -> Option<node::Connection> {
        let state = self.node_edit.as_ref()?;
        let environment = self.state.environment.as_ref()?;
        let tree = &environment.bake_program.as_ref()?.tree;
        let document = self.state.document(state.document)?;
        if let Some(picked) = picked
            && let Some((prefab, location)) = document.prefab_instance(picked)
        {
            if node::instance_matches(document, tree, &state.group, picked) {
                return self.node_connection_at(location.coord);
            }

            let ty = tree.id_of(&prefab.path)?;
            let roots = tree.roots();
            let floor_or_area = roots.turf.is_some_and(|root| tree.is_subtype_of(ty, root))
                || roots.area.is_some_and(|root| tree.is_subtype_of(ty, root));
            if !floor_or_area {
                return None;
            }
        }

        self.node_connection_at(pointed)
    }

    pub(crate) fn delete_standalone_node(&mut self, coord: Coord) -> bool {
        if self.node_dragging() {
            return false;
        }
        let Some(overlay) = self.node_overlay() else {
            return false;
        };
        if !overlay.nodes.contains(&coord) || overlay.segments.iter().any(|(from, to)| *from == coord || *to == coord) {
            return false;
        }

        self.delete_node_tiles(&[coord], "delete standalone node")
    }

    fn delete_node_tiles(&mut self, coords: &[Coord], label: &str) -> bool {
        let Some(group) = self.node_edit.as_ref().map(|state| state.group.clone()) else {
            return false;
        };
        let action = {
            let Some((environment, document)) = self.state.active_pair_mut() else {
                return false;
            };
            let Some(program) = environment.bake_program.as_ref() else {
                return false;
            };
            if coords.iter().any(|coord| !document.allows_edit_at(*coord)) {
                return false;
            }

            let mut edit = Edit::new(label);
            let mut affected = Vec::new();
            for coord in coords.iter().copied() {
                let removed = document
                    .instance_ids_at(coord)
                    .iter()
                    .copied()
                    .filter(|id| node::instance_matches(document, &program.tree, &group, *id))
                    .collect::<HashSet<_>>();
                if removed.is_empty() {
                    continue;
                }

                let mut after = document.placed_tile(coord).unwrap_or_default();
                after.retain(|placed| !removed.contains(&placed.id()));
                edit.change(document, coord, after);
                affected.extend(removed);
            }

            (!edit.is_empty()).then_some(ToolEdit {
                edit,
                selected: None,
                affected,
            })
        };

        action.is_some_and(|action| self.commit(action, None))
    }

    fn apply_node_route(&mut self, state: &mut NodeEditState, path: Option<&[Coord]>) -> bool {
        if self.state.active() != Some(state.document) || self.z() != state.z {
            return false;
        }
        let Some(drag) = state.drag.as_ref() else {
            return false;
        };

        if self
            .state
            .environment
            .as_ref()
            .and_then(|environment| environment.bake_program.as_ref())
            .is_some_and(|program| state.group.shapes(&program.tree, &state.brush))
        {
            return self.apply_oriented_node_route(state, path);
        }

        let edit_group = drag.group;
        let previous_owned = drag.owned.clone();
        let previous_original = drag.original.clone();
        let previous_ids = previous_owned.values().copied().collect::<HashSet<_>>();
        let desired_path = path.unwrap_or_default();

        let desired = {
            let Some((environment, document)) = self.state.active_pair() else {
                return false;
            };
            let Some(program) = environment.bake_program.as_ref() else {
                return false;
            };

            desired_path
                .iter()
                .copied()
                .filter(|coord| {
                    !document.instance_ids_at(*coord).iter().any(|id| {
                        !previous_ids.contains(id) && node::instance_matches(document, &program.tree, &state.group, *id)
                    })
                })
                .collect::<HashSet<_>>()
        };

        let mut next_owned = previous_owned.clone();
        let mut next_original = previous_original.clone();
        let action = {
            let Some((environment, document)) = self.state.active_pair_mut() else {
                return false;
            };
            let mut edit = Edit::new(format!("route {}", state.brush.path));
            let mut affected = Vec::new();

            for (coord, _) in previous_owned.iter().filter(|(coord, _)| !desired.contains(coord)) {
                let Some(before) = document.placed_tile(*coord) else {
                    continue;
                };
                let Some(original) = previous_original.get(coord) else {
                    continue;
                };
                if before.as_slice() != original.as_slice() {
                    affected.extend(before.iter().map(|placed| placed.id()));
                    affected.extend(original.iter().map(|placed| placed.id()));
                    edit.change(document, *coord, original.clone());
                }
                next_owned.remove(coord);
                next_original.remove(coord);
            }

            for coord in desired.iter().filter(|coord| !previous_owned.contains_key(coord)) {
                let Some(original) = document.placed_tile(*coord) else {
                    continue;
                };
                let Some(placement) = Tool::Place.build_edit(&mut ToolContext {
                    document,
                    tree: &environment.tree,
                    prefab: Some(&state.brush),
                    target: None,
                    coord: *coord,
                    anchor: None,
                    fill_mode: FillMode::default(),
                    custom_fill_boundaries: &[],
                }) else {
                    continue;
                };
                let Some(selected) = placement.selected else {
                    continue;
                };
                edit.changes.extend(placement.edit.changes);
                affected.extend(placement.affected);
                next_owned.insert(*coord, selected);
                next_original.insert(*coord, original);
            }

            (!edit.is_empty()).then_some(ToolEdit {
                edit,
                selected: None,
                affected,
            })
        };

        let changed = action.is_some_and(|action| self.commit(action, Some(edit_group)));
        if let Some(drag) = state.drag.as_mut() {
            drag.owned = next_owned;
            drag.original = next_original;
        }

        changed
    }

    fn apply_oriented_node_route(&mut self, state: &mut NodeEditState, path: Option<&[Coord]>) -> bool {
        let Some(drag) = state.drag.as_ref() else {
            return false;
        };
        let previous_owned = drag.owned.clone();
        let previous_original = drag.original.clone();
        let desired_path = path.unwrap_or_default();
        let edit_group = drag.group;
        let mut next_owned = previous_owned.clone();
        let mut next_original = HashMap::new();

        let action = {
            let Some((environment, document)) = self.state.active_pair_mut() else {
                return false;
            };
            let Some(program) = environment.bake_program.as_ref() else {
                return false;
            };
            let tree = &program.tree;
            let Some(directions) = node::oriented_route_directions(
                document,
                tree,
                &state.group,
                &state.brush,
                desired_path,
                &previous_original,
            ) else {
                return false;
            };

            let mut after_tiles = previous_original.clone();
            let desired = desired_path.iter().copied().collect::<HashSet<_>>();
            next_owned.retain(|coord, _| desired.contains(coord));

            for coord in desired_path.iter().copied() {
                let baseline = match previous_original.get(&coord) {
                    Some(tile) => tile.clone(),
                    None => match document.placed_tile(coord) {
                        Some(tile) => tile,
                        None => return false,
                    },
                };

                let occupied = baseline.iter().any(|placed| state.group.matches(tree, placed.prefab()));
                if occupied {
                    after_tiles.entry(coord).or_insert(baseline);
                } else if let Some(id) = previous_owned.get(&coord).copied() {
                    let Some(current) = document.placed_tile(coord) else {
                        return false;
                    };
                    after_tiles.insert(coord, current);
                    next_owned.insert(coord, id);
                } else {
                    let Some(placement) = Tool::Place.build_edit(&mut ToolContext {
                        document,
                        tree: &environment.tree,
                        prefab: Some(&state.brush),
                        target: None,
                        coord,
                        anchor: None,
                        fill_mode: FillMode::default(),
                        custom_fill_boundaries: &[],
                    }) else {
                        return false;
                    };
                    let Some(id) = placement.selected else {
                        return false;
                    };
                    let Some(tile) = placement
                        .edit
                        .changes
                        .into_iter()
                        .find(|change| change.coord == coord)
                        .map(|change| change.after)
                    else {
                        return false;
                    };
                    after_tiles.insert(coord, tile);
                    next_owned.insert(coord, id);
                }
            }

            for (coord, direction) in directions {
                let Some(tile) = after_tiles.get_mut(&coord) else {
                    return false;
                };
                let Some(placed) = tile
                    .iter_mut()
                    .rev()
                    .find(|placed| state.group.shapes(tree, placed.prefab()))
                else {
                    return false;
                };
                let prefab = placed.prefab_mut();
                let inherited = visual::resolve(tree, &Prefab::new(prefab.path.clone())).dir;
                if inherited == direction {
                    prefab.remove_var(&Identifier::from("dir"));
                } else {
                    prefab.set_var(Identifier::from("dir"), Value::Num(direction as f32));
                }
            }

            let mut edit = Edit::new(format!("route {}", state.brush.path));
            let mut affected = Vec::new();
            for (coord, after) in after_tiles {
                let Some(current) = document.placed_tile(coord) else {
                    return false;
                };
                let baseline = previous_original.get(&coord).unwrap_or(&current);
                if &after != baseline {
                    next_original.insert(coord, baseline.clone());
                }
                if after != current {
                    affected.extend(current.iter().map(|placed| placed.id()));
                    affected.extend(after.iter().map(|placed| placed.id()));
                    edit.change(document, coord, after);
                }
            }

            (!edit.is_empty()).then_some(ToolEdit {
                edit,
                selected: None,
                affected,
            })
        };

        let changed = action.is_some_and(|action| self.commit(action, Some(edit_group)));
        if let Some(drag) = state.drag.as_mut() {
            drag.owned = next_owned;
            drag.original = next_original;
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use core::{path::TreePath, types::Value};
    use std::{
        collections::{HashMap, HashSet},
        sync::Arc,
    };

    use dmm::{Coord, Map, Prefab, Size};
    use editor::{document::MapDocument, focus::AreaFocus};

    use crate::session::{
        DocumentCache,
        Session,
        fixtures::{
            node_environment,
            node_map,
            node_session,
            node_tile_has_group,
            oriented_dir,
            oriented_node_map,
            oriented_node_session,
        },
    };

    #[test]
    fn node_candidate_uses_the_visible_pick_with_tile_fallbacks() {
        let coord = Coord::new(1, 1, 1);
        let floor_path = TreePath::parse("/turf/open/floor");
        let area_path = TreePath::parse("/area/station");
        let supply_path = TreePath::parse("/obj/pipe/supply");
        let scrubbers_path = TreePath::parse("/obj/pipe/scrubbers");
        let other_path = TreePath::parse("/obj/not_node");
        let map = node_map(
            1,
            1,
            &[(coord, vec!["/obj/pipe/supply", "/obj/pipe/scrubbers", "/obj/not_node"])],
        );
        let environment = node_environment();
        let document = MapDocument::new(map, 1);
        let candidates = document
            .instance_ids_at(coord)
            .iter()
            .filter_map(|id| {
                let (prefab, _) = document.prefab_instance(*id)?;

                [&floor_path, &area_path, &supply_path, &scrubbers_path, &other_path]
                    .contains(&&prefab.path)
                    .then_some((prefab.path.clone(), *id))
            })
            .collect::<HashMap<_, _>>();
        let floor = candidates[&floor_path];
        let area = candidates[&area_path];
        let supply = candidates[&supply_path];
        let scrubbers = candidates[&scrubbers_path];
        let other = candidates[&other_path];
        let bake = editor::bake::build(&environment, &document).expect("node profile bakes");
        let mut session = Session::new();
        session.state.environment = Some(Arc::new(environment));
        let document_id = session.state.open_document(document);
        session.caches.insert(
            document_id,
            DocumentCache {
                bake: Some(bake),
                ..Default::default()
            },
        );
        session.rebuild_instances(document_id);

        assert_eq!(session.node_candidate_from_pick(Some(supply), coord), Some(supply));
        assert_eq!(
            session.node_candidate_from_pick(Some(scrubbers), coord),
            Some(scrubbers)
        );
        assert_eq!(session.node_candidate_from_pick(Some(floor), coord), Some(scrubbers));
        assert_eq!(session.node_candidate_from_pick(Some(area), coord), Some(scrubbers));
        assert_eq!(session.node_candidate_from_pick(Some(other), coord), None);
        assert_eq!(session.node_candidate_from_pick(None, coord), Some(scrubbers));
    }

    #[test]
    fn node_drag_clones_the_seed_live_and_is_one_undo_step() {
        let environment = node_environment();
        let start = Coord::new(1, 2, 1);
        let middle = Coord::new(3, 2, 1);
        let detour = Coord::new(3, 3, 1);
        let end = Coord::new(5, 2, 1);
        let mut seed = Prefab::new(TreePath::parse("/obj/cable/heavy"));
        seed.set_var("color".into(), Value::Text(String::from("#65aaff")));
        seed.set_var("dir".into(), Value::Num(4.0));
        let mut map = Map::new(Size { x: 5, y: 3, z: 1 });
        for y in 1..=map.size.y {
            for x in 1..=map.size.x {
                let mut tile = vec![
                    Prefab::new(TreePath::parse("/turf/open/floor")),
                    Prefab::new(TreePath::parse("/area/station")),
                ];
                if Coord::new(x, y, 1) == start {
                    tile.push(seed.clone());
                }
                let key = map.intern_tile(tile);
                map.grid[0][(map.size.y - y) as usize][(x - 1) as usize] = key;
            }
        }
        let document = MapDocument::new(map, 1);
        let target = document
            .instance_ids_at(start)
            .iter()
            .copied()
            .find(|instance| {
                document
                    .prefab_instance(*instance)
                    .is_some_and(|(prefab, _)| prefab.path == seed.path)
            })
            .expect("seed cable");
        let bake = editor::bake::build(&environment, &document).expect("node profile bakes");
        assert_eq!(bake.node_groups().len(), 3);

        let mut session = Session::new();
        session.state.environment = Some(Arc::new(environment));
        let document_id = session.state.open_document(document);
        session.caches.insert(
            document_id,
            DocumentCache {
                bake: Some(bake),
                ..Default::default()
            },
        );
        session.rebuild_instances(document_id);

        assert!(session.node_tool_available());
        assert!(session.begin_node_edit(target));
        assert!(session.start_node_drag(start));
        assert!(session.update_node_drag(middle));
        session.cancel_node_drag();
        for x in 2..=3 {
            assert!(
                session
                    .map()
                    .unwrap()
                    .tile_at(Coord::new(x, 2, 1))
                    .unwrap()
                    .iter()
                    .all(|prefab| prefab.path != seed.path),
                "cancelling restores the route at x={x}"
            );
        }
        assert!(!session.undo(), "a cancelled route leaves no history entry");

        assert!(session.start_node_drag(start));
        assert!(session.update_node_drag(detour));
        assert!(session.update_node_drag(end));

        assert!(
            session
                .map()
                .unwrap()
                .tile_at(detour)
                .unwrap()
                .iter()
                .all(|prefab| prefab.path != seed.path),
            "rerouting removes the obsolete preview branch"
        );

        for x in 1..=5 {
            let tile = session.map().unwrap().tile_at(Coord::new(x, 2, 1)).unwrap();
            let routed = tile
                .iter()
                .filter(|prefab| prefab.path == seed.path)
                .collect::<Vec<_>>();
            assert_eq!(routed, vec![&seed], "exactly one cloned seed at x={x}");
        }
        assert!(session.finish_node_drag(true));

        assert!(session.undo());
        for x in 2..=5 {
            assert!(
                session
                    .map()
                    .unwrap()
                    .tile_at(Coord::new(x, 2, 1))
                    .unwrap()
                    .iter()
                    .all(|prefab| prefab.path != seed.path),
                "one undo removes the routed stroke at x={x}"
            );
        }
        assert!(!session.undo());

        assert!(session.redo());
        for x in 1..=5 {
            assert!(
                session
                    .map()
                    .unwrap()
                    .tile_at(Coord::new(x, 2, 1))
                    .unwrap()
                    .iter()
                    .any(|prefab| prefab == &seed),
                "redo restores the exact seed at x={x}"
            );
        }
        assert!(!session.redo());
    }

    #[test]
    fn switching_maps_cancels_a_live_node_route_before_activation() {
        let start = Coord::new(1, 2, 1);
        let middle = Coord::new(2, 2, 1);
        let end = Coord::new(4, 2, 1);
        let first_map = node_map(4, 2, &[(start, vec!["/obj/cable"])]);
        let (mut session, target) = node_session(first_map, start);
        let first = session.state.active().unwrap();

        assert!(session.begin_node_edit(target));
        assert!(session.start_node_drag(start));
        assert!(session.update_node_drag(end));
        assert!(node_tile_has_group(&session, middle));

        let second_map = node_map(4, 2, &[(middle, vec!["/obj/not_node"])]);
        let second_before = second_map.tile_at(middle).cloned().unwrap();
        let second = session.activate_document(MapDocument::new(second_map, 1));

        assert_eq!(session.state.active(), Some(second));
        assert!(session.node_edit.is_none());
        assert_eq!(
            session.state.document(second).unwrap().map.tile_at(middle),
            Some(&second_before)
        );

        let first_document = session.state.document(first).unwrap();
        for x in 2..=4 {
            assert!(
                first_document
                    .map
                    .tile_at(Coord::new(x, 2, 1))
                    .unwrap()
                    .iter()
                    .all(|prefab| prefab.path != TreePath::parse("/obj/cable")),
                "switching maps restores the preview route at x={x}"
            );
        }
        assert_eq!(
            first_document.undo_label(),
            None,
            "the cancelled preview leaves no history entry"
        );
    }

    #[test]
    fn oriented_route_preview_reroute_cancel_and_undo_preserve_directions() {
        let start = Coord::new(1, 2, 1);
        let straight = Coord::new(2, 2, 1);
        let bend = Coord::new(3, 2, 1);
        let end = Coord::new(3, 3, 1);
        let map = oriented_node_map(5, 4, &[(start, "/obj/link/segment", 1)]);
        let (mut session, seed) = oriented_node_session(map, start);
        assert!(session.begin_node_edit(seed));
        assert!(session.start_node_drag(start));
        assert!(session.update_node_drag(end));
        assert_eq!(oriented_dir(&session, start), Some(4));
        assert_eq!(oriented_dir(&session, straight), Some(4));
        assert_eq!(oriented_dir(&session, bend), Some(9));
        assert_eq!(oriented_dir(&session, end), Some(1));

        assert!(session.update_node_drag(Coord::new(4, 2, 1)));
        assert_eq!(oriented_dir(&session, end), None);
        assert_eq!(oriented_dir(&session, bend), Some(4));
        session.cancel_node_drag();
        assert_eq!(oriented_dir(&session, start), Some(1));
        assert_eq!(oriented_dir(&session, straight), None);
        assert!(!session.undo());

        assert!(session.start_node_drag(start));
        assert!(session.update_node_drag(end));
        assert!(session.finish_node_drag(true));
        assert!(session.undo());
        assert_eq!(oriented_dir(&session, start), Some(1));
        assert_eq!(oriented_dir(&session, bend), None);
        assert!(session.redo());
        assert_eq!(oriented_dir(&session, start), Some(4));
        assert_eq!(oriented_dir(&session, bend), Some(9));
    }

    #[test]
    fn oriented_route_reorients_safe_endpoint_and_rejects_three_way_segment() {
        let start = Coord::new(1, 2, 1);
        let middle = Coord::new(2, 2, 1);
        let endpoint = Coord::new(3, 2, 1);
        let north = Coord::new(3, 3, 1);
        let placements = [
            (start, "/obj/link/segment", 4),
            (endpoint, "/obj/link/segment", 1),
            (north, "/obj/link/segment", 1),
        ];
        let (mut session, seed) = oriented_node_session(oriented_node_map(4, 4, &placements), start);
        let original_id = session
            .state
            .active_document()
            .unwrap()
            .instance_ids_at(endpoint)
            .iter()
            .copied()
            .find(|id| {
                session
                    .state
                    .active_document()
                    .unwrap()
                    .prefab_instance(*id)
                    .is_some_and(|(prefab, _)| prefab.path == TreePath::parse("/obj/link/segment"))
            })
            .unwrap();
        assert!(session.begin_node_edit(seed));
        assert!(session.start_node_drag(start));
        assert!(session.update_node_drag(endpoint));
        assert_eq!(oriented_dir(&session, endpoint), Some(9));
        session.update_node_drag(Coord::new(4, 2, 1));
        assert!(!session.node_overlay().unwrap().route_valid);
        assert_eq!(oriented_dir(&session, endpoint), Some(1));
        assert_eq!(oriented_dir(&session, middle), None);
        assert!(session.update_node_drag(endpoint));
        assert!(session.finish_node_drag(true));
        assert!(
            session
                .state
                .active_document()
                .unwrap()
                .prefab_instance(original_id)
                .is_some()
        );
        assert!(session.undo());
        assert_eq!(oriented_dir(&session, endpoint), Some(1));
        assert_eq!(oriented_dir(&session, middle), None);

        let south = Coord::new(3, 1, 1);
        let mut branched = placements.to_vec();
        branched.push((south, "/obj/link/segment", 1));
        let (mut session, seed) = oriented_node_session(oriented_node_map(4, 4, &branched), start);
        assert!(session.begin_node_edit(seed));
        assert!(session.start_node_drag(start));
        assert!(!session.update_node_drag(endpoint));
        assert!(!session.node_overlay().unwrap().route_valid);
        assert_eq!(oriented_dir(&session, middle), None);
        assert_eq!(oriented_dir(&session, endpoint), Some(1));
        assert!(!session.finish_node_drag(true));
        assert!(!session.undo());
    }

    #[test]
    fn node_connection_deletion_prunes_orphaned_endpoints_and_is_one_undo_step() {
        let start = Coord::new(1, 2, 1);
        let end = Coord::new(5, 2, 1);
        let map = node_map(
            5,
            3,
            &[
                (start, vec!["/obj/cable"]),
                (
                    Coord::new(2, 2, 1),
                    vec!["/obj/structure/table", "/obj/cable", "/obj/cable/heavy"],
                ),
                (Coord::new(3, 2, 1), vec!["/obj/cable/heavy"]),
                (Coord::new(4, 2, 1), vec!["/obj/cable"]),
                (end, vec!["/obj/cable/heavy"]),
            ],
        );
        let (mut session, target) = node_session(map, start);

        assert!(session.begin_node_edit(target));
        let overlay = session.node_overlay().unwrap();
        assert_eq!(overlay.nodes, vec![start, end]);
        assert_eq!(overlay.connections.len(), 1);
        let connection = overlay.connections[0].clone();

        assert!(session.delete_node_connection(&connection));
        for x in 1..=5 {
            assert!(
                session
                    .map()
                    .unwrap()
                    .tile_at(Coord::new(x, 2, 1))
                    .unwrap()
                    .iter()
                    .all(|prefab| !prefab.path.to_string().starts_with("/obj/cable")),
                "all node-group placements were removed at x={x}"
            );
        }
        assert!(
            session
                .map()
                .unwrap()
                .tile_at(Coord::new(2, 2, 1))
                .unwrap()
                .iter()
                .any(|prefab| prefab.path == TreePath::parse("/obj/structure/table")),
            "unrelated placements remain"
        );
        assert!(session.node_overlay().is_none());

        assert!(session.undo());
        assert!(!session.undo());
        assert_eq!(session.node_overlay().unwrap().connections, vec![connection.clone()]);
        assert_eq!(
            session
                .map()
                .unwrap()
                .tile_at(Coord::new(2, 2, 1))
                .unwrap()
                .iter()
                .filter(|prefab| prefab.path.to_string().starts_with("/obj/cable"))
                .count(),
            2
        );

        assert!(session.redo());
        assert!(!session.redo());
        assert!(session.node_overlay().is_none());
    }

    #[test]
    fn node_connection_deletion_keeps_endpoints_with_other_connections() {
        let start = Coord::new(1, 2, 1);
        let junction = Coord::new(3, 2, 1);
        let map = node_map(
            5,
            3,
            &[
                (start, vec!["/obj/cable"]),
                (Coord::new(2, 2, 1), vec!["/obj/cable"]),
                (junction, vec!["/obj/cable"]),
                (Coord::new(4, 2, 1), vec!["/obj/cable"]),
                (Coord::new(5, 2, 1), vec!["/obj/cable"]),
                (Coord::new(3, 3, 1), vec!["/obj/cable"]),
            ],
        );
        let (mut session, target) = node_session(map, start);

        assert!(session.begin_node_edit(target));
        let connection = session
            .node_overlay()
            .unwrap()
            .connections
            .into_iter()
            .find(|connection| connection.first() == Some(&start) && connection.last() == Some(&junction))
            .unwrap();
        assert!(session.delete_node_connection(&connection));

        for coord in [start, Coord::new(2, 2, 1)] {
            assert!(
                session
                    .map()
                    .unwrap()
                    .tile_at(coord)
                    .unwrap()
                    .iter()
                    .all(|prefab| !prefab.path.to_string().starts_with("/obj/cable"))
            );
        }
        for coord in [junction, Coord::new(4, 2, 1), Coord::new(5, 2, 1), Coord::new(3, 3, 1)] {
            assert!(
                session
                    .map()
                    .unwrap()
                    .tile_at(coord)
                    .unwrap()
                    .iter()
                    .any(|prefab| prefab.path.to_string().starts_with("/obj/cable"))
            );
        }
        let overlay = session.node_overlay().unwrap();
        assert!(overlay.nodes.contains(&junction));
        assert_eq!(overlay.connections.len(), 2);

        assert!(session.undo());
        assert_eq!(session.node_overlay().unwrap().connections.len(), 3);
    }

    #[test]
    fn adjacent_non_leaf_nodes_do_not_block_connection_deletion() {
        let left = Coord::new(2, 2, 1);
        let right = Coord::new(3, 2, 1);
        let remaining = [
            Coord::new(1, 2, 1),
            Coord::new(2, 3, 1),
            Coord::new(4, 2, 1),
            Coord::new(3, 3, 1),
        ];
        let map = node_map(
            4,
            3,
            &[
                (remaining[0], vec!["/obj/cable"]),
                (left, vec!["/obj/cable"]),
                (remaining[1], vec!["/obj/cable"]),
                (right, vec!["/obj/cable"]),
                (remaining[2], vec!["/obj/cable"]),
                (remaining[3], vec!["/obj/cable"]),
            ],
        );
        let (mut session, target) = node_session(map, left);

        assert!(session.begin_node_edit(target));
        let connection = session
            .node_overlay()
            .unwrap()
            .connections
            .into_iter()
            .find(|connection| connection.as_slice() == [left, right])
            .expect("adjacent structural nodes have a connection with no interior tile");

        assert!(session.delete_node_connection(&connection));
        assert!(
            node_tile_has_group(&session, left),
            "the active seed is retained on a tie"
        );
        assert!(
            !node_tile_has_group(&session, right),
            "one endpoint is removed to sever the inferred edge"
        );
        for coord in remaining {
            assert!(
                node_tile_has_group(&session, coord),
                "other branch placements remain at {coord:?}"
            );
        }

        assert!(session.undo());
        assert!(node_tile_has_group(&session, right));
        assert_eq!(session.node_overlay().unwrap().connections.len(), 6);
    }

    #[test]
    fn node_connection_deletion_removes_a_straight_span_between_two_elbows() {
        let lower = Coord::new(3, 1, 1);
        let middle = Coord::new(3, 2, 1);
        let upper = Coord::new(3, 3, 1);
        let placements = [
            (Coord::new(1, 1, 1), vec!["/obj/cable"]),
            (Coord::new(2, 1, 1), vec!["/obj/cable"]),
            (lower, vec!["/obj/cable"]),
            (middle, vec!["/obj/cable/heavy"]),
            (upper, vec!["/obj/cable"]),
            (Coord::new(4, 3, 1), vec!["/obj/cable"]),
            (Coord::new(5, 3, 1), vec!["/obj/cable"]),
        ];
        let map = node_map(5, 3, &placements);
        let (mut session, target) = node_session(map, lower);

        assert!(session.begin_node_edit(target));
        let overlay = session.node_overlay().unwrap();
        let connection = overlay
            .connections
            .iter()
            .find(|connection| connection.as_slice() == [lower, middle, upper])
            .cloned()
            .expect("the straight span between the elbows is a connection");
        let middle_owner = session
            .state
            .active_document()
            .unwrap()
            .instance_ids_at(middle)
            .iter()
            .copied()
            .find(|id| {
                session
                    .state
                    .active_document()
                    .unwrap()
                    .prefab_instance(*id)
                    .is_some_and(|(prefab, _)| prefab.path.to_string().starts_with("/obj/cable"))
            })
            .unwrap();
        assert_eq!(
            session.node_connection_from_pick(Some(middle_owner), Coord::new(1, 3, 1)),
            Some(connection.clone()),
            "a visibly shifted prefab resolves through its anchor tile"
        );
        assert_eq!(
            session.node_connection_from_pick(None, middle),
            Some(connection.clone()),
            "an empty visibility hit falls back to the pointed tile"
        );

        assert!(session.delete_node_connection(&connection));
        assert!(!node_tile_has_group(&session, middle));
        for coord in [
            Coord::new(1, 1, 1),
            Coord::new(2, 1, 1),
            lower,
            upper,
            Coord::new(4, 3, 1),
            Coord::new(5, 3, 1),
        ] {
            assert!(
                node_tile_has_group(&session, coord),
                "the remaining branches keep {coord:?}"
            );
        }
    }

    #[test]
    fn adjacent_terminal_nodes_are_removed_with_their_connection() {
        let start = Coord::new(1, 1, 1);
        let end = Coord::new(2, 1, 1);
        let map = node_map(2, 1, &[(start, vec!["/obj/cable"]), (end, vec!["/obj/cable/heavy"])]);
        let (mut session, target) = node_session(map, start);

        assert!(session.begin_node_edit(target));
        let connection = session.node_overlay().unwrap().connections[0].clone();
        assert_eq!(connection, vec![start, end]);
        assert!(!session.delete_standalone_node(start));
        assert!(session.delete_node_connection(&connection));
        for coord in [start, end] {
            assert!(
                session
                    .map()
                    .unwrap()
                    .tile_at(coord)
                    .unwrap()
                    .iter()
                    .all(|prefab| !prefab.path.to_string().starts_with("/obj/cable"))
            );
        }
        assert!(session.node_overlay().is_none());

        assert!(session.undo());
        assert!(!session.undo());
        assert_eq!(session.node_overlay().unwrap().connections, vec![connection]);
    }

    #[test]
    fn standalone_node_deletion_removes_the_group_and_is_one_undo_step() {
        let coord = Coord::new(1, 1, 1);
        let map = node_map(
            1,
            1,
            &[(coord, vec!["/obj/structure/table", "/obj/cable", "/obj/cable/heavy"])],
        );
        let (mut session, target) = node_session(map, coord);

        assert!(session.begin_node_edit(target));
        let overlay = session.node_overlay().unwrap();
        assert_eq!(overlay.nodes, vec![coord]);
        assert!(overlay.segments.is_empty());

        assert!(session.delete_standalone_node(coord));
        let tile = session.map().unwrap().tile_at(coord).unwrap();
        assert!(
            tile.iter()
                .all(|prefab| !prefab.path.to_string().starts_with("/obj/cable"))
        );
        assert!(
            tile.iter()
                .any(|prefab| prefab.path == TreePath::parse("/obj/structure/table"))
        );
        assert!(session.node_overlay().is_none());

        assert!(session.undo());
        assert!(!session.undo());
        assert_eq!(
            session
                .map()
                .unwrap()
                .tile_at(coord)
                .unwrap()
                .iter()
                .filter(|prefab| prefab.path.to_string().starts_with("/obj/cable"))
                .count(),
            2
        );
        assert_eq!(session.node_overlay().unwrap().nodes, vec![coord]);

        assert!(session.redo());
        assert!(!session.redo());
        assert!(session.node_overlay().is_none());
    }

    #[test]
    fn node_connection_deletion_is_rejected_whole_outside_the_focus() {
        let start = Coord::new(1, 1, 1);
        let outside = Coord::new(3, 1, 1);
        let end = Coord::new(4, 1, 1);
        let map = node_map(
            4,
            1,
            &[
                (start, vec!["/obj/cable"]),
                (Coord::new(2, 1, 1), vec!["/obj/cable"]),
                (outside, vec!["/obj/cable"]),
                (end, vec!["/obj/cable"]),
            ],
        );
        let (mut session, target) = node_session(map, start);
        let area = session
            .state
            .active_document()
            .unwrap()
            .instance_ids_at(start)
            .iter()
            .copied()
            .find(|id| {
                session
                    .state
                    .active_document()
                    .unwrap()
                    .prefab_instance(*id)
                    .is_some_and(|(prefab, _)| prefab.path == TreePath::parse("/area/station"))
            })
            .unwrap();
        session
            .state
            .active_document_mut()
            .unwrap()
            .set_focus(Some(AreaFocus::new(
                start,
                Prefab::new(TreePath::parse("/area/station")),
                area,
                HashSet::from([start, Coord::new(2, 1, 1), end]),
            )));

        assert!(session.begin_node_edit(target));
        let connection = session.node_overlay().unwrap().connections[0].clone();
        assert!(!session.delete_node_connection(&connection));
        for x in 1..=4 {
            assert!(
                session
                    .map()
                    .unwrap()
                    .tile_at(Coord::new(x, 1, 1))
                    .unwrap()
                    .iter()
                    .any(|prefab| prefab.path.to_string().starts_with("/obj/cable"))
            );
        }
        assert!(!session.undo());
    }

    #[test]
    fn node_drag_merges_a_nearby_group_and_exposes_its_far_endpoint() {
        let environment = node_environment();
        let start = Coord::new(1, 2, 1);
        let endpoint = Coord::new(3, 2, 1);
        let far_endpoint = Coord::new(5, 2, 1);
        let seed = Prefab::new(TreePath::parse("/obj/cable/heavy"));
        let mut map = Map::new(Size { x: 5, y: 3, z: 1 });
        for y in 1..=map.size.y {
            for x in 1..=map.size.x {
                let coord = Coord::new(x, y, 1);
                let mut tile = vec![
                    Prefab::new(TreePath::parse("/turf/open/floor")),
                    Prefab::new(TreePath::parse("/area/station")),
                ];
                if coord == start || matches!(x, 4 | 5) && y == 2 {
                    tile.push(seed.clone());
                }
                let key = map.intern_tile(tile);
                map.grid[0][(map.size.y - y) as usize][(x - 1) as usize] = key;
            }
        }
        let document = MapDocument::new(map, 1);
        let target = document
            .instance_ids_at(start)
            .iter()
            .copied()
            .find(|instance| {
                document
                    .prefab_instance(*instance)
                    .is_some_and(|(prefab, _)| prefab.path == seed.path)
            })
            .expect("seed cable");
        let bake = editor::bake::build(&environment, &document).expect("node profile bakes");
        let mut session = Session::new();
        session.state.environment = Some(Arc::new(environment));
        let document_id = session.state.open_document(document);
        session.caches.insert(
            document_id,
            DocumentCache {
                bake: Some(bake),
                ..Default::default()
            },
        );
        session.rebuild_instances(document_id);

        assert!(session.begin_node_edit(target));
        assert_eq!(session.node_overlay().unwrap().nodes, vec![start]);
        assert!(session.start_node_drag(start));
        assert!(session.update_node_drag(endpoint));

        let live = session.node_overlay().unwrap();
        assert_eq!(
            live.segments.len(),
            4,
            "the routed line merged with the nearby component"
        );
        assert_eq!(live.nodes, vec![start, endpoint, far_endpoint]);

        assert!(session.finish_node_drag(true));
        let committed = session.node_overlay().unwrap();
        assert_eq!(committed.segments.len(), 4);
        assert_eq!(committed.nodes, vec![start, endpoint, far_endpoint]);
    }

    #[test]
    fn node_drag_restores_an_abandoned_blocker_detour() {
        let environment = node_environment();
        let start = Coord::new(3, 2, 1);
        let first_target = Coord::new(5, 2, 1);
        let final_target = Coord::new(5, 1, 1);
        let blocker = Coord::new(4, 2, 1);
        let seed = Prefab::new(TreePath::parse("/obj/cable/heavy"));
        let mut map = Map::new(Size { x: 5, y: 3, z: 1 });
        for y in 1..=map.size.y {
            for x in 1..=map.size.x {
                let coord = Coord::new(x, y, 1);
                let turf = if coord == blocker {
                    "/turf/closed/wall"
                } else {
                    "/turf/open/floor"
                };
                let mut tile = vec![
                    Prefab::new(TreePath::parse(turf)),
                    Prefab::new(TreePath::parse("/area/station")),
                ];
                if coord == start {
                    tile.push(seed.clone());
                }
                let key = map.intern_tile(tile);
                map.grid[0][(map.size.y - y) as usize][(x - 1) as usize] = key;
            }
        }
        let document = MapDocument::new(map, 1);
        let target = document
            .instance_ids_at(start)
            .iter()
            .copied()
            .find(|instance| {
                document
                    .prefab_instance(*instance)
                    .is_some_and(|(prefab, _)| prefab.path == seed.path)
            })
            .expect("seed cable");
        let bake = editor::bake::build(&environment, &document).expect("node profile bakes");
        let mut session = Session::new();
        session.state.environment = Some(Arc::new(environment));
        let document_id = session.state.open_document(document);
        session.caches.insert(
            document_id,
            DocumentCache {
                bake: Some(bake),
                ..Default::default()
            },
        );
        session.rebuild_instances(document_id);

        assert!(session.begin_node_edit(target));
        assert!(session.start_node_drag(start));
        assert!(session.update_node_drag(first_target));
        for coord in [Coord::new(3, 3, 1), Coord::new(4, 3, 1), Coord::new(5, 3, 1)] {
            assert!(
                session
                    .map()
                    .unwrap()
                    .tile_at(coord)
                    .unwrap()
                    .iter()
                    .any(|prefab| prefab.path == seed.path),
                "the initial route uses the upper detour at {coord:?}"
            );
        }

        assert!(session.update_node_drag(final_target));
        for coord in [Coord::new(3, 3, 1), Coord::new(4, 3, 1), Coord::new(5, 3, 1)] {
            assert!(
                session
                    .map()
                    .unwrap()
                    .tile_at(coord)
                    .unwrap()
                    .iter()
                    .all(|prefab| prefab.path != seed.path),
                "the abandoned upper detour was restored at {coord:?}"
            );
        }
        for coord in [Coord::new(3, 1, 1), Coord::new(4, 1, 1), Coord::new(5, 1, 1)] {
            assert!(
                session
                    .map()
                    .unwrap()
                    .tile_at(coord)
                    .unwrap()
                    .iter()
                    .any(|prefab| prefab.path == seed.path),
                "the final route uses the lower detour at {coord:?}"
            );
        }
    }
}
