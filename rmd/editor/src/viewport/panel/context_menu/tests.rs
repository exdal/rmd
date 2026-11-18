use base::{coord::Coord, location::Location, path::TreePath, types::Value};
use std::path::PathBuf;

use dear_imgui_rs::{Condition, sys};
use dmm::{Map, Size};
use objtree::ObjectTree;
use prefab::Prefab;
use ui::testing::{CONTEXT_LOCK, rectangle_context};

use super::{POPUP, Target, draw_popup};
use crate::{
    environment::worker::LoadedMap, object_tree::panel::type_search::matching_type_paths_up_to_filtered,
    session::Session, settings::Settings, tool::session::context_placement_group,
};

#[test]
fn hovering_atom_rows_opens_submenus_without_truncating_names_or_hit_areas() {
    let _guard = CONTEXT_LOCK.lock().unwrap();
    // With the test font, these table names previously padded the menu labels
    // to 123, 124 and 125 bytes. At 124 bytes ImGui's window name ended in ###.
    let cases = [
        ("/obj/structure/table", "x".repeat(81)),
        ("/obj/structure/table", "x".repeat(82)),
        ("/obj/structure/table", "x".repeat(83)),
        ("/obj/structure/table", "阀门 café ###名前 ".repeat(10)),
        ("/area/station", "Atmospherics pumping room ".repeat(6)),
    ];
    for (path, name, scale) in cases
        .into_iter()
        .flat_map(|(path, name)| [1.0, 1.5, 2.0].map(|scale| (path, name.clone(), scale)))
    {
        let mut context = rectangle_context();
        context.io_mut().set_display_size([4800.0, 2400.0]);
        context.style_mut().scale_all_sizes(scale);
        context.style_mut().set_font_scale_main(scale);
        let mut session = Session::new();
        session
            .load_environment(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/env/test.dme"))
            .unwrap();
        let mut prefab = Prefab::new(TreePath::parse(path));
        prefab.set_var("name".into(), Value::Text(name.clone()));
        let thumbnail_texture = session
            .prefab_thumbnail(&prefab)
            .map(|thumbnail| render::Renderer::sprite_texture(thumbnail.texture.index));
        let mut map = Map::new(Size { x: 1, y: 1, z: 1 });
        let key = map.intern_tile(vec![prefab]);
        map.grid[0][0][0] = key;
        session.apply_map(LoadedMap {
            path: PathBuf::from("context-menu-test.dmm"),
            map,
            z: 1,
            errors: vec![],
            repo: None,
            conflict: None,
        });
        let coord = Coord::new(1, 1, 1);
        let document = session.state.active().unwrap();
        let mut target = Target {
            document,
            coord,
            atoms: session.state.active_document().unwrap().instance_ids_at(coord).to_vec(),
            node: None,
            region_selected: false,
            replace_for: None,
            replace_query: String::new(),
        };
        let mut pointer = [30.0, 30.0];
        let mut opened = [false; 2];
        let mut has_thumbnail = false;
        let settings = Settings::default();
        for frame in 0..20 {
            let mut icon_center = [0.0; 2];
            context.io_mut().add_mouse_pos_event(pointer);
            let ui = context.frame();
            ui.window("context-menu-test")
                .position([0.0, 0.0], Condition::Always)
                .size([4750.0, 2350.0], Condition::Always)
                .build(|| {
                    if frame == 0 {
                        ui.open_popup(POPUP);
                    }
                    assert!(draw_popup(ui, &mut session, &settings, &mut target).is_none());
                    // SAFETY: the current frame owns the open popup and its live window.
                    unsafe {
                        let raw = &*sys::igGetCurrentContext();
                        assert!(raw.OpenPopupStack.Size > 0);
                        let popup = &*(*raw.OpenPopupStack.Data).Window;
                        icon_center = [
                            popup.WorkRect.Min.x + ui.text_line_height() * 0.5,
                            popup.DC.CursorPosPrevLine.y + ui.text_line_height() * 0.5,
                        ];
                        if frame >= 2 {
                            let text_width = ui.calc_text_size(&name)[0] + ui.calc_text_size(format!("[{path}]"))[0];
                            assert!(popup.WorkRect.Max.x - popup.WorkRect.Min.x >= text_width);
                            // Hover near the path/arrow, then near the icon: the whole row is a menu.
                            pointer = [
                                if frame < 10 {
                                    popup.WorkRect.Max.x - 20.0
                                } else {
                                    popup.WorkRect.Min.x + 8.0
                                },
                                popup.DC.CursorPosPrevLine.y + ui.text_line_height() * 0.5,
                            ];
                        }
                        let menu = sys::igFindWindowByName(c"Menu_00".as_ptr());
                        if !menu.is_null() && (*menu).Active {
                            assert_ne!((*menu).ID, 0);
                            assert_eq!((*menu).ParentWindow, std::ptr::from_ref(popup).cast_mut());
                            opened[usize::from(frame >= 10)] = true;
                        }
                    }
                });
            let data = context.render_legacy();
            assert!(data.valid());
            for list in data.draw_lists() {
                for command in list.commands() {
                    if let dear_imgui_rs::render::DrawCmd::Elements { count, cmd_params } = command
                        && Some(cmd_params.texture_id) == thumbnail_texture
                    {
                        has_thumbnail = true;
                        let indices = &list.idx_buffer()[cmd_params.idx_offset..cmd_params.idx_offset + count];
                        let (min, max) = indices
                            .iter()
                            .map(|&index| list.vtx_buffer()[index as usize + cmd_params.vtx_offset].pos)
                            .fold(([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]), |(min, max), pos| {
                                (
                                    std::array::from_fn(|axis| min[axis].min(pos[axis])),
                                    std::array::from_fn(|axis| max[axis].max(pos[axis])),
                                )
                            });
                        for axis in 0..2 {
                            assert!(
                                ((min[axis] + max[axis]) * 0.5 - icon_center[axis]).abs() < 0.01,
                                "thumbnail {min:?}..{max:?} is not centered at {icon_center:?}"
                            );
                        }
                    }
                }
            }
        }
        assert!(
            opened.into_iter().all(|opened| opened),
            "hover did not open both ends of the row for {path}"
        );
        assert_eq!(has_thumbnail, path.starts_with("/obj"));
    }
}

#[test]
fn replacement_search_matches_object_types_only() {
    let mut tree = ObjectTree::new();
    for path in [
        "/atom/movable",
        "/obj/machinery/light",
        "/obj/machinery/lightbulb",
        "/turf/open/light",
        "/area/light",
    ] {
        tree.register(&TreePath::parse(path), Location::default());
    }
    tree.get_mut(tree.roots().obj.unwrap()).unwrap().parent_type = Some(TreePath::parse("/atom/movable"));
    tree.resolve_parent_types();

    let paths = matching_type_paths_up_to_filtered(&tree, "LiGhT", 50, |path| {
        context_placement_group(&tree, path) == Some(0)
    });
    assert_eq!(
        paths.iter().map(ToString::to_string).collect::<Vec<_>>(),
        ["/obj/machinery/light", "/obj/machinery/lightbulb"]
    );
}
