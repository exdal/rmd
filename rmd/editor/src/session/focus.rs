use dmm::Coord;
use editor::{
    document::{DocumentId, PrefabInstanceId},
    focus::AreaFocus,
    frame::{self},
};
use objtree::{Roots, TypeId};

use super::Session;

impl Session {
    pub fn area_at(&self, id: DocumentId, coord: Coord) -> Option<PrefabInstanceId> {
        self.instance_under_root_at(id, coord, |roots| roots.area)
    }

    pub fn turf_at(&self, id: DocumentId, coord: Coord) -> Option<PrefabInstanceId> {
        self.instance_under_root_at(id, coord, |roots| roots.turf)
    }

    fn instance_under_root_at(
        &self, id: DocumentId, coord: Coord, root: impl FnOnce(Roots) -> Option<TypeId>,
    ) -> Option<PrefabInstanceId> {
        let environment = self.state.environment.as_ref()?;
        let root = root(environment.tree.roots())?;
        let document = self.state.document(id)?;
        let tile = document.map.tile_at(coord)?;

        tile.iter()
            .zip(document.instance_ids_at(coord))
            .find_map(|(prefab, owner)| {
                let id = environment.tree.id_of(&prefab.path)?;

                environment.tree.is_subtype_of(id, root).then_some(*owner)
            })
    }

    pub fn focused_area(&self) -> Option<PrefabInstanceId> { Some(self.state.active_document()?.focus()?.component()) }

    pub fn toggle_focus_at(&mut self, coord: Option<Coord>) {
        if self.focused_area().is_some()
            && coord.is_none_or(|coord| {
                self.instances()
                    .and_then(|instances| instances.area_component_at(coord))
                    == self.focused_area()
            })
        {
            self.set_focus(None);

            return;
        }

        let focus = coord.and_then(|seed| self.resolve_focus(seed));
        self.set_focus(focus);
    }

    pub fn can_edit_at(&self, coord: Coord) -> bool {
        self.state
            .active_document()
            .is_none_or(|document| document.allows_edit_at(coord))
    }

    fn set_focus(&mut self, focus: Option<AreaFocus>) {
        if let Some(document) = self.state.active_document_mut() {
            document.set_focus(focus);
        }
    }

    fn resolve_focus(&self, seed: Coord) -> Option<AreaFocus> {
        let owner = self.area_at(self.state.active()?, seed)?;
        let prefab = self.state.active_document()?.prefab_instance(owner)?.0.clone();
        let instances = self.instances()?;
        let component = instances.area_component_at(seed)?;

        Some(AreaFocus::new(
            seed,
            prefab,
            component,
            instances.area_component_tiles(component),
        ))
    }

    pub(super) fn revalidate_focus(&mut self) {
        let Some(seed) = self.state.active_document().and_then(|document| {
            let focus = document.focus()?;

            Some((focus.seed(), focus.prefab().clone()))
        }) else {
            return;
        };

        let resolved = self
            .resolve_focus(seed.0)
            .filter(|focus| frame::same_area(&seed.1, focus.prefab()));
        self.set_focus(resolved);
    }
}

#[cfg(test)]
mod tests {
    use core::path::TreePath;

    use dmm::Coord;
    use editor::tool::Tool;

    use crate::session::fixtures::{area_at, focus_session};

    #[test]
    fn focus_toggles_connected_regions_and_clears_without_an_area() {
        let mut session = focus_session();
        let first = Coord::new(1, 1, 1);
        let same_region = Coord::new(2, 1, 1);
        let different_area = Coord::new(3, 1, 1);
        let disconnected_match = Coord::new(4, 1, 1);

        session.toggle_focus_at(Some(first));
        let first_focus = session.focused_area().unwrap();
        assert!(session.can_edit_at(first));
        assert!(session.can_edit_at(same_region));
        assert!(!session.can_edit_at(different_area));
        assert!(!session.can_edit_at(disconnected_match));

        session.toggle_focus_at(Some(same_region));
        assert_eq!(session.focused_area(), None);

        session.toggle_focus_at(Some(first));
        session.toggle_focus_at(Some(disconnected_match));
        assert_ne!(session.focused_area(), Some(first_focus));
        assert!(session.can_edit_at(disconnected_match));
        assert!(!session.can_edit_at(first));

        session.toggle_focus_at(None);
        assert_eq!(session.focused_area(), None);
        assert!(session.can_edit_at(first));
    }

    #[test]
    fn focus_blocks_direct_placement_outside_the_region() {
        let mut session = focus_session();
        let inside = Coord::new(1, 1, 1);
        let outside = Coord::new(4, 1, 1);
        let table = session
            .tree()
            .unwrap()
            .id_of(&TreePath::parse("/obj/structure/table"))
            .unwrap();
        let outside_before = session.map().unwrap().tile_at(outside).unwrap().clone();

        assert!(session.choose_type(table));
        session.toggle_focus_at(Some(inside));
        assert_eq!(session.place_at(outside, None), None);
        assert_eq!(session.map().unwrap().tile_at(outside), Some(&outside_before));
        assert!(session.place_at(inside, None).is_some());
    }

    #[test]
    fn focus_blocks_every_tool_not_just_placement() {
        let mut session = focus_session();
        let inside = Coord::new(1, 1, 1);
        let outside = Coord::new(4, 1, 1);
        session.toggle_focus_at(Some(inside));
        assert!(session.can_edit_at(inside));
        assert!(!session.can_edit_at(outside));

        let outside_target = session.state.active_document().unwrap().instance_ids_at(outside)[0];
        let inside_target = session.state.active_document().unwrap().instance_ids_at(inside)[0];
        let outside_before = session.map().unwrap().tile_at(outside).unwrap().clone();

        session.set_tool(Tool::Delete);
        assert!(!session.delete_instance(outside_target));
        assert!(session.delete_instance(inside_target));
        assert_eq!(session.map().unwrap().tile_at(outside), Some(&outside_before));
        assert_eq!(
            session
                .state
                .active_document()
                .unwrap()
                .instance_location(inside_target),
            None
        );

        // the gizmo drags the selection, and dragging it out of the region is the same escape
        let neighbor = Coord::new(2, 1, 1);
        let movable = session.state.active_document().unwrap().instance_ids_at(neighbor)[0];
        session.select_instance(Some(movable));
        assert_eq!(
            session.move_selected_instance(outside, "move out", &[], None),
            Some(false)
        );
        assert_eq!(session.selected_location().unwrap().coord, neighbor);
        assert_eq!(session.move_selected_instance(inside, "move in", &[], None), Some(true));
    }

    #[test]
    fn deleting_areas_revalidates_or_clears_the_active_focus() {
        let mut session = focus_session();
        let seed = Coord::new(1, 1, 1);
        let neighbor = Coord::new(2, 1, 1);
        session.toggle_focus_at(Some(seed));
        session.set_tool(Tool::Delete);

        let neighbor_area = area_at(&session, neighbor).unwrap();
        assert!(session.delete_instance(neighbor_area));
        assert!(session.focused_area().is_some());
        assert!(session.can_edit_at(seed));
        assert!(!session.can_edit_at(neighbor));

        let seed_area = area_at(&session, seed).unwrap();
        assert!(session.delete_instance(seed_area));
        assert_eq!(session.focused_area(), None);
    }
}
