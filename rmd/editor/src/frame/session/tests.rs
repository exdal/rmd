use base::{path::TreePath, types::Value};

use dmm::{Coord, Map, Prefab, Size};

use crate::{
    bake::session::MAX_PENDING_UPDATES,
    document::{MapDocument, selection::Selection},
    frame::session::TypeLayer,
    session::{
        Session,
        fixtures::{assert_render_cache_matches_rebuild, examples, flat_session, settle_bake},
    },
    tool::Tool,
};

#[test]
fn capture_regions_measure_from_the_bottom_left_and_clamp_to_the_map() {
    let session = flat_session(4, 3);
    let id = session.state.active().unwrap();
    let tile = session.options.tile_size;
    let region = |min: (u32, u32), max: (u32, u32)| {
        Some(Selection::from_drag(
            Coord::new(min.0, min.1, 1),
            Coord::new(max.0, max.1, 1),
        ))
    };

    assert_eq!(session.capture_region(id, None), Some(([0, 0], [4 * tile, 3 * tile])));
    assert_eq!(
        session.capture_region(id, region((2, 2), (3, 3))),
        Some(([tile, tile], [2 * tile, 2 * tile]))
    );
    assert_eq!(
        session.capture_region(id, region((3, 2), (9, 9))),
        Some(([2 * tile, tile], [2 * tile, 2 * tile]))
    );
    assert_eq!(session.capture_region(id, region((5, 1), (6, 1))), None);
}

#[test]
fn remote_edits_update_the_render_cache_like_a_rebuild() {
    let mut session = flat_session(3, 1);
    let id = session.state.active().unwrap();
    let (from, to) = (Coord::new(1, 1, 1), Coord::new(2, 1, 1));
    session.set_tool(Tool::Place);
    session
        .state
        .choose_prefab(Prefab::new(TreePath::parse("/obj/structure/table")));
    session.place_at(from, None).unwrap();

    let tile = |session: &Session, coord| session.map().unwrap().tile_at(coord).unwrap().clone();
    let mut source = tile(&session, from);
    let table = source.remove(source.len() - 1);
    let mut destination = tile(&session, to);
    destination.push(table);
    let mut repainted = tile(&session, Coord::new(3, 1, 1));
    let mut engineering = Prefab::new(TreePath::parse("/area/station"));
    engineering.set_var("name".into(), Value::Text(String::from("Engineering")));
    *repainted.last_mut().unwrap() = engineering;
    let affected = session.state.document_mut(id).unwrap().apply_remote(vec![
        (from, source),
        (to, destination),
        (Coord::new(3, 1, 1), repainted),
    ]);
    session.update_document_instances(id, &affected);

    assert_eq!(affected.len(), 4, "the moved table and the repainted area, old and new");
    assert_render_cache_matches_rebuild(&session);
}

fn named_table(name: &str) -> Prefab {
    let mut table = Prefab::new(TreePath::parse("/obj/structure/table"));
    table.set_var("name".into(), Value::Text(name.into()));

    table
}

fn tables_session(names: &[&str]) -> Session {
    let mut session = flat_session(1, 1);
    session.set_tool(Tool::Place);
    for name in names {
        session.state.choose_prefab(named_table(name));
        session.place_at(Coord::new(1, 1, 1), None).unwrap();
    }

    session
}

fn drawn_tables(session: &Session) -> Vec<String> {
    let document = session.state.active_document().unwrap();
    session
        .instances()
        .unwrap()
        .sprites
        .iter()
        .filter_map(|sprite| document.prefab_instance(sprite.owner))
        .filter(|instance| instance.prefab().path == TreePath::parse("/obj/structure/table"))
        .map(|instance| match instance.prefab().var(&"name".into()) {
            Some(Value::Text(name)) => name.clone(),
            _ => String::new(),
        })
        .collect()
}

fn apply_remote_tile(session: &mut Session, tile: Vec<Prefab>) {
    let id = session.state.active().unwrap();
    assert!(session.apply_remote_tiles(id, vec![(Coord::new(1, 1, 1), tile)]));
}

#[test]
fn a_remote_reorder_changes_equal_layer_draw_order() {
    let mut session = tables_session(&["a", "b"]);
    assert_eq!(drawn_tables(&session), ["a", "b"]);
    let mut tile = session.map().unwrap().tile_at(Coord::new(1, 1, 1)).unwrap().clone();
    let a = tile.iter().position(|prefab| *prefab == named_table("a")).unwrap();
    let b = tile.iter().position(|prefab| *prefab == named_table("b")).unwrap();
    tile.swap(a, b);

    apply_remote_tile(&mut session, tile);

    assert_eq!(drawn_tables(&session), ["b", "a"]);
    assert_render_cache_matches_rebuild(&session);
}

#[test]
fn a_remote_insert_between_equal_layers_draws_between_them() {
    let mut session = tables_session(&["a", "b"]);
    let mut tile = session.map().unwrap().tile_at(Coord::new(1, 1, 1)).unwrap().clone();
    let b = tile.iter().position(|prefab| *prefab == named_table("b")).unwrap();
    tile.insert(b, named_table("c"));

    apply_remote_tile(&mut session, tile);

    assert_eq!(drawn_tables(&session), ["a", "c", "b"]);
    assert_render_cache_matches_rebuild(&session);
}

