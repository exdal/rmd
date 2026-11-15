use base::path::TreePath;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};

use dmm::{Coord, Map, Prefab, Size};
use vm::profile::ProfileHook;

use crate::{
    bake::session::FaultState,
    document::{DocumentId, MapDocument, PrefabInstanceId},
    session::{
        Session,
        fixtures::{
            examples, faulting_profile_session, highlight_toggle_fault_session, interaction_fault_session, settle_bake,
        },
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

fn toggle_smooth(session: &mut Session, smooth: bool) -> crate::bake::UiFrame {
    session.dm_ui(
        0,
        crate::bake::UiFeedback {
            values: HashMap::from([(String::from("Panel/Smooth"), crate::bake::UiValue::Bool(smooth))]),
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
    session.dm_ui(0, crate::bake::UiFeedback::default());

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
    use crate::bake::{UiFeedback, UiValue};

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
    use crate::bake::{UiCommand, UiFeedback, UiValue};

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
    use crate::bake::{UiFeedback, UiValue};

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
