use core::types::{Identifier, Value};

use dmm::{Coord, Prefab};
use editor::{
    command::EditGroupId,
    document::{DocumentId, MapDocument, PrefabInstanceId, PrefabLocation, VarMutation},
    tool::Tool,
    visual,
};
use render::SpriteInstance;

use super::{DirectionalTypes, Session, direction_state};

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SelectedTransform {
    pub selected: PrefabInstanceId,
    pub sprite: SpriteInstance,
    pub pixel: [i32; 2],
    pub step: [i32; 2],
    pub is_movable: bool,
    pub dir: u32,
    pub dmi_directions: Option<u32>,
    pub directional_types: Option<DirectionalTypes>,
}

impl Session {
    pub fn selected_instance(&self) -> Option<PrefabInstanceId> {
        self.state.active_document().and_then(MapDocument::selected_instance)
    }

    pub fn selected_prefab(&self) -> Option<&Prefab> {
        let document = self.state.active_document()?;
        let selected = document.selected_instance()?;

        document.prefab_instance(selected).map(|(prefab, _)| prefab)
    }

    pub fn selected_location(&self) -> Option<PrefabLocation> {
        let document = self.state.active_document()?;

        document.instance_location(document.selected_instance()?)
    }

    pub(crate) fn gizmo_transform(&self) -> Option<SelectedTransform> {
        if self.tool() != Tool::Select {
            return None;
        }

        self.selected_transform()
            .filter(|transform| transform.sprite.z == self.z())
    }

    pub(crate) fn selected_transform(&self) -> Option<SelectedTransform> {
        let environment = self.state.environment.as_ref()?;
        let document = self.state.active_document()?;
        let selected = document.selected_instance()?;
        let (prefab, _) = document.prefab_instance(selected)?;
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
        let direction = direction_state(environment, id, &appearance);

        Some(SelectedTransform {
            selected,
            sprite: *self.instances()?.sprite(selected)?,
            pixel: [appearance.pixel_x, appearance.pixel_y],
            step: [appearance.step_x, appearance.step_y],
            is_movable,
            dir: direction.dir,
            dmi_directions: direction.dmi_directions,
            directional_types: direction.directional_types,
        })
    }

    pub fn select_instance(&mut self, selected: Option<PrefabInstanceId>) {
        let prefab = self.state.active_document().and_then(|document| {
            let selected = selected?;

            document.prefab_instance(selected).map(|(prefab, _)| prefab.clone())
        });
        if let Some(document) = self.state.active_document_mut() {
            document.select_instance(selected);
        }
        if let Some(prefab) = prefab {
            self.state.choose_prefab(prefab);
        }
    }

    pub fn set_selected_instance_var(&mut self, name: Identifier, value: Value) -> Option<bool> {
        let label = format!("set {name}");

        self.edit_selected_instance_vars(label, &[VarMutation::Set(name, value)], None)
    }

    pub fn edit_selected_instance_vars(
        &mut self, label: impl Into<String>, mutations: &[VarMutation], group: Option<EditGroupId>,
    ) -> Option<bool> {
        let selected = self.selected_instance()?;
        let document = self.state.active_document_mut()?;

        let changed = document.edit_instance_vars(selected, label, mutations, group)?;

        if changed {
            self.update_instance(selected);
        }

        Some(changed)
    }

    pub fn move_selected_instance(
        &mut self, to_coord: Coord, label: impl Into<String>, mutations: &[VarMutation], group: Option<EditGroupId>,
    ) -> Option<bool> {
        let selected = self.selected_instance()?;
        let document = self.state.active_document_mut()?;

        let changed = document.move_instance(selected, to_coord, label, mutations, group)?;

        if changed {
            self.update_instance(selected);
        }

        Some(changed)
    }

    pub fn selected_instance_of(&self, id: DocumentId) -> Option<PrefabInstanceId> {
        self.state.document(id).and_then(MapDocument::selected_instance)
    }
}

#[cfg(test)]
mod tests {
    use core::{path::TreePath, types::Value};

    use dmm::Coord;
    use editor::tool::Tool;

    use crate::session::{
        Session,
        fixtures::{assert_render_cache_matches_rebuild, examples},
    };

