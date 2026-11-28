use std::{collections::HashSet, convert::Infallible};

use base::{
    coord::Coord,
    types::{Identifier, Value},
};
use vm::{DEFAULT_VIEW_SIZE, clamp_view_size, view_box, view_span, visible_tiles, world::Position};

use crate::{
    bake::Bake,
    document::{DocumentId, MapDocument},
    environment::Environment,
    frame::visual::resolve_value,
    maploader::preview::MapView,
    session::{DocumentCache, Session, SharedHighlights},
    viewport::highlight::turf_view_marker,
};

#[derive(Debug, Clone)]
pub(crate) struct TurfView {
    pub(crate) origin: Coord,
    pub(crate) visible: HashSet<Coord>,
    pub(crate) highlights: SharedHighlights,
}

impl TurfView {
    pub(crate) fn allows(&self, coord: Coord) -> bool {
        coord.z != self.origin.z || self.visible.contains(&coord)
    }
}

/// `None` while the bake that would answer has not landed, so the caller keeps what it had
fn turf_view_visible(
    environment: &Environment, map: MapView<'_>, bake: Option<&mut Bake>, origin: Coord,
) -> Option<HashSet<Coord>> {
    let Some(program) = environment.bake_program.as_ref() else {
        return Some(static_turf_view(environment, map, origin));
    };

    let position = Position::new(origin.x as i32, origin.y as i32, origin.z as i32);
    match bake?.view_from(&program.tree, &program.module, position) {
        Ok(visible) => Some(
            visible
                .into_iter()
                .map(|[x, y]| Coord::new(x as u32, y as u32, origin.z))
                .collect::<HashSet<_>>(),
        ),
        Err(fault) => {
            log::warn!("view from turf failed: {}", fault.kind);

            Some(HashSet::new())
        },
    }
}

/// Fallback if session has baking disabled (opaqueness is estimated from objtree, might not always
/// be accurate)
fn static_turf_view(environment: &Environment, map: MapView<'_>, origin: Coord) -> HashSet<Coord> {
    let tree = &environment.tree;
    let opacity = &tree.builtins().opacity;
    let turf = tree.roots().turf;
    let map_size = map.size();
    let world_view = tree
        .roots()
        .world
        .and_then(|world| tree.var_inherited(world, &Identifier::from("view")));
    let size = match world_view.map(|var| &var.value) {
        Some(Value::Num(distance)) => view_span(*distance),
        Some(Value::Text(text)) => view_box(text).unwrap_or(DEFAULT_VIEW_SIZE),
        _ => DEFAULT_VIEW_SIZE,
    };
    let size = clamp_view_size(size, (map_size.x, map_size.y));

    let Ok(visible) = visible_tiles((origin.x as i32, origin.y as i32), size, |x, y| {
        let (Ok(x), Ok(y)) = (u32::try_from(x), u32::try_from(y)) else {
            return Ok::<_, Infallible>(None);
        };
        let Some(tile) = map.tile_at(Coord::new(x, y, origin.z)) else {
            return Ok(None);
        };

        let mut is_turf = false;
        let mut is_opaque = false;
        for prefab in tile.iter() {
            let Some(id) = tree.id_of(&prefab.path) else {
                continue;
            };

            is_turf |= turf.is_some_and(|turf| tree.is_subtype_of(id, turf));
            is_opaque |= resolve_value(tree, id, prefab, opacity).is_some_and(|resolved| resolved.value.is_truthy());
        }

        Ok(is_turf.then_some(is_opaque))
    });

    visible
        .into_iter()
        .map(|(x, y)| Coord::new(x as u32, y as u32, origin.z))
        .collect::<HashSet<_>>()
}

pub(crate) fn refresh_turf_view(
    cache: &mut DocumentCache, environment: Option<&Environment>, document: Option<&MapDocument>,
) {
    let (Some(view), Some(environment), Some(document)) = (cache.turf_view.as_mut(), environment, document) else {
        return;
    };

    let map = cache.modules.view(&document.map);
    if let Some(visible) = turf_view_visible(environment, map, cache.bake.as_mut(), view.origin) {
        view.visible = visible;
    }
}

