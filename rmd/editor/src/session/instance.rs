use core::path::TreePath;

use dmm::{Coord, Prefab};
use editor::{
    command::EditGroupId,
    document::{DocumentId, MapDocument, PrefabInstanceId, PrefabLocation, VarMutation},
    tool::{PlacementKind, Tool, placement_kind},
    visual,
};
use render::SpriteInstance;

use super::{DirectionalTypes, Session, direction_state};

#[derive(Debug)]
pub(super) struct IdenticalCache {
    document: DocumentId,
    generation: u64,
    focus: Option<PrefabInstanceId>,
    prefab: Prefab,
    instances: Vec<PrefabInstanceId>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum EditScope {
    #[default]
    Selected,
    Identical,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PickedTransform {
    pub picked: PrefabInstanceId,
    pub sprite: SpriteInstance,
    pub pixel: [i32; 2],
    pub step: [i32; 2],
    pub is_movable: bool,
    pub dir: u32,
    pub dmi_directions: Option<u32>,
    pub declared: Option<[bool; 8]>,
    pub directional_types: Option<DirectionalTypes>,
}

impl Session {
    pub fn picked_instance(&self) -> Option<PrefabInstanceId> {
        self.state.active_document().and_then(MapDocument::picked_instance)
    }

    pub fn picked_prefab(&self) -> Option<&Prefab> {
        let active = self.state.active()?;
        let picked = self.state.active_document()?.picked_instance()?;

        self.effective_grid(active)?
            .prefab_instance(picked)
            .map(|instance| instance.prefab())
    }

    pub(crate) fn picked_placement_kind(&self) -> Option<PlacementKind> {
        let environment = self.state.environment.as_ref()?;

        placement_kind(&environment.tree, self.picked_prefab()?)
    }

    pub fn picked_location(&self) -> Option<PrefabLocation> {
        let active = self.state.active()?;
        let picked = self.state.active_document()?.picked_instance()?;

        self.effective_grid(active)?.location(picked)
    }

    pub(crate) fn gizmo_transform(&self) -> Option<PickedTransform> {
        if self.tool() != Tool::Pick {
            return None;
        }

        self.picked_transform()
            .filter(|transform| transform.sprite.z == self.z())
    }

    pub(crate) fn picked_transform(&self) -> Option<PickedTransform> {
        let environment = self.state.environment.as_ref()?;
        let document = self.state.active_document()?;
        let picked = document.picked_instance()?;
        let prefab = document.prefab_instance(picked)?.prefab();
        let id = environment.tree.id_of(&prefab.path)?;
        let atom = environment.tree.roots().atom?;
        if !environment.tree.is_subtype_of(id, atom) {
            return None;
        }

        let appearance = visual::resolve_id(&environment.tree, id, prefab);
        let is_movable = environment
            .tree
            .roots()
            .movable
            .is_some_and(|movable| environment.tree.is_subtype_of(id, movable));
        let direction = direction_state(environment, id, &appearance, self.declared_directions(&prefab.path));

        Some(PickedTransform {
            picked,
            sprite: *self.instances()?.sprite(picked)?,
            pixel: [appearance.pixel_x, appearance.pixel_y],
            step: [appearance.step_x, appearance.step_y],
            is_movable,
            dir: direction.dir,
            dmi_directions: direction.dmi_directions,
            declared: direction.declared,
            directional_types: direction.directional_types,
        })
    }

    pub fn pick_instance(&mut self, picked: Option<PrefabInstanceId>) {
        let prefab = self.state.active().and_then(|active| {
            let picked = picked?;

            self.effective_grid(active)?
                .prefab_instance(picked)
                .map(|instance| instance.prefab().clone())
        });
        if let Some(document) = self.state.active_document_mut() {
            document.pick_instance(picked);
        }
        if let Some(prefab) = prefab {
            self.state.choose_prefab(prefab);
        }
    }

    #[cfg(test)]
    pub fn set_picked_instance_var(
        &mut self, name: core::types::Identifier, value: core::types::Value,
    ) -> Option<bool> {
        let label = format!("set {name}");

        self.edit_picked_instance_vars(label, &[VarMutation::Set(name, value)], None)
    }

    pub fn edit_picked_instance_vars(
        &mut self, label: impl Into<String>, mutations: &[VarMutation], group: Option<EditGroupId>,
    ) -> Option<bool> {
        self.edit_instance_vars_in(EditScope::Selected, label, mutations, group)
    }

    pub(crate) fn edit_instance_vars_in(
        &mut self, scope: EditScope, label: impl Into<String>, mutations: &[VarMutation], group: Option<EditGroupId>,
    ) -> Option<bool> {
        self.edit_instances_in(scope, label, None, mutations, group)
    }

    pub(super) fn edit_instances_in(
        &mut self, scope: EditScope, label: impl Into<String>, path: Option<&TreePath>, mutations: &[VarMutation],
        group: Option<EditGroupId>,
    ) -> Option<bool> {
        let targets = self.scope_targets(scope)?;
        let mut label = label.into();
        if targets.len() > 1 {
            label = format!("{label} on {} instances", targets.len());
        }
        let document = self.state.active_document_mut()?;

        let changed = document.edit_instances(&targets, label, path, mutations, group)?;

        if changed {
            self.update_instances(&targets);
        }

        Some(changed)
    }

    fn scope_targets(&mut self, scope: EditScope) -> Option<Vec<PrefabInstanceId>> {
        let picked = self.picked_instance()?;
        let mut targets = match scope {
            EditScope::Selected => Vec::new(),
            EditScope::Identical => {
                self.refresh_identical();
                self.identical().to_vec()
            },
        };
        if !targets.contains(&picked) {
            targets.push(picked);
        }

        Some(targets)
    }

    /// Placements identical to the selected one that an identical scope edit would change, as of the last
    /// [`Self::refresh_identical`]
    pub(crate) fn identical(&self) -> &[PrefabInstanceId] {
        self.identical.as_ref().map_or(&[], |cache| &cache.instances)
    }

    /// Map pixel bounds as `[left, bottom, width, height]` of the identical placements on the current level,
    /// besides the selected one
    pub(crate) fn identical_bounds(&self) -> Vec<[f32; 4]> {
        let Some(document) = self.state.active_document() else {
            return Vec::new();
        };
        let picked = document.picked_instance();
        let tile = self.options.tile_size.max(1) as f32;
        let instances = self.instances();

        self.identical()
            .iter()
            .filter(|id| Some(**id) != picked)
            .filter_map(|id| {
                let location = document.instance_location(*id)?;
                if location.coord.z != document.z {
                    return None;
                }

                Some(match instances.and_then(|instances| instances.sprite(*id)) {
                    Some(sprite) => [sprite.x, sprite.y, sprite.width, sprite.height],
                    None => [
                        (location.coord.x - 1) as f32 * tile,
                        (location.coord.y - 1) as f32 * tile,
                        tile,
                        tile,
                    ],
                })
            })
            .collect()
    }

    pub(crate) fn refresh_identical(&mut self) {
        let Some(document) = self.state.active_document() else {
            self.identical = None;
            return;
        };
        let Some(instance) = document
            .picked_instance()
            .and_then(|picked| document.prefab_instance(picked))
        else {
            self.identical = None;
            return;
        };
        let prefab = instance.prefab();
        let stale = self.identical.as_ref().is_none_or(|cache| {
            cache.document != document.id()
                || cache.generation != document.generation()
                || cache.focus != document.focus().map(|focus| focus.component())
                || cache.prefab != *prefab
        });
        if stale {
            let instances = document
                .identical_instances(prefab)
                .into_iter()
                .filter(|id| document.allows_edit_instance(*id))
                .collect();
            self.identical = Some(IdenticalCache {
                document: document.id(),
                generation: document.generation(),
                focus: document.focus().map(|focus| focus.component()),
                prefab: prefab.clone(),
                instances,
            });
        }
    }

    pub fn move_picked_instance(
        &mut self, to_coord: Coord, label: impl Into<String>, mutations: &[VarMutation], group: Option<EditGroupId>,
    ) -> Option<bool> {
        let picked = self.picked_instance()?;
        let (environment, document) = self.state.active_pair_mut()?;

        let changed = document.move_instance(&environment.tree, picked, to_coord, label, mutations, group)?;

        if changed {
            self.update_instance(picked);
        }

        Some(changed)
    }

    pub fn picked_instance_of(&self, id: DocumentId) -> Option<PrefabInstanceId> {
        self.state.document(id).and_then(MapDocument::picked_instance)
    }
}

#[cfg(test)]
mod tests {
    use core::{path::TreePath, types::Value};

    use dmm::{Coord, Prefab};
    use editor::{
        document::{PrefabInstanceId, VarMutation},
        tool::Tool,
    };

    use super::EditScope;
    use crate::session::{
        Session,
        fixtures::{assert_render_cache_matches_rebuild, examples, flat_session, focus_session},
    };

    fn place(session: &mut Session, placements: &[(u32, &Prefab)]) -> Vec<PrefabInstanceId> {
        session.set_tool(Tool::Place);
        placements
            .iter()
            .map(|(x, prefab)| {
                session.state.choose_prefab((*prefab).clone());
                session.place_at(Coord::new(*x, 1, 1), None).unwrap()
            })
            .collect()
    }

    fn names(session: &Session, ids: &[PrefabInstanceId]) -> Vec<Option<Value>> {
        let document = session.state.active_document().unwrap();

        ids.iter()
            .filter_map(|id| document.prefab_instance(*id))
            .map(|instance| instance.prefab().var(&"name".into()).cloned())
            .collect()
    }

    fn set_name(name: &str) -> [VarMutation; 1] {
        [VarMutation::Set("name".into(), Value::Text(name.to_string()))]
    }

    #[test]
    fn an_identical_scope_edit_changes_every_matching_placement_in_one_undo() {
        let mut session = flat_session(4, 1);
        let table = Prefab::new(TreePath::parse("/obj/structure/table"));
        let mut other = table.clone();
        other.set_var("name".into(), Value::Text(String::from("Other")));
        let ids = place(&mut session, &[(1, &table), (2, &table), (3, &other), (4, &table)]);
        session.pick_instance(Some(ids[0]));
        session.refresh_identical();
        assert_eq!(session.identical().len(), 3);

        assert_eq!(
            session.edit_instance_vars_in(EditScope::Identical, "set name", &set_name("Oak"), None),
            Some(true)
        );

        let oak = Some(Value::Text(String::from("Oak")));
        let other_name = Some(Value::Text(String::from("Other")));
        assert_eq!(
            names(&session, &ids),
            [oak.clone(), oak.clone(), other_name.clone(), oak]
        );
        assert_eq!(
            session.state.active_document().unwrap().undo_label(),
            Some("set name on 3 instances")
        );
        assert_render_cache_matches_rebuild(&session);

        session.undo();
        assert_eq!(names(&session, &ids), [None, None, other_name, None]);
    }

    #[test]
    fn identical_placements_refresh_as_the_map_and_focus_change() {
        let mut session = focus_session();
        let table = Prefab::new(TreePath::parse("/obj/structure/table"));
        let ids = place(&mut session, &[(1, &table), (4, &table)]);
        session.pick_instance(Some(ids[0]));
        session.refresh_identical();
        assert_eq!(session.identical(), [ids[0], ids[1]]);

        let added = place(&mut session, &[(2, &table)]);
        session.pick_instance(Some(ids[0]));
        session.refresh_identical();
        assert_eq!(session.identical().len(), 3, "a new placement joins");

        session.toggle_focus_at(Some(Coord::new(1, 1, 1)));
        session.refresh_identical();
        assert_eq!(
            session.identical(),
            [ids[0], added[0]],
            "focus leaves out what it protects"
        );

        session.edit_instance_vars_in(EditScope::Selected, "set name", &set_name("Oak"), None);
        session.refresh_identical();
        assert_eq!(
            session.identical(),
            [ids[0]],
            "the edited one no longer matches the others"
        );
    }

    #[test]
    fn identical_bounds_cover_the_other_placements_on_the_current_level() {
        let mut session = flat_session(3, 1);
        let table = Prefab::new(TreePath::parse("/obj/structure/table"));
        let ids = place(&mut session, &[(1, &table), (3, &table)]);
        session.pick_instance(Some(ids[0]));
        session.refresh_identical();

        let tile = session.options.tile_size as f32;
        assert_eq!(session.identical_bounds(), [[tile * 2.0, 0.0, tile, tile]]);
    }

    #[test]
    fn a_selected_scope_edit_leaves_identical_placements_alone() {
        let mut session = flat_session(2, 1);
        let table = Prefab::new(TreePath::parse("/obj/structure/table"));
        let ids = place(&mut session, &[(1, &table), (2, &table)]);
        session.pick_instance(Some(ids[0]));

        session.edit_instance_vars_in(EditScope::Selected, "set name", &set_name("Oak"), None);

        assert_eq!(names(&session, &ids), [Some(Value::Text(String::from("Oak"))), None]);
    }

    #[test]
    fn an_identical_scope_edit_skips_placements_outside_the_focused_area() {
        let mut session = focus_session();
        let table = Prefab::new(TreePath::parse("/obj/structure/table"));
        let ids = place(&mut session, &[(1, &table), (2, &table), (4, &table)]);
        session.toggle_focus_at(Some(Coord::new(1, 1, 1)));
        session.pick_instance(Some(ids[0]));

        assert_eq!(
            session.edit_instance_vars_in(EditScope::Identical, "set name", &set_name("Oak"), None),
            Some(true)
        );

        let oak = Some(Value::Text(String::from("Oak")));
        assert_eq!(names(&session, &ids), [oak.clone(), oak, None]);
    }

    #[test]
    fn the_gizmo_only_shows_for_the_pick_tool() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).unwrap();
        session.open_map(&root.join("test.dmm"), 1).unwrap();
        let picked = session
            .state
            .active_document()
            .and_then(|document| document.instance_ids_at(Coord::new(6, 3, 1)).first())
            .copied()
            .unwrap();
        session.pick_instance(Some(picked));
        assert_eq!(session.gizmo_transform(), session.picked_transform());

        for tool in [Tool::Place, Tool::Delete, Tool::Fill] {
            session.set_tool(tool);
            assert_eq!(session.gizmo_transform(), None, "{tool:?}");
        }

        session.set_tool(Tool::Pick);
        assert!(session.gizmo_transform().is_some());
    }

