use std::error::Error;

use super::*;
use crate::session::modules::tests::{
    Fixture,
    MODULE,
    TestResult,
    WALL,
    assert_settled,
    has,
    open_document,
    place_in,
    show,
};

struct Shared {
    fixture: Fixture,
    session: Session,
    main: DocumentId,
    module: DocumentId,
}

fn hosting_modules() -> TestResult<Shared> {
    let fixture = Fixture::new(&[(2, "outer")])?;
    let mut session = fixture.session()?;
    let main = session.state.active().ok_or("no active document")?;
    let module = open_document(&mut session, &fixture, "a.dmm")?;
    host_fixture(&mut session)?;
    poll_until(&mut [&mut session], |sessions| connected(sessions[0]));

    Ok(Shared {
        fixture,
        session,
        main,
        module,
    })
}

// the fixture's environment is built in memory and has no fingerprint to host with
fn host_fixture(session: &mut Session) -> TestResult {
    let password = net::hash_password("hunter2");
    let codebase = CodebaseId {
        hash: CodebaseHash([7; 32]),
        git_hint: None,
    };
    let server = Server::spawn(ServerConfig {
        bind: SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
        password,
        codebase: Some(codebase.clone()),
    })?;
    let port = server.local_addr().port();
    let connection = CoopConnection::Host { port, password };
    session
        .start_coop(
            SocketAddr::from((Ipv4Addr::LOCALHOST, port)).to_string(),
            password,
            String::from("host"),
            codebase,
            connection,
            Some(server),
        )
        .map_err(Box::<dyn Error>::from)?;

    Ok(())
}

fn share(session: &mut Session, id: DocumentId, path: &str) {
    session.set_active_document(id);
    session.share_coop_map();
    poll_until(&mut [session], |sessions| {
        sessions[0].coop().unwrap().shared_maps.contains_key(path) && settled(sessions)
    });
}

fn is_read_only(session: &Session, id: DocumentId) -> bool { session.state.document(id).unwrap().is_read_only() }

fn module_edit(seen: u32) -> net::MapEdit {
    let mut patch = dmm::Map::new(dmm::Size { x: 1, y: 1, z: 1 });
    let tile = ["/obj/module", "/turf/open/floor", "/area/station"]
        .map(|path| Prefab::new(TreePath::parse(path)))
        .to_vec();
    patch.grid[0][0][0] = patch.intern_tile(tile);

    net::MapEdit {
        path: String::from("a.dmm"),
        generation: GenerationId(1),
        coords: vec![[1, 1, 1]],
        patch: dmm::writer::write(&patch),
        level: None,
        seen: SeqId(seen),
        level_id: 0,
    }
}

#[test]
fn a_shared_map_placed_as_a_module_is_read_only() -> TestResult {
    let Shared {
        mut session,
        main,
        module,
        ..
    } = hosting_modules()?;

    share(&mut session, module, "a.dmm");

    assert!(is_read_only(&session, module));
    assert!(!is_read_only(&session, main));
    session.set_active_document(module);
    assert!(session.place_at(dmm::Coord::new(1, 1, 1), None).is_none());

    Ok(())
}

#[test]
fn a_module_is_editable_again_once_its_container_closes() -> TestResult {
    let Shared {
        mut session,
        main,
        module,
        ..
    } = hosting_modules()?;
    share(&mut session, module, "a.dmm");
    assert!(is_read_only(&session, module));

    assert!(session.close_map(main));
    session.poll_coop();

    assert!(!is_read_only(&session, module));

    Ok(())
}

#[test]
fn a_module_is_editable_again_once_it_stops_being_shared() -> TestResult {
    let Shared {
        mut session, module, ..
    } = hosting_modules()?;
    share(&mut session, module, "a.dmm");
    assert!(is_read_only(&session, module));

    session.stop_sharing_coop_document(module);
    poll_until(&mut [&mut session], |sessions| {
        sessions[0].coop().unwrap().shared_maps.is_empty()
    });

    assert!(!is_read_only(&session, module));

    Ok(())
}

#[test]
fn a_module_is_editable_again_once_co_op_ends() -> TestResult {
    let Shared {
        mut session, module, ..
    } = hosting_modules()?;
    share(&mut session, module, "a.dmm");
    assert!(is_read_only(&session, module));

    session.leave_coop();

    assert!(!is_read_only(&session, module));

    Ok(())
}

#[test]
fn the_containing_map_stays_editable_while_shared() -> TestResult {
    let Shared {
        mut session,
        main,
        module,
        ..
    } = hosting_modules()?;

    share(&mut session, main, "main.dmm");
    assert!(!is_read_only(&session, main));

    share(&mut session, module, "a.dmm");
    assert!(is_read_only(&session, module));
    assert!(!is_read_only(&session, main));

    Ok(())
}

#[test]
fn a_shared_map_no_open_map_places_stays_editable() -> TestResult {
    let Shared {
        fixture, mut session, ..
    } = hosting_modules()?;
    fs::copy(fixture.dir.join("a.dmm"), fixture.dir.join("other.dmm"))?;
    let other = open_document(&mut session, &fixture, "other.dmm")?;

    share(&mut session, other, "other.dmm");

    assert!(!is_read_only(&session, other));

    Ok(())
}

#[test]
fn an_unshared_module_stays_editable_and_shows_up_in_its_container() -> TestResult {
    let Shared {
        mut session,
        main,
        module,
        ..
    } = hosting_modules()?;
    share(&mut session, main, "main.dmm");
    session.poll_coop();

    assert!(!is_read_only(&session, module));
    place_in(&mut session, module, 1, MODULE)?;
    show(&mut session, main);

    assert!(has(&session, 2, MODULE));
    assert_settled(&session);

    Ok(())
}

#[test]
fn a_remote_edit_to_a_locked_module_shows_up_in_its_container() -> TestResult {
    let Shared {
        fixture,
        mut session,
        main,
        module,
    } = hosting_modules()?;
    share(&mut session, module, "a.dmm");
    assert!(is_read_only(&session, module));
    show(&mut session, main);
    assert!(has(&session, 2, WALL));
    assert!(!has(&session, 2, MODULE));

    deliver(
        &mut session,
        &fixture.dir,
        Event::Edit {
            by: OTHER,
            seq: SeqId(0),
            edit: module_edit(0),
        },
    );
    show(&mut session, main);

    assert!(is_read_only(&session, module));
    assert_eq!(
        top(&session, &fixture.dir.join("a.dmm"), dmm::Coord::new(1, 1, 1)).as_deref(),
        Some(MODULE)
    );
    assert!(has(&session, 2, MODULE));
    assert!(!has(&session, 2, WALL));
    assert_settled(&session);

    Ok(())
}
