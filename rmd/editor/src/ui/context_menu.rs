use core::{path::TreePath, types::Identifier, vars};

use dear_imgui_rs::{MouseButton, Ui, WindowHoveredFlags};
use dmm::{Coord, Prefab, PrefabInstanceId};
use editor::{blame::BlameCell, conflict::Side, document::DocumentId};
use objtree::ObjectTree;

use super::{SelectionTransform, Session, Tool, common::fit_icon, draw_type_path_search, find::SimilarMatchKind};
use crate::session::context_placement_group;

pub(super) const POPUP: &str = "Map context##map-context";

#[derive(Clone)]
pub(super) enum NodeContext {
    Standalone(Coord),
    Connection(Vec<Coord>),
    Pick(Coord, [u32; 2]),
}

#[derive(Clone)]
pub(super) struct Target {
    pub(super) document: DocumentId,
    pub(super) coord: Coord,
    pub(super) atoms: Vec<PrefabInstanceId>,
    pub(super) node: Option<NodeContext>,
    pub(super) block_selected: bool,
    pub(super) replace_for: Option<PrefabInstanceId>,
    pub(super) replace_query: String,
}

pub(super) enum Action {
    Conflict(Vec<Coord>, Side),
    BlameCopy(String),
    BlamePin(u32),
    BlameRun,
    Restore(Vec<Coord>),
    Undo,
    Redo,
    Copy(Coord),
    Paste(Coord),
    Cut(Coord),
    Delete(Coord),
    Select(PrefabInstanceId),
    DeleteAtom(PrefabInstanceId),
    Reorder(PrefabInstanceId, bool),
    Reset(PrefabInstanceId),
    Replace(PrefabInstanceId, TreePath),
    Search(PrefabInstanceId, SimilarMatchKind),
    Node(NodeContext),
    Mirror(SelectionTransform),
}