impl Session {
    pub(crate) fn toggle_turf_view(&mut self, id: DocumentId, coord: Coord) {
        let Self { state, caches, .. } = self;
        let Some(cache) = caches.get_mut(&id) else {
            return;
        };

        if cache.turf_view.as_ref().is_some_and(|view| view.origin == coord) {
            cache.turf_view = None;
        } else {
            let environment = state.environment.as_deref();
            let visible = match (environment, state.document(id)) {
                (Some(environment), Some(document)) => {
                    let map = cache.modules.view(&document.map);
                    turf_view_visible(environment, map, cache.bake.as_mut(), coord).unwrap_or_default()
                },
                _ => HashSet::new(),
            };
            cache.turf_view = Some(TurfView {
                origin: coord,
                visible,
                highlights: turf_view_marker(coord),
            });
        }

        self.publish_turf_view(id);
    }

    pub(crate) fn clear_turf_view(&mut self, id: DocumentId) {
        let Some(cache) = self.caches.get_mut(&id) else {
            return;
        };

        cache.turf_view = None;
        self.publish_turf_view(id);
    }

    fn publish_turf_view(&mut self, id: DocumentId) {
        let Some(cache) = self.caches.get_mut(&id) else {
            return;
        };

        let update = cache
            .instances
            .apply_visibility(&self.type_visibility, cache.turf_view.as_ref());
        self.publish_frame_update(id, update);
    }

    pub(crate) fn turf_view_origin(&self, id: DocumentId) -> Option<Coord> {
        self.caches.get(&id)?.turf_view.as_ref().map(|view| view.origin)
    }

    pub(crate) fn can_view_from_turf(&self, id: DocumentId) -> bool {
        let Some(environment) = self.state.environment.as_ref() else {
            return false;
        };

        environment.bake_program.is_none() || self.caches.get(&id).is_some_and(|cache| cache.bake.is_some())
    }
}

#[cfg(test)]
mod tests {
    use base::{
        coord::Coord,
        path::TreePath,
        types::{Identifier, Value},
    };

    use dmm::{Map, Size};
    use prefab::Prefab;

    use crate::{
        document::MapDocument,
        session::{
            Session,
            fixtures::{codebase, examples, settle_bake},
        },
    };

    #[test]
    fn turf_view_hides_atoms_outside_the_view() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).expect("codebase");
        session.open_map(&root.join("test.dmm"), 1).expect("map");
        settle_bake(&mut session);
        let id = session.state.active().unwrap();
        let origin = Coord::new(2, 2, 1);
        let far = Coord::new(8, 3, 1);
        let is_drawn = |session: &Session, coord: Coord| {
            let document = session.state.document(id).unwrap();
            let owners = document.instance_ids_at(coord);
            let instances = &session.caches[&id].instances;
            owners.iter().any(|owner| instances.sprite(*owner).is_some())
        };

        assert!(is_drawn(&session, origin) && is_drawn(&session, far));
        session.toggle_turf_view(id, origin);
        assert!(is_drawn(&session, origin));
        assert!(!is_drawn(&session, far));

        session.toggle_turf_view(id, origin);
        assert!(is_drawn(&session, far));
    }

    #[test]
    fn turf_view_without_a_bake_program_stops_at_opaque_walls() {
        let mut session = Session::new();
        session.apply_codebase(codebase(&["/turf/wall"]));
        let mut wall = Prefab::new(TreePath::parse("/turf/wall"));
        wall.set_var(Identifier::from("opacity"), Value::Num(1.0));
        let floor = Prefab::new(TreePath::parse("/turf/floor"));
        let thing = Prefab::new(TreePath::parse("/obj/known"));
        let mut map = Map::new(Size { x: 7, y: 3, z: 1 });
        let columns = [
            vec![floor.clone()],
            vec![floor.clone()],
            vec![floor.clone(), thing.clone()],
            vec![wall],
            vec![floor.clone()],
            vec![floor.clone(), thing],
            vec![floor],
        ]
        .map(|tile| map.intern_tile(tile));
        for row in &mut map.grid[0] {
            row.copy_from_slice(&columns);
        }
        let id = session.activate_document(MapDocument::new(map, 1));
        let is_visible = |session: &Session, x: u32| {
            session.caches[&id]
                .turf_view
                .as_ref()
                .is_some_and(|view| view.allows(Coord::new(x, 2, 1)))
        };

        assert!(session.can_view_from_turf(id));
        session.toggle_turf_view(id, Coord::new(2, 2, 1));
        assert!(is_visible(&session, 3));
        assert!(is_visible(&session, 4), "the wall itself is seen");
        assert!(!is_visible(&session, 6), "the room behind the wall is hidden");
    }
}
