use std::collections::HashSet;

use dmm::Coord;
use editor::{document::PrefabInstanceId, visual};
use render::GuideLine;

use super::Session;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct GuideBadge {
    pub position: [f32; 2],
    pub z: u32,
}

#[derive(Debug, Default)]
pub(crate) struct SelectionGuides {
    pub lines: Vec<GuideLine>,
    pub badges: Vec<GuideBadge>,
    pub connected: Vec<PrefabInstanceId>,
}

impl Session {
    pub(crate) fn selected_offset_guide(&self) -> Option<GuideLine> {
        let environment = self.state.environment.as_ref()?;
        let document = self.state.active_document()?;
        let selected = document.selected_instance()?;
        let (prefab, location) = document.prefab_instance(selected)?;
        if location.coord.z != document.z {
            return None;
        }

        let id = environment.tree.id_of(&prefab.path)?;
        let appearance = visual::resolve_id(&environment.tree, id, prefab);
        let displacement = [
            appearance
                .step_x
                .saturating_add(appearance.pixel_x)
                .saturating_add(appearance.pixel_w),
            appearance
                .step_y
                .saturating_add(appearance.pixel_y)
                .saturating_add(appearance.pixel_z),
        ];
        if displacement == [0, 0] {
            return None;
        }

        let sprite = self.instances()?.sprite(selected)?;
        let tile_size = self.options.tile_size.max(1) as f32;

        Some(GuideLine {
            origin: [
                (location.coord.x as f32 - 0.5) * tile_size,
                (location.coord.y as f32 - 0.5) * tile_size,
            ],
            target: [sprite.x + sprite.width * 0.5, sprite.y + sprite.height * 0.5],
        })
    }

    pub(crate) fn selected_guides(&self) -> SelectionGuides {
        let mut guides = SelectionGuides::default();
        if let Some(offset) = self.selected_offset_guide() {
            guides.lines.push(offset);
        }

        let Some(document_id) = self.state.active() else {
            return guides;
        };
        let Some(document) = self.state.document(document_id) else {
            return guides;
        };
        let Some(selected) = document.selected_instance() else {
            return guides;
        };
        let Some((_, selected_location)) = document.prefab_instance(selected) else {
            return guides;
        };

        if selected_location.coord.z != document.z {
            return guides;
        }

        let Some(cache) = self.caches.get(&document_id) else {
            return guides;
        };

        let Some(bake) = cache.bake.as_ref() else {
            return guides;
        };

        let tile_size = self.options.tile_size.max(1) as f32;
        let center = |id: PrefabInstanceId, coord: Coord| {
            cache.instances.sprite(id).map_or_else(
                || [(coord.x as f32 - 0.5) * tile_size, (coord.y as f32 - 0.5) * tile_size],
                |sprite| [sprite.x + sprite.width * 0.5, sprite.y + sprite.height * 0.5],
            )
        };

        let selected_center = center(selected, selected_location.coord);
        let mut badged = HashSet::new();

        for connected in bake.connections(selected.get()) {
            let Some(connected) = PrefabInstanceId::from_raw(connected) else {
                continue;
            };
            let Some((_, location)) = document.prefab_instance(connected) else {
                continue;
            };
            let target = center(connected, location.coord);
            guides.lines.push(GuideLine {
                origin: selected_center,
                target,
            });
            guides.connected.push(connected);

            if location.coord.z != document.z && badged.insert((location.coord.x, location.coord.y, location.coord.z)) {
                guides.badges.push(GuideBadge {
                    position: target,
                    z: location.coord.z,
                });
            }
        }

        guides
    }
}

#[cfg(test)]
mod tests {
    use core::{path::TreePath, types::Value};
    use std::sync::atomic::{AtomicUsize, Ordering};

    use dmm::{Coord, Map, Prefab, Size};
    use editor::document::{MapDocument, VarMutation};
    use render::GuideLine;

    use super::GuideBadge;
    use crate::session::{
        Session,
        fixtures::{examples, settle_bake},
    };

    #[test]
    fn selection_guides_connect_the_tile_center_to_the_rendered_offset() {
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

        assert_eq!(session.selected_offset_guide(), None);
        assert_eq!(
            session.edit_selected_instance_vars(
                "offset selected object",
                &[
                    VarMutation::Set("step_x".into(), Value::Num(-2.0)),
                    VarMutation::Set("pixel_x".into(), Value::Num(5.0)),
                    VarMutation::Set("pixel_w".into(), Value::Num(1.0)),
                    VarMutation::Set("step_y".into(), Value::Num(3.0)),
                    VarMutation::Set("pixel_y".into(), Value::Num(-4.0)),
                    VarMutation::Set("pixel_z".into(), Value::Num(2.0)),
                ],
                None,
            ),
            Some(true),
        );

        let guide = session.selected_offset_guide().unwrap();
        assert_eq!(guide.origin, [176.0, 80.0]);
        assert_eq!(guide.target, [180.0, 81.0]);

        session.state.active_document_mut().unwrap().z = 2;
        assert_eq!(session.selected_offset_guide(), None);
        session.state.active_document_mut().unwrap().z = 1;
        assert_eq!(
            session.edit_selected_instance_vars(
                "cancel selected object offset",
                &[
                    VarMutation::Set("step_x".into(), Value::Num(-5.0)),
                    VarMutation::Set("pixel_x".into(), Value::Num(5.0)),
                    VarMutation::Set("pixel_w".into(), Value::Num(0.0)),
                    VarMutation::Set("step_y".into(), Value::Num(4.0)),
                    VarMutation::Set("pixel_y".into(), Value::Num(-4.0)),
                    VarMutation::Set("pixel_z".into(), Value::Num(0.0)),
                ],
                None,
            ),
            Some(true),
        );
        assert_eq!(session.selected_offset_guide(), None);
    }

