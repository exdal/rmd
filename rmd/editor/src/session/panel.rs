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
        let is_faulted = bake.ui_fault.is_some();

        let frame = match drawn {
            Ok(frame) => {
                if !is_faulted {
                    self.logged_ui_fault = None;
                }

                if frame.committed {
                    if let Some(is_enabled) = frame.modular_preview {
                        self.set_modules_enabled(is_enabled);
                    }

                    let mut settled = vec![id];
                    for other in self.state.document_ids() {
                        if other != id && self.replay_ui(other, &replayed) {
                            settled.push(other);
                        }
                    }

                    self.ui_feedback = Some(replayed);
                    for document in settled {
                        self.settle_modules(document, Vec::new());
                    }
                }

                frame
            },
            Err(fault) => {
                let key = fault.key();
                if self.logged_ui_fault.as_ref() != Some(&key) {
                    if let Some(environment) = &self.state.environment {
                        log::warn!("DM ui(): {}", environment.fault_fault_report(&fault));
                    }
                    self.logged_ui_fault = Some(key);
                }

                editor::bake::UiFrame::default()
            },
        };

        self.log_runtimes();
        frame
    }

    pub(super) fn replay_ui(&mut self, id: DocumentId, feedback: &editor::bake::UiFeedback) -> bool {
        let Some(environment) = self.state.environment.clone() else {
            return false;
        };
        let Some(program) = environment.bake_program.as_ref() else {
            return false;
        };
        let target = self
            .state
            .document(id)
            .and_then(|document| document.selected_instance())
            .map(PrefabInstanceId::get);
        let Some(cache) = self.caches.get_mut(&id) else {
            return false;
        };

        // a bake still on the baker thread is caught up by poll_bake instead
        let Some(bake) = cache.bake.as_mut() else {
            return false;
        };

        let drawn = bake.ui(&program.tree, &program.module, target, 0, feedback.clone());
        report_bake_output(bake);

        let Ok(frame) = drawn else {
            return false;
        };

        cache.pending_rebake.merge(frame.rebake);

        frame.committed
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

        self.collect_bake_diagnostics();
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
        self.settle_modules(id, Vec::new());
    }
}

#[cfg(test)]
mod tests {
    use core::path::TreePath;
    use std::{
        collections::{HashMap, HashSet},
        path::PathBuf,
    };

    use dmm::{Coord, Map, Prefab, Size};
    use editor::document::{DocumentId, MapDocument, PrefabInstanceId};
    use vm::profile::ProfileHook;

    use crate::session::{
        FaultState, Session,
        fixtures::{
            examples, faulting_profile_session, highlight_toggle_fault_session, interaction_fault_session, settle_bake,
        },
    };

    fn listed(session: &Session) -> Vec<(Option<ProfileHook>, usize, FaultState)> {
        session
            .runtime_faults()
            .iter()
            .flat_map(|(_, faults)| faults.iter().map(|fault| (fault.hook, fault.count, fault.state)))
            .collect()
    }

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

    #[test]
    fn ui_and_bake_faults_are_listed_with_their_hook_and_line() {
        let (mut session, id) = faulting_profile_session();
        session.dm_ui(0, editor::bake::UiFeedback::default());

        let maps = session.runtime_faults();
        let [(document, faults)] = maps.as_slice() else {
            panic!("one map has faults");
        };
        assert_eq!(*document, id);
        let listed = faults
            .iter()
            .map(|fault| (fault.hook, fault.fault.location.begin.line))
            .collect::<Vec<_>>();
        assert_eq!(listed, [(Some(ProfileHook::Bake), 6), (Some(ProfileHook::Ui), 9)]);
    }