    #[test]
    fn reanchoring_the_selected_instance_keeps_its_rendered_position() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).unwrap();
        session.open_map(&root.join("test.dmm"), 1).unwrap();
        let coord = Coord::new(6, 3, 1);
        let picked = session
            .state
            .active_document()
            .and_then(|document| document.instance_ids_at(coord).first())
            .copied()
            .unwrap();
        session.pick_instance(Some(picked));
        let before = session.picked_transform().unwrap();
        let destination = Coord::new(coord.x + 1, coord.y, coord.z);

        let moved = session.move_picked_instance(
            destination,
            "re-anchor object",
            &[editor::document::VarMutation::Set(
                "pixel_x".into(),
                Value::Num((before.pixel[0] - 32) as f32),
            )],
            None,
        );

        assert_eq!(moved, Some(true));
        assert_eq!(session.picked_location().unwrap().coord, destination);
        let after = session.picked_transform().unwrap();
        assert_eq!(after.pixel, [before.pixel[0] - 32, before.pixel[1]]);
        assert_eq!([after.sprite.x, after.sprite.y], [before.sprite.x, before.sprite.y]);
    }

    #[test]
    fn editing_the_selected_prefab_updates_only_its_cached_sprite() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).unwrap();
        session.open_map(&root.join("test.dmm"), 1).unwrap();
        let coord = Coord::new(6, 3, 1);
        let picked = session
            .state
            .active_document()
            .and_then(|document| document.instance_ids_at(coord).first())
            .copied()
            .unwrap();
        assert_eq!(session.picked_prefab(), None);
        session.pick_instance(Some(picked));
        let transform = session.picked_transform().unwrap();
        assert_eq!(transform.picked, picked);
        assert!(transform.is_movable);
        assert_eq!(transform.pixel, [0, 0]);
        assert_eq!(transform.step, [0, 0]);
        assert_eq!(transform.dir, 2);
        assert_eq!(transform.dmi_directions, Some(1));
        assert_eq!(transform.directional_types, None);
        assert_eq!(transform.sprite.owner, picked);
        let revision = session.revision();
        let before = session.instances().unwrap().sprites.clone();

        assert_eq!(
            session.set_picked_instance_var("pixel_x".into(), Value::Num(7.0)),
            Some(true),
        );
        assert_eq!(session.revision(), revision.wrapping_add(1));
        let update = session.frame_update().unwrap();
        assert_eq!(update.previous_revision, revision);
        let [sprites] = update.sprites[..] else {
            panic!("one changed span: {:?}", update.sprites);
        };
        assert!(
            session.instances().unwrap().sprites[sprites.start..sprites.end]
                .iter()
                .any(|sprite| sprite.owner == picked)
        );
        assert_eq!(session.picked_transform().unwrap().pixel, [7, 0]);
        assert_eq!(
            session
                .instances()
                .unwrap()
                .sprites
                .iter()
                .zip(before)
                .filter(|(after, before)| *after != before)
                .count(),
            1,
        );

        let rendered_revision = session.revision();
        assert_eq!(
            session.set_picked_instance_var("name".into(), Value::Text("edited".into())),
            Some(true),
        );
        assert_eq!(session.revision(), rendered_revision);
        assert_eq!(
            session.picked_prefab().and_then(|prefab| prefab.var(&"name".into())),
            Some(&Value::Text("edited".into())),
        );
    }

    #[test]
    fn editing_a_hidden_type_keeps_it_hidden_and_showing_it_uses_the_latest_appearance() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).unwrap();
        session.open_map(&root.join("test.dmm"), 1).unwrap();
        let table = session
            .tree()
            .unwrap()
            .id_of(&TreePath::parse("/obj/structure/table"))
            .unwrap();
        let light = session
            .tree()
            .unwrap()
            .id_of(&TreePath::parse("/obj/machinery/light"))
            .unwrap();
        let coord = Coord::new(6, 3, 1);
        let picked = session.state.active_document().unwrap().instance_ids_at(coord)[0];
        session.pick_instance(Some(picked));
        let light_texture = session.type_thumbnail(light).unwrap().texture;
        let before_revision = session.revision();

        assert!(session.toggle_type_visibility(table));
        assert!(!session.is_type_visible(table));
        assert_eq!(session.revision(), before_revision.wrapping_add(1));
        assert!(session.instances().unwrap().sprite(picked).is_none());
        assert_eq!(
            session.set_picked_instance_var("icon_state".into(), Value::Text(String::from("light"))),
            Some(true),
        );
        assert!(session.instances().unwrap().sprite(picked).is_none());

        assert!(session.toggle_type_visibility(table));
        assert!(session.is_type_visible(table));
        assert_eq!(
            session.instances().unwrap().sprite(picked).unwrap().texture,
            light_texture
        );
        assert_render_cache_matches_rebuild(&session);
    }

    fn tables_north_and_south() -> (Session, PrefabInstanceId, Prefab) {
        let mut session = flat_session(1, 2);
        let table = Prefab::new(TreePath::parse("/obj/structure/table"));
        session.set_tool(Tool::Place);
        session.state.choose_prefab(table.clone());
        let north = session.place_at(Coord::new(1, 2, 1), None).unwrap();
        session.place_at(Coord::new(1, 1, 1), None).unwrap();

        (session, north, table)
    }

    #[test]
    fn a_moved_placement_draws_in_the_order_of_its_new_tile() {
        let (mut session, north, _) = tables_north_and_south();
        session.pick_instance(Some(north));

        assert_eq!(
            session.move_picked_instance(Coord::new(1, 1, 1), "move", &[], None),
            Some(true)
        );

        assert_render_cache_matches_rebuild(&session);
    }

    #[test]
    fn a_peers_nudge_draws_in_the_order_of_a_clean_build() {
        let (mut session, _, table) = tables_north_and_south();
        let id = session.state.active().unwrap();
        let north = Coord::new(1, 2, 1);
        let mut north_tile = session.map().unwrap().tile_at(north).unwrap().clone();
        // its instance comes back new, and must still draw under the south table
        let nudged = north_tile.iter_mut().find(|prefab| **prefab == table).unwrap();
        nudged.set_var("pixel_y".into(), Value::Num(-16.0));

        assert!(session.apply_remote_tiles(id, vec![(north, north_tile)]));

        assert_render_cache_matches_rebuild(&session);
    }
}