    #[test]
    fn the_gizmo_only_shows_for_the_select_tool() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).unwrap();
        session.open_map(&root.join("test.dmm"), 1).unwrap();
        let selected = session
            .state
            .active_document()
            .and_then(|document| document.instance_ids_at(Coord::new(6, 3, 1)).first())
            .copied()
            .unwrap();
        session.select_instance(Some(selected));
        assert_eq!(session.gizmo_transform(), session.selected_transform());

        for tool in [Tool::Place, Tool::Delete, Tool::Fill] {
            session.set_tool(tool);
            assert_eq!(session.gizmo_transform(), None, "{tool:?}");
        }

        session.set_tool(Tool::Select);
        assert!(session.gizmo_transform().is_some());
    }

    #[test]
    fn reanchoring_the_selected_instance_keeps_its_rendered_position() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).unwrap();
        session.open_map(&root.join("test.dmm"), 1).unwrap();
        let coord = Coord::new(6, 3, 1);
        let selected = session
            .state
            .active_document()
            .and_then(|document| document.instance_ids_at(coord).first())
            .copied()
            .unwrap();
        session.select_instance(Some(selected));
        let before = session.selected_transform().unwrap();
        let destination = Coord::new(coord.x + 1, coord.y, coord.z);

        let moved = session.move_selected_instance(
            destination,
            "re-anchor object",
            &[editor::document::VarMutation::Set(
                "pixel_x".into(),
                Value::Num((before.pixel[0] - 32) as f32),
            )],
            None,
        );

        assert_eq!(moved, Some(true));
        assert_eq!(session.selected_location().unwrap().coord, destination);
        let after = session.selected_transform().unwrap();
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
        let selected = session
            .state
            .active_document()
            .and_then(|document| document.instance_ids_at(coord).first())
            .copied()
            .unwrap();
        assert_eq!(session.selected_prefab(), None);
        session.select_instance(Some(selected));
        let transform = session.selected_transform().unwrap();
        assert_eq!(transform.selected, selected);
        assert!(transform.is_movable);
        assert_eq!(transform.pixel, [0, 0]);
        assert_eq!(transform.step, [0, 0]);
        assert_eq!(transform.dir, 2);
        assert_eq!(transform.dmi_directions, Some(1));
        assert_eq!(transform.directional_types, None);
        assert_eq!(transform.sprite.owner, selected);
        let revision = session.revision();
        let before = session.instances().unwrap().sprites.clone();

        assert_eq!(
            session.set_selected_instance_var("pixel_x".into(), Value::Num(7.0)),
            Some(true),
        );
        assert_eq!(session.revision(), revision.wrapping_add(1));
        let update = session.frame_update().unwrap();
        assert_eq!(update.previous_revision, revision);
        let sprites = update.sprites.unwrap();
        assert_eq!(sprites.end, sprites.start + 1);
        assert_eq!(session.instances().unwrap().sprites[sprites.start].owner, selected);
        assert_eq!(session.selected_transform().unwrap().pixel, [7, 0]);
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
            session.set_selected_instance_var("name".into(), Value::Text("edited".into())),
            Some(true),
        );
        assert_eq!(session.revision(), rendered_revision);
        assert_eq!(
            session.selected_prefab().and_then(|prefab| prefab.var(&"name".into())),
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
        let selected = session.state.active_document().unwrap().instance_ids_at(coord)[0];
        session.select_instance(Some(selected));
        let light_texture = session.type_thumbnail(light).unwrap().texture;
        let before_revision = session.revision();

        assert!(session.toggle_type_visibility(table));
        assert!(!session.is_type_visible(table));
        assert_eq!(session.revision(), before_revision.wrapping_add(1));
        assert!(session.instances().unwrap().sprite(selected).is_none());
        assert_eq!(
            session.set_selected_instance_var("icon_state".into(), Value::Text(String::from("light"))),
            Some(true),
        );
        assert!(session.instances().unwrap().sprite(selected).is_none());

        assert!(session.toggle_type_visibility(table));
        assert!(session.is_type_visible(table));
        assert_eq!(
            session.instances().unwrap().sprite(selected).unwrap().texture,
            light_texture
        );
        assert_render_cache_matches_rebuild(&session);
    }
}
