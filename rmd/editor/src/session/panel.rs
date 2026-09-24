use editor::document::{DocumentId, PrefabInstanceId};

use super::{Session, report_bake_output};

impl Session {
    pub(crate) fn dm_ui(&mut self, dockspace: u32, feedback: editor::bake::UiFeedback) -> editor::bake::UiFrame {
        let Some(id) = self.state.active() else {
            return editor::bake::UiFrame::default();
        };
        let Some(program) = self
            .state
            .environment
            .as_ref()
            .and_then(|environment| environment.bake_program.as_ref())
        else {
            return editor::bake::UiFrame::default();
        };
        let target = self
            .state
            .document(id)
            .and_then(|document| document.selected_instance())
            .map(PrefabInstanceId::get);
        let Some(bake) = self.caches.get_mut(&id).and_then(|cache| cache.bake.as_mut()) else {
            return editor::bake::UiFrame::default();
        };

        let mut replayed = feedback.clone();
        replayed.mouse_popup_requested = false;
        let drawn = bake.ui(&program.tree, &program.module, target, dockspace, feedback);
        report_bake_output(bake);

        match drawn {
            Ok(frame) => {
                self.ui_fault = None;
                if frame.committed {
                    for other in self.state.document_ids() {
                        if other != id {
                            self.replay_ui(other, &replayed);
                        }
                    }

                    self.ui_feedback = Some(replayed);
                }

                frame
            },
            Err(fault) => {
                if self.ui_fault.as_ref() != Some(&fault.kind) {
                    log::warn!("demir_ui: {fault}");
                    self.ui_fault = Some(fault.kind);
                }

                editor::bake::UiFrame::default()
            },
        }
    }

    pub(super) fn replay_ui(&mut self, id: DocumentId, feedback: &editor::bake::UiFeedback) {
        let Some(environment) = self.state.environment.clone() else {
            return;
        };
        let Some(program) = environment.bake_program.as_ref() else {
            return;
        };
        let target = self
            .state
            .document(id)
            .and_then(|document| document.selected_instance())
            .map(PrefabInstanceId::get);
        let Some(cache) = self.caches.get_mut(&id) else {
            return;
        };

        // a bake still on the baker thread is caught up by poll_bake instead
        let Some(bake) = cache.bake.as_mut() else {
            return;
        };

        let drawn = bake.ui(&program.tree, &program.module, target, 0, feedback.clone());
        report_bake_output(bake);

        if let Ok(frame) = drawn {
            cache.pending_rebake.merge(frame.rebake);
        }
    }

    pub(super) fn flush_pending_rebake(&mut self, id: DocumentId) {
        let request = self
            .caches
            .get_mut(&id)
            .map(|cache| std::mem::take(&mut cache.pending_rebake))
            .unwrap_or_default();

        if !request.is_empty() {
            self.run_rebake(id, request);
        }
    }

    fn flush_pending_rebakes(&mut self) {
        for id in self.state.document_ids() {
            self.flush_pending_rebake(id);
        }
    }

    pub(crate) fn dm_ui_rebake(&mut self, request: editor::bake::UiRebake) {
        if let Some(id) = self.state.active() {
            self.caches.entry(id).or_default().pending_rebake.merge(request);
        }

        self.flush_pending_rebakes();
    }

    fn run_rebake(&mut self, id: DocumentId, request: editor::bake::UiRebake) {
        let Some(program) = self
            .state
            .environment
            .as_ref()
            .and_then(|environment| environment.bake_program.as_ref())
        else {
            return;
        };
        let Some(bake) = self.caches.get_mut(&id).and_then(|cache| cache.bake.as_mut()) else {
            return;
        };

        let update = bake.rebake(&program.tree, &program.module, request);
        report_bake_output(bake);

        self.apply_bake_update(
            id,
            editor::bake::BakeUpdate {
                appearances: update
                    .appearances
                    .into_iter()
                    .filter_map(PrefabInstanceId::from_raw)
                    .collect(),
                lighting: update.lighting,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use core::path::TreePath;
    use std::collections::HashMap;

    use dmm::{Coord, Map, Prefab, Size};
    use editor::document::{DocumentId, MapDocument, PrefabInstanceId};

    use crate::session::{
        Session,
        fixtures::{examples, settle_bake},
    };

    fn panel_session() -> Session {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).expect("codebase");

        session
    }

    fn wall_map(session: &mut Session) -> (DocumentId, PrefabInstanceId) {
        let mut map = Map::new(Size { x: 1, y: 1, z: 1 });
        let key = map.intern_tile(vec![Prefab::new(TreePath::parse("/turf/closed/wall"))]);
        map.grid[0] = vec![vec![key]];
        let id = session.activate_document(MapDocument::new(map, 1));
        let wall = session
            .state
            .document(id)
            .expect("the map is open")
            .instance_ids_at(Coord::new(1, 1, 1))[0];

        (id, wall)
    }

    fn baked_name(session: &Session, id: DocumentId, wall: PrefabInstanceId) -> Option<&str> {
        session.caches[&id]
            .bake
            .as_ref()?
            .appearances
            .get(&wall.get())?
            .vars
            .iter()
            .find(|(name, _)| name.as_str() == "name")
            .and_then(|(_, value)| value.as_text())
    }

    fn toggle_smooth(session: &mut Session, smooth: bool) -> editor::bake::UiFrame {
        session.dm_ui(
            0,
            editor::bake::UiFeedback {
                values: HashMap::from([(String::from("Panel/Smooth"), editor::bake::UiValue::Bool(smooth))]),
                interacted: true,
                ..Default::default()
            },
        )
    }

    #[test]
    fn a_panel_toggle_reaches_every_open_map() {
        let mut session = panel_session();
        let (first, first_wall) = wall_map(&mut session);
        let (second, second_wall) = wall_map(&mut session);
        settle_bake(&mut session);

        assert_eq!(baked_name(&session, first, first_wall), Some("wall 0"));
        assert_eq!(baked_name(&session, second, second_wall), Some("wall 0"));

        let frame = toggle_smooth(&mut session, false);

        assert!(frame.committed, "the interaction keeps the profile's write");
        assert!(!frame.rebake.is_empty(), "the profile asks for its appearances back");

        session.dm_ui_rebake(frame.rebake);

        assert_eq!(baked_name(&session, second, second_wall), Some("plain 0"));
        assert_eq!(
            baked_name(&session, first, first_wall),
            Some("plain 0"),
            "the map nobody is looking at re-derives with the one that was toggled",
        );
        assert!(session.caches[&first].pending_rebake.is_empty());

        session.state.set_active(first);
        session.poll_bake();

        assert_eq!(
            baked_name(&session, first, first_wall),
            Some("plain 0"),
            "activating it changes nothing, it was already current",
        );
    }

    #[test]
    fn a_map_opened_after_a_panel_toggle_catches_up_before_it_is_shown() {
        let mut session = panel_session();
        let (first, first_wall) = wall_map(&mut session);
        settle_bake(&mut session);

        let frame = toggle_smooth(&mut session, false);
        session.dm_ui_rebake(frame.rebake);

        assert_eq!(baked_name(&session, first, first_wall), Some("plain 0"));

        let (late, late_wall) = wall_map(&mut session);
        session.state.set_active(first);
        settle_bake(&mut session);

        assert_eq!(
            baked_name(&session, late, late_wall),
            Some("plain 0"),
            "a bake that lands later catches up without ever being active",
        );
        assert!(session.caches[&late].pending_rebake.is_empty());
    }
}