    #[test]
    fn a_ui_fault_from_an_interaction_survives_the_idle_frames_after_it() {
        use editor::bake::{UiFeedback, UiValue};

        let (mut session, _) = interaction_fault_session();
        let slider = |hours: f32| UiFeedback {
            values: HashMap::from([(String::from("Panel/Hour"), UiValue::Num(hours))]),
            interacted: true,
            ..Default::default()
        };

        assert!(!session.dm_ui(0, UiFeedback::default()).commands.is_empty());
        assert_eq!(listed(&session), []);

        assert!(session.dm_ui(0, slider(14.0)).commands.is_empty());
        assert_eq!(listed(&session), [(Some(ProfileHook::Ui), 1, FaultState::Occurring)]);
        assert!(session.logged_ui_fault.is_some());

        assert!(!session.dm_ui(0, UiFeedback::default()).commands.is_empty());
        assert_eq!(listed(&session), [(Some(ProfileHook::Ui), 1, FaultState::Occurring)]);
        assert!(session.logged_ui_fault.is_some(), "the same fault is not logged again");

        assert!(!session.dm_ui(0, slider(10.0)).commands.is_empty());
        assert_eq!(
            listed(&session),
            [(Some(ProfileHook::Ui), 1, FaultState::Stopped)],
            "a fault that stopped stays in the log",
        );
        assert!(session.logged_ui_fault.is_none());

        session.clear_runtimes();
        assert_eq!(listed(&session), []);
    }

    #[test]
    #[ignore = "requires the local target/Vanderlin checkout"]
    fn vanderlin_tallies_loot_and_shades_ambushes_within_budget() {
        use editor::bake::{UiCommand, UiFeedback, UiValue};

        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/Vanderlin");
        let mut session = Session::new();
        session
            .load_environment(&root.join("vanderlin.dme"))
            .expect("Vanderlin codebase");
        session
            .open_map(&root.join("_maps/map_files/vanderlin/vanderlin.dmm"), 1)
            .expect("Vanderlin map");
        settle_bake(&mut session);

        let commands = session.dm_ui(0, UiFeedback::default()).commands;
        let key = |prefix: &str| {
            commands
                .iter()
                .find_map(|command| match command {
                    UiCommand::Button { key, label } | UiCommand::Checkbox { key, label, .. }
                        if label.starts_with(prefix) =>
                    {
                        Some(key.clone())
                    },
                    _ => None,
                })
                .unwrap_or_else(|| panic!("the panel draws {prefix}"))
        };
        let tally = key("Tally loot");
        let ambushable = key("Ambushable");

        let pressed = session.dm_ui(
            0,
            UiFeedback {
                clicks: HashSet::from([tally]),
                interacted: true,
                ..Default::default()
            },
        );
        assert!(pressed.committed, "{:#?}", listed(&session));

        let shaded = session.dm_ui(
            0,
            UiFeedback {
                values: HashMap::from([(ambushable, UiValue::Bool(true))]),
                interacted: true,
                ..Default::default()
            },
        );
        session.dm_ui_rebake(shaded.rebake);

        let tallied = session
            .dm_ui(0, UiFeedback::default())
            .commands
            .into_iter()
            .any(|command| matches!(command, UiCommand::Text { text, .. } if text.starts_with("Spawners:")));
        assert!(tallied, "the tally is shown");
        assert_eq!(listed(&session), []);
    }

    #[test]
    fn a_highlight_fault_stays_listed_after_its_checkbox_turns_off() {
        use editor::bake::{UiFeedback, UiValue};

        let (mut session, _) = highlight_toggle_fault_session();
        let toggle = |session: &mut Session, show: bool| {
            let frame = session.dm_ui(
                0,
                UiFeedback {
                    values: HashMap::from([(String::from("Panel/Show"), UiValue::Bool(show))]),
                    interacted: true,
                    ..Default::default()
                },
            );
            assert!(frame.committed);
            session.dm_ui_rebake(frame.rebake);
        };

        session.dm_ui(0, UiFeedback::default());
        assert_eq!(listed(&session), []);

        toggle(&mut session, true);
        assert_eq!(
            listed(&session),
            [(Some(ProfileHook::Highlights), 1, FaultState::Occurring)]
        );

        toggle(&mut session, false);
        assert_eq!(
            listed(&session),
            [(Some(ProfileHook::Highlights), 1, FaultState::Stopped)]
        );

        toggle(&mut session, true);
        assert_eq!(
            listed(&session),
            [(Some(ProfileHook::Highlights), 1, FaultState::Occurring)],
            "the same fault comes back as one entry",
        );

        session.clear_runtimes();
        assert_eq!(
            listed(&session),
            [(Some(ProfileHook::Highlights), 1, FaultState::Occurring)],
            "clearing keeps what still faults",
        );
    }
}