pub(super) fn draw_popup(
    ui: &Ui, session: &mut Session, settings: &super::Settings, target: &mut Target,
) -> Option<Action> {
    let _popup = ui.begin_popup(POPUP)?;
    if session.state.active() != Some(target.document) {
        return None;
    }

    if ui.is_mouse_clicked(MouseButton::Middle) && !ui.is_window_hovered_with_flags(WindowHoveredFlags::CHILD_WINDOWS) {
        ui.close_current_popup();

        return None;
    }

    let coord = target.coord;
    ui.text_disabled(format!("X: {}, Y: {}, Z: {}", coord.x, coord.y, coord.z));
    // every section starts with its own separator so none of them double up
    let mut action = None;
    if let Some(conflicts) = session
        .git_state(target.document)
        .and_then(|git| git.conflicts.as_ref())
        && conflicts.conflict_at(coord).is_some()
    {
        ui.separator();
        for side in [Side::Ours, Side::Theirs] {
            if ui.menu_item(format!("Take {} for this tile", conflicts.side_label(side))) {
                action = Some(Action::Conflict(vec![coord], side));
            }
        }

        if let Some(region) = session
            .conflict_regions(target.document, Some(coord.z))
            .iter()
            .find(|region| region.tiles.contains(&coord))
        {
            for side in [Side::Ours, Side::Theirs] {
                if ui.menu_item(format!("Take {} for region", conflicts.side_label(side))) {
                    action = Some(Action::Conflict(region.tiles.clone(), side));
                }
            }
        }

        if target.block_selected
            && let Some(selection) = session.selection()
        {
            let coords = session
                .selection_mode()
                .tiles(selection)
                .filter(|coord| conflicts.conflict_at(*coord).is_some())
                .collect::<Vec<_>>();
            if !coords.is_empty() {
                for side in [Side::Ours, Side::Theirs] {
                    if ui.menu_item(format!("Take {} for selection", conflicts.side_label(side))) {
                        action = Some(Action::Conflict(coords.clone(), side));
                    }
                }
            }
        }
    }

    if let Some(state) = session
        .git_state(target.document)
        .and_then(|git| git.diff.as_ref())
        .filter(|state| state.restorable())
        && let Some(diff) = session.diff(target.document)
    {
        let mut coords = vec![coord];
        if target.block_selected
            && let Some(selection) = session.selection()
        {
            coords = session.selection_mode().tiles(selection).collect();
        }
        coords.retain(|coord| diff.at(*coord).is_some());

        if !coords.is_empty() {
            ui.separator();
            let count = match coords.len() {
                1 => String::new(),
                count => format!(" ({count} tiles)"),
            };
            if ui.menu_item(format!("Restore from {}{count}", state.from.label())) {
                action = Some(Action::Restore(coords));
            }
        }
    }

    if let Some(git) = session.git_state(target.document) {
        ui.separator();
        if let Some(_menu) = ui.begin_menu("Blame") {
            if let Some((cell, changed)) = session.blame_at(target.document, coord) {
                match cell {
                    BlameCell::Commit(index, commit) if !changed => {
                        ui.text_disabled(format!("{} {}", commit.short, commit.summary));
                        if ui.menu_item("Copy hash") {
                            action = Some(Action::BlameCopy(commit.hash.clone()));
                        }
                        if ui.menu_item("Pin commit tiles") {
                            action = Some(Action::BlamePin(index));
                        }
                    },
                    BlameCell::Boundary if !changed => ui.text_disabled(git.blame.as_ref().map_or_else(
                        || String::from("Older than blame depth"),
                        |blame| blame.result.boundary_label(),
                    )),
                    _ => ui.text_disabled(editor::blame::pending_note(cell, changed).unwrap_or_default()),
                }
            } else if git.blame.is_none() && ui.menu_item("Run blame") {
                action = Some(Action::BlameRun);
            }
        }
    }

    if let Some(node) = target.node.as_ref() {
        ui.separator();
        let label = match node {
            NodeContext::Standalone(_) => "Delete standalone node",
            NodeContext::Connection(_) => "Delete connection",
            NodeContext::Pick(..) => "Delete node or connection",
        };

        if ui.menu_item_enabled_selected_no_shortcut(
            label,
            false,
            session.tool() == Tool::Node && !session.node_dragging(),
        ) {
            action = Some(Action::Node(node.clone()));
        }
    }

    ui.separator();
    let undo = session.undo_label();
    let undo_label = undo.map_or_else(|| "Undo".to_owned(), |label| format!("Undo {label}"));
    if ui.menu_item_enabled_selected_with_shortcut(
        undo_label,
        settings.keybindings.get(super::KeybindAction::Undo).label(ui),
        false,
        undo.is_some(),
    ) {
        action = Some(Action::Undo);
    }
    let redo = session.redo_label();
    let redo_label = redo.map_or_else(|| "Redo".to_owned(), |label| format!("Redo {label}"));
    if ui.menu_item_enabled_selected_with_shortcut(
        redo_label,
        settings.keybindings.get(super::KeybindAction::Redo).label(ui),
        false,
        redo.is_some(),
    ) {
        action = Some(Action::Redo);
    }
    ui.separator();
    let tile_exists = session.map().is_some_and(|map| map.tile_at(coord).is_some());
    if ui.menu_item_enabled_selected_with_shortcut(
        "Copy",
        settings.keybindings.get(super::KeybindAction::Copy).label(ui),
        false,
        tile_exists,
    ) {
        action = Some(Action::Copy(coord));
    }
    let can_paste = session.can_paste_clipboard(coord, super::SelectionRotation::Original);
    if ui.menu_item_enabled_selected_with_shortcut(
        "Paste",
        settings.keybindings.get(super::KeybindAction::Paste).label(ui),
        false,
        can_paste,
    ) {
        action = Some(Action::Paste(coord));
    }
    let can_clear = session.can_clear_tile(coord);
    if ui.menu_item_enabled_selected_no_shortcut("Cut", false, can_clear) {
        action = Some(Action::Cut(coord));
    }
    if ui.menu_item_enabled_selected_no_shortcut("Delete", false, can_clear) {
        action = Some(Action::Delete(coord));
    }
    ui.separator();
    let (name_width, path_width) = target
        .atoms
        .iter()
        .copied()
        .filter_map(|instance| session.state.active_document()?.prefab_instance(instance))
        .filter(|instance| instance.location.coord == coord)
        .fold((0.0_f32, 0.0_f32), |(name_width, path_width), instance| {
            let prefab = instance.prefab();
            let name = display_name(session.tree(), prefab);
            (
                name_width.max(ui.calc_text_size(&name)[0]),
                path_width.max(ui.calc_text_size(format!("[{}]", prefab.path))[0]),
            )
        });
    let icon_extent = ui.text_line_height();
    let icon_width = icon_extent + ui.clone_style().item_inner_spacing()[0];
    let path_column = icon_width + name_width + 20.0;
    let label_width = path_column + path_width + 36.0;
    let cursor = ui.cursor_pos();
    ui.dummy([label_width, 0.0]);
    ui.set_cursor_pos(cursor);
    for instance in target.atoms.iter().copied() {
        let Some(placed) = session
            .state
            .active_document()
            .and_then(|document| document.prefab_instance(instance))
        else {
            continue;
        };

        if placed.location.coord != coord {
            continue;
        }

        let prefab = placed.prefab().clone();
        let name = display_name(session.tree(), &prefab);
        let path_text = format!("[{}]", prefab.path);
        let label = format!("##atom-{}", instance.get());
        let thumbnail = session.prefab_thumbnail(&prefab);
        let editable = session.can_edit_instance(instance);
        let reorderable = editable
            && session
                .tree()
                .is_some_and(|tree| context_placement_group(tree, &prefab.path) == Some(0));
        let origin = ui.cursor_screen_pos();
        let submenu = ui.begin_menu(&label);
        if let Some(_menu) = submenu {
            if ui.menu_item_enabled_selected_no_shortcut("Move to Top", false, reorderable) {
                action = Some(Action::Reorder(instance, true));
            }
            if ui.menu_item_enabled_selected_no_shortcut("Move to Bottom", false, reorderable) {
                action = Some(Action::Reorder(instance, false));
            }
            ui.separator();
            if ui.menu_item("Select") {
                action = Some(Action::Select(instance));
            }
            if ui.menu_item_enabled_selected_no_shortcut("Delete", false, editable) {
                action = Some(Action::DeleteAtom(instance));
            }
            if let Some(_replace) = ui.begin_menu_with_enabled("Replace", editable) {
                if target.replace_for != Some(instance) {
                    target.replace_for = Some(instance);
                    target.replace_query.clear();
                }
                if let Some(selected) = session.palette()
                    && let Some(tree) = session.tree()
                    && let Some(kind) = context_placement_group(tree, &prefab.path)
                    && context_placement_group(tree, &selected.path) == Some(kind)
                {
                    let enabled = selected.path != prefab.path || !prefab.vars.is_empty();
                    if ui.menu_item_enabled_selected_no_shortcut(
                        format!("Use selected type: {}", selected.path),
                        false,
                        enabled,
                    ) {
                        action = Some(Action::Replace(instance, selected.path.clone()));
                    }
                    ui.separator();
                }
                let kind = session
                    .tree()
                    .and_then(|tree| context_placement_group(tree, &prefab.path));
                if let Some(path) = draw_type_path_search(
                    ui,
                    session.tree(),
                    &mut target.replace_query,
                    "##replace-type-search",
                    50,
                    |tree, path| kind.is_some_and(|kind| context_placement_group(tree, path) == Some(kind)),
                    |path| *path != prefab.path || !prefab.vars.is_empty(),
                ) {
                    action = Some(Action::Replace(instance, path));
                }
            }
            if ui.menu_item_enabled_selected_no_shortcut("Reset to Default", false, editable && !prefab.vars.is_empty())
            {
                action = Some(Action::Reset(instance));
            }
            ui.separator();
            if ui.menu_item("Search by Type") {
                action = Some(Action::Search(instance, SimilarMatchKind::Type));
            }
            if ui.menu_item("Search by Prefab ID") {
                action = Some(Action::Search(instance, SimilarMatchKind::Prefab));
            }
        }
        let draw = ui.get_window_draw_list();
        if let Some(thumbnail) = thumbnail {
            let size = fit_icon(thumbnail.texture.width, thumbnail.texture.height, icon_extent);
            let min = [
                origin[0] + (icon_extent - size[0]) * 0.5,
                origin[1] + (icon_extent - size[1]) * 0.5,
            ];
            draw.add_image(
                super::Renderer::sprite_texture(thumbnail.texture.index),
                min,
                [min[0] + size[0], min[1] + size[1]],
                thumbnail.uv0,
                thumbnail.uv1,
                thumbnail.tint,
            );
        } else {
            let icon = super::ICON_IMAGE_BROKEN.to_string();
            let size = ui.calc_text_size(&icon);
            draw.add_text(
                [
                    origin[0] + (icon_extent - size[0]) * 0.5,
                    origin[1] + (icon_extent - size[1]) * 0.5,
                ],
                ui.style_color(dear_imgui_rs::StyleColor::TextDisabled),
                icon,
            );
        }
        let text_color = ui.style_color(dear_imgui_rs::StyleColor::Text);
        draw.add_text([origin[0] + icon_width, origin[1]], text_color, name);
        draw.add_text([origin[0] + path_column, origin[1]], text_color, path_text);
    }
    if target.block_selected {
        ui.separator();
        let mode = session.selection_mode();
        for (label, transform) in [
            ("Mirror selection horizontally", SelectionTransform::MirrorHorizontal),
            ("Mirror selection vertically", SelectionTransform::MirrorVertical),
        ] {
            if ui.menu_item_enabled_selected_no_shortcut(
                label,
                false,
                session.can_transform_selected_block_with_mode(transform, mode),
            ) {
                action = Some(Action::Mirror(transform));
            }
        }
    }
    action
}