#[test]
fn edits_between_frames_chain_their_updates_up_to_a_cap() {
    let mut session = flat_session(12, 1);
    session.set_tool(Tool::Place);
    session
        .state
        .choose_prefab(Prefab::new(TreePath::parse("/obj/structure/table")));
    let before = session.active_cache().revision;

    session.place_at(Coord::new(1, 1, 1), None).unwrap();
    session.place_at(Coord::new(2, 1, 1), None).unwrap();

    let updates = &session.active_cache().frame_updates;
    assert_eq!(updates.len(), 2);
    assert_eq!(updates[0].previous_revision, before);

    for x in 3..=12 {
        session.place_at(Coord::new(x, 1, 1), None).unwrap();
    }
    assert_eq!(session.active_cache().frame_updates.len(), MAX_PENDING_UPDATES);
}

#[test]
fn layer_toggles_hide_a_base_type_and_show_all_restores_everything() {
    let mut session = flat_session(2, 1);
    let turf = session.map().unwrap().tile_at(Coord::new(1, 1, 1)).unwrap()[0].clone();
    assert!(session.is_layer_visible(TypeLayer::Turf));
    assert!(!session.hides_types());

    assert!(session.toggle_layer(TypeLayer::Turf));
    assert!(session.toggle_layer(TypeLayer::Obj));
    assert!(!session.is_layer_visible(TypeLayer::Turf));
    assert!(session.is_layer_visible(TypeLayer::Area));
    assert!(
        session.hidden_types().hides(&turf),
        "subtypes are hidden with their base type"
    );

    assert!(session.toggle_layer(TypeLayer::Turf));
    assert!(session.is_layer_visible(TypeLayer::Turf));
    assert!(!session.is_layer_visible(TypeLayer::Obj));

    assert!(session.show_all_types());
    assert!(session.is_layer_visible(TypeLayer::Obj));
    assert!(!session.hides_types());
    assert!(!session.show_all_types(), "nothing left to show");
}

#[test]
fn both_open_maps_contribute_their_own_sprites_to_one_frame() {
    let root = examples();
    let mut session = Session::new();
    session.load_environment(&root.join("test.dme")).expect("codebase");
    session.open_map(&root.join("test.dmm"), 1).expect("first map");
    session.open_map(&root.join("test2.dmm"), 1).expect("second map");
    let ids = session.state.document_ids();

    // Two maps docked side by side in one window.
    let left = render::MapViewRect {
        x: 0,
        y: 0,
        width: 400,
        height: 600,
    };
    let right = render::MapViewRect {
        x: 400,
        y: 0,
        width: 400,
        height: 600,
    };
    let map_views = [
        session
            .map_view_frame(ids[0], left, Default::default(), Default::default(), &[], &[], &[])
            .expect("left map view"),
        session
            .map_view_frame(ids[1], right, Default::default(), Default::default(), &[], &[], &[])
            .expect("right map view"),
    ];
    let frame = session.frame(&map_views, Some(1));

    assert_eq!(frame.map_views.len(), 2, "both maps are in the frame");
    assert_eq!(frame.picking, Some(1));
    assert_ne!(frame.map_views[0].rect, frame.map_views[1].rect);
    // Each map view carries its own map's sprites, not a shared cache.
    for id in &ids {
        assert!(session.caches[id].instances.live_sprites().next().is_some());
    }
    assert_ne!(frame.map_views[0].sprite_instances, frame.map_views[1].sprite_instances);
}

#[test]
fn hiding_a_type_redraws_from_the_cached_bake() {
    let root = examples();
    let mut session = Session::new();
    session.load_environment(&root.join("test.dme")).expect("codebase");
    session.open_map(&root.join("test.dmm"), 1).expect("map");
    settle_bake(&mut session);
    let id = session.state.active().expect("active map");
    let table = session
        .tree()
        .and_then(|tree| tree.id_of(&TreePath::parse("/obj/structure/table")))
        .expect("table type");

    // A bake that ran again would start its counters over.
    session
        .caches
        .get_mut(&id)
        .and_then(|cache| cache.bake.as_mut())
        .expect("the example map is baked")
        .cache_hits = usize::MAX;

    let revision = session.active_cache().revision;
    assert!(session.toggle_type_visibility(table));
    assert_eq!(
        session.active_cache().bake.as_ref().map(|bake| bake.cache_hits),
        Some(usize::MAX)
    );
    assert_eq!(
        session
            .active_cache()
            .frame_updates
            .last()
            .map(|update| update.previous_revision),
        Some(revision),
        "the renderer patches the pages it touched instead of uploading a rebuilt cache"
    );
}