    #[test]
    fn selection_guides_follow_profile_connections_in_both_directions_and_across_levels() {
        static NEXT: AtomicUsize = AtomicUsize::new(0);

        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("dmed-connection-guides-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&root).expect("temp connection fixture");
        std::fs::copy(examples().join("icons/test.dmi"), root.join("test.dmi")).expect("fixture icon");
        std::fs::write(
            root.join("game.dm"),
            r#"
/area/test
/turf/floor
    icon = 'test.dmi'
    icon_state = "floor"
/obj/source
    icon = 'test.dmi'
    icon_state = "table"
    pixel_x = 4
    var/channel
/obj/target
    icon = 'test.dmi'
    icon_state = "light"
    var/channel
"#,
        )
        .expect("fixture game");
        std::fs::write(
            root.join("profile.dm"),
            r#"
#ifdef __DEMIR_BAKE__
/datum/demir/test
    default = TRUE
/datum/demir/test/highlights(atom/target)
    if(!istype(target, /obj/source))
        return
    return list(list("width" = 3, "height" = 1, "when" = DEMIR_HIGHLIGHT_SELECTED))
/datum/demir/test/connections(atom/target)
    var/list/connections = list()
    if(istype(target, /obj/source))
        var/obj/source/source = target
        connections[source.channel] = DEMIR_CONNECTION_SOURCE
    else if(istype(target, /obj/target))
        var/obj/target/destination = target
        connections[destination.channel] = DEMIR_CONNECTION_TARGET
    return connections
#endif
"#,
        )
        .expect("fixture profile");
        std::fs::write(root.join("test.dme"), "#include \"game.dm\"\n#include \"profile.dm\"\n")
            .expect("fixture environment");

        let mut session = Session::new();
        session
            .load_environment(&root.join("test.dme"))
            .expect("connection environment");
        let mut map = Map::new(Size { x: 3, y: 1, z: 2 });
        let floor = || Prefab::new(TreePath::parse("/turf/floor"));
        let area = || Prefab::new(TreePath::parse("/area/test"));
        let mut source = Prefab::new(TreePath::parse("/obj/source"));
        source.set_var("channel".into(), Value::Text(String::from("doors")));
        let mut target = Prefab::new(TreePath::parse("/obj/target"));
        target.set_var("channel".into(), Value::Text(String::from("doors")));
        let source_tile = map.intern_tile(vec![source, floor(), area()]);
        let target_tile = map.intern_tile(vec![target.clone(), floor(), area()]);
        let floor_tile = map.intern_tile(vec![floor(), area()]);
        map.grid[0][0] = vec![source_tile, floor_tile, target_tile];
        map.grid[1][0] = vec![floor_tile, target_tile, floor_tile];
        session.activate_document(MapDocument::new(map, 1));
        settle_bake(&mut session);

        let endpoint = |session: &Session, coord: Coord, path: &str| {
            let document = session.state.active_document().expect("active fixture map");
            document
                .instance_ids_at(coord)
                .iter()
                .copied()
                .find(|id| {
                    document
                        .prefab_instance(*id)
                        .is_some_and(|(prefab, _)| prefab.path == TreePath::parse(path))
                })
                .expect("fixture endpoint")
        };
        let source = endpoint(&session, Coord::new(1, 1, 1), "/obj/source");
        let same_level = endpoint(&session, Coord::new(3, 1, 1), "/obj/target");
        session.select_instance(Some(source));

        let guides = session.selected_guides();
        assert_eq!(guides.lines.len(), 3, "one offset guide and two connections");
        assert!(guides.lines.contains(&GuideLine {
            origin: [16.0, 16.0],
            target: [20.0, 16.0],
        }));
        assert!(guides.lines.contains(&GuideLine {
            origin: [20.0, 16.0],
            target: [80.0, 16.0],
        }));
        assert!(guides.lines.contains(&GuideLine {
            origin: [20.0, 16.0],
            target: [48.0, 16.0],
        }));
        assert_eq!(
            guides.badges,
            vec![GuideBadge {
                position: [48.0, 16.0],
                z: 2,
            }]
        );
        assert_eq!(
            guides.connected.len(),
            2,
            "both endpoints are highlighted, the offset guide adds none"
        );
        assert!(guides.connected.contains(&same_level));

        let document_id = session.state.active().expect("active fixture document");
        let highlights = session
            .highlights(document_id, None)
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        let [highlight] = highlights.as_slice() else {
            panic!("the selected source declares one highlight");
        };
        assert_eq!(
            highlight.tiles.iter().map(|tile| tile.position).collect::<Vec<_>>(),
            vec![[1, 1], [2, 1], [3, 1]]
        );
        assert!(
            session
                .highlights(document_id, Some(Coord::new(1, 1, 1)))
                .iter()
                .count()
                == 1,
            "a selection-only highlight is not repeated by hovering its own tile"
        );

        session.select_instance(Some(same_level));
        let reverse = session.selected_guides();
        assert_eq!(
            reverse.lines,
            vec![GuideLine {
                origin: [80.0, 16.0],
                target: [20.0, 16.0],
            }]
        );
        assert!(reverse.badges.is_empty());
        assert_eq!(reverse.connected, vec![source]);
        assert!(
            session.highlights(document_id, None).iter().next().is_none(),
            "only the source declares a highlight"
        );

        assert_eq!(
            session.set_selected_instance_var("channel".into(), Value::Text(String::from("other"))),
            Some(true),
        );
        let disconnected = session.selected_guides();
        assert!(disconnected.lines.is_empty());
        assert!(disconnected.connected.is_empty());

        let _ = std::fs::remove_dir_all(root);
    }
}