fn display_name(tree: Option<&ObjectTree>, prefab: &Prefab) -> String {
    let name = Identifier::from(vars::NAME);
    prefab
        .var(&name)
        .or_else(|| {
            tree.and_then(|tree| {
                tree.id_of(&prefab.path)
                    .and_then(|id| tree.var_inherited(id, &name))
                    .map(|var| &var.value)
            })
        })
        .and_then(|value| value.as_text())
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| prefab.path.to_string().rsplit('/').next().unwrap_or("atom").to_owned())
}

#[cfg(test)]
mod tests {
    use core::{location::Location, path::TreePath, types::Value};
    use std::path::PathBuf;

    use dear_imgui_rs::{Condition, sys};
    use dmm::{Coord, Map, Prefab, Size};
    use objtree::ObjectTree;

    use super::{POPUP, Target, draw_popup};
    use crate::{
        loader::LoadedMap,
        session::{Session, context_placement_group},
        settings::Settings,
        ui::{IMGUI_CONTEXT, fixtures::rectangle_context, search::matching_type_paths_up_to_filtered},
    };

    #[test]
    fn hovering_atom_rows_opens_submenus_without_truncating_names_or_hit_areas() {
        let _guard = IMGUI_CONTEXT.lock().unwrap();
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
                block_selected: false,
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
                                let text_width =
                                    ui.calc_text_size(&name)[0] + ui.calc_text_size(format!("[{path}]"))[0];
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
}