#[test]
fn a_map_view_frame_follows_its_own_documents_z_level() {
    let root = examples();
    let mut session = Session::new();
    session.load_environment(&root.join("test.dme")).expect("codebase");
    session.open_map(&root.join("test.dmm"), 1).expect("first map");
    let first = session.state.active().expect("first is active");
    // test2.dmm has two z levels.
    session.open_map(&root.join("test2.dmm"), 2).expect("second map");
    let second = session.state.active().expect("second is active");

    let rect = render::MapViewRect::default();
    let a = session
        .map_view_frame(first, rect, Default::default(), Default::default(), &[], &[], &[])
        .expect("first map view");
    let b = session
        .map_view_frame(second, rect, Default::default(), Default::default(), &[], &[], &[])
        .expect("second map view");

    assert_eq!(a.active_z, 1);
    assert_eq!(b.active_z, 2, "z is per document, not per editor");
}

#[test]
fn two_open_maps_never_share_a_sprite_revision() {
    let root = examples();
    let mut session = Session::new();
    session.load_environment(&root.join("test.dme")).expect("codebase");
    session.open_map(&root.join("test.dmm"), 1).expect("first map");
    session.open_map(&root.join("test2.dmm"), 1).expect("second map");

    let ids = session.state.document_ids();
    assert_eq!(ids.len(), 2, "both maps stay open");

    // The renderer holds a single `uploaded_revision` for whatever is on the
    // GPU. Two documents claiming one revision makes it skip the upload and
    // keep drawing the map that got there first.
    let revisions: Vec<u64> = ids.iter().map(|id| session.caches[id].revision).collect();
    assert_ne!(revisions[0], revisions[1]);

    // And the caches really do describe different maps.
    let first = session
        .map_view_frame(
            ids[0],
            render::MapViewRect::default(),
            render::Camera::default(),
            Default::default(),
            &[],
            &[],
            &[],
        )
        .expect("first frame");
    let second = session
        .map_view_frame(
            ids[1],
            render::MapViewRect::default(),
            render::Camera::default(),
            Default::default(),
            &[],
            &[],
            &[],
        )
        .expect("second frame");
    assert_ne!(first.revision, second.revision);
    assert_ne!(first.sprite_instances, second.sprite_instances);
}

#[test]
fn editing_one_open_map_leaves_the_other_cache_alone() {
    let root = examples();
    let mut session = Session::new();
    session.load_environment(&root.join("test.dme")).expect("codebase");
    session.open_map(&root.join("test.dmm"), 1).expect("first map");
    let first = session.state.active().expect("first is active");
    session.open_map(&root.join("test2.dmm"), 1).expect("second map");
    let second = session.state.active().expect("second is active");

    let untouched = session.caches[&first].revision;
    let before = session.caches[&second].revision;

    session.choose_type(
        session
            .tree()
            .and_then(|tree| tree.id_of(&TreePath::parse("/obj/structure/table")))
            .expect("a placeable type"),
    );
    assert!(session.place_at(Coord::new(2, 2, 1), None).is_some(), "edit applied");

    assert_eq!(session.caches[&first].revision, untouched, "the other map is untouched");
    assert_ne!(session.caches[&second].revision, before, "the edited map re-uploads");
}

#[test]
fn view_changes_do_not_change_the_sprite_revision() {
    let mut session = Session::new();
    let id = session
        .state
        .open_document(MapDocument::new(Map::new(Size { x: 1, y: 1, z: 3 }), 1));
    session.caches.entry(id).or_default().revision = 7;

    session.set_level(2);
    session.set_underlay_depth(1);
    session.toggle_areas();
    assert!(session.options.show_areas);
    assert!(session.options.show_area_outlines);
    session.toggle_area_outlines();

    let view = session
        .map_view_frame(
            id,
            render::MapViewRect::default(),
            Default::default(),
            Default::default(),
            &[],
            &[],
            &[],
        )
        .expect("the open map has a map view");
    assert_eq!(view.revision, 7, "changing the view does not re-upload sprites");
    assert_eq!(view.active_z, 2);

    let map_views = [view];
    let frame = session.frame(&map_views, None);
    assert_eq!(frame.underlay_depth, 1);
    assert!(frame.show_areas);
    assert!(!frame.show_area_outlines);
}

#[test]
fn toggling_lighting_off_withholds_the_light_grid() {
    let mut session = Session::new();
    let id = session
        .state
        .open_document(MapDocument::new(Map::new(Size { x: 1, y: 1, z: 1 }), 1));
    let cache = session.caches.entry(id).or_default();
    cache.instances.lighting_size = [1, 1, 1];
    cache.instances.light_tiles = vec![render::LightTile { corners: [[1.0; 3]; 4] }];

    let view = |session: &Session| {
        session
            .map_view_frame(
                id,
                render::MapViewRect::default(),
                Default::default(),
                Default::default(),
                &[],
                &[],
                &[],
            )
            .expect("the open map has a map view")
            .lighting
            .map(|lighting| lighting.minimum_brightness)
    };

    assert!(session.options.show_lighting);
    assert_eq!(view(&session), Some(0.0));

    session.options.minimum_light_brightness_percent = 35;
    assert_eq!(view(&session), Some(0.35));

    session.toggle_lighting();
    assert_eq!(view(&session), None);

    session.toggle_lighting();
    assert_eq!(view(&session), Some(0.35));
}
