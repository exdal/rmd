use base::{
    types::{Identifier, Value},
    vars,
};
use std::collections::{HashMap, HashSet};

use dmm::{Coord, Prefab};

use crate::session::Session;
use crate::{
    document::{
        DocumentId, PlacedTile, PrefabInstanceId,
        command::{Edit, EditGroupId},
    },
    frame::visual,
    node::route,
    tool::{FillMode, Tool, ToolContext, ToolEdit},
};

pub(crate) fn registered_types(
    environment: Option<&crate::environment::Environment>, bake: Option<&crate::bake::Bake>,
) -> HashSet<objtree::TypeId> {
    let Some((environment, bake)) = environment.zip(bake) else {
        return HashSet::new();
    };
    let Some(program) = environment.bake_program.as_ref() else {
        return HashSet::new();
    };

    bake.node_groups()
        .iter()
        .filter_map(|group| program.tree.get(group.subtype))
        .filter_map(|decl| environment.tree.id_of(&decl.path))
        .flat_map(|root| environment.tree.descendants(root))
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NodeOverlay {
    pub nodes: Vec<Coord>,
    pub segments: Vec<(Coord, Coord)>,
    pub connections: Vec<route::Connection>,
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
pub(crate) struct NodeEditState {
    pub(crate) document: DocumentId,
    z: u32,
    group: route::ResolvedGroup,
    seed: Coord,
    brush: Prefab,
    manual: HashSet<Coord>,
    drag: Option<NodeDrag>,
}

impl Session {
    pub(crate) fn select_node_from_pick(&mut self, picked: render::PickResult) -> bool {
        let render::PickResult::Hit(owner) = picked else {
            return false;
        };
        self.begin_node_edit(owner)
    }

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
            && let Some(instance) = document.prefab_instance(picked)
        {
            let prefab = instance.prefab();
            if route::group_for_prefab(tree, groups, prefab).is_some() {
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
            let prefab = document.prefab_instance(*instance)?.prefab();

            route::group_for_prefab(tree, groups, prefab)
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
        let Some(instance) = document.prefab_instance(target) else {
            return false;
        };
        let prefab = instance.prefab();
        let location = instance.location;
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
        let Some(group_index) = route::group_for_prefab(&program.tree, groups, prefab) else {
            return false;
        };
        let Some(group) = route::resolve_group(&program.tree, groups, group_index, prefab) else {
            return false;
        };
        let coord = location.coord;
        let brush = prefab.clone();
        let mut manual = HashSet::from([coord]);
        if let Some(previous) = self.node_edit.as_ref()
            && previous.document == document_id
            && previous.z == document.z
            && previous.group == group
            && let Some(component) = route::component(document, &program.tree, &group, coord)
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
        self.pick_instance(Some(target));

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
        let mut component = route::component(document, tree, &state.group, state.seed)?;
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
        let connections = route::connections(&component, &state.manual);

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
            let path = route::route_with_context(document, tree, &state.group, start, target, &transient)?;
            if state.group.shapes(tree, &state.brush) {
                let original = &state.drag.as_ref()?.original;
                route::oriented_route_directions(document, tree, &state.group, &state.brush, &path, original)?;
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

    pub(crate) fn node_dragging(&self) -> bool {
        self.node_edit.as_ref().is_some_and(|state| state.drag.is_some())
    }

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

    pub(crate) fn node_connection_at(&self, coord: Coord) -> Option<route::Connection> {
        let overlay = self.node_overlay()?;

        route::connection_at_tile(&overlay.connections, coord).cloned()
    }

    pub(crate) fn node_connection_from_pick(
        &self, picked: Option<PrefabInstanceId>, pointed: Coord,
    ) -> Option<route::Connection> {
        let state = self.node_edit.as_ref()?;
        let environment = self.state.environment.as_ref()?;
        let tree = &environment.bake_program.as_ref()?.tree;
        let document = self.state.document(state.document)?;
        if let Some(picked) = picked
            && let Some(instance) = document.prefab_instance(picked)
        {
            if route::instance_matches(document, tree, &state.group, picked) {
                return self.node_connection_at(instance.location.coord);
            }

            let ty = tree.id_of(&instance.prefab().path)?;
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
                    .filter(|id| route::instance_matches(document, &program.tree, &group, *id))
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
                picked: None,
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
                        !previous_ids.contains(id)
                            && route::instance_matches(document, &program.tree, &state.group, *id)
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
                    world: &environment.world,
                    prefab: Some(&state.brush),
                    target: None,
                    coord: *coord,
                    anchor: None,
                    fill_mode: FillMode::default(),
                    custom_fill_boundaries: &[],
                }) else {
                    continue;
                };
                let Some(selected) = placement.picked else {
                    continue;
                };
                edit.changes.extend(placement.edit.changes);
                affected.extend(placement.affected);
                next_owned.insert(*coord, selected);
                next_original.insert(*coord, original);
            }

            (!edit.is_empty()).then_some(ToolEdit {
                edit,
                picked: None,
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
            let Some(directions) = route::oriented_route_directions(
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
                        world: &environment.world,
                        prefab: Some(&state.brush),
                        target: None,
                        coord,
                        anchor: None,
                        fill_mode: FillMode::default(),
                        custom_fill_boundaries: &[],
                    }) else {
                        return false;
                    };
                    let Some(id) = placement.picked else {
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
                    prefab.remove_var(&Identifier::from(vars::DIR));
                } else {
                    prefab.set_var(Identifier::from(vars::DIR), Value::Num(direction as f32));
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
                picked: None,
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
mod tests;
