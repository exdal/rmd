use core::{path::TreePath, types::Identifier};

use dear_imgui_rs::{MouseButton, Ui, WindowHoveredFlags};
use dmm::{Coord, Prefab, PrefabInstanceId};
use editor::{blame::BlameCell, conflict::Side, document::DocumentId};
use objtree::ObjectTree;

use super::{SelectionTransform, Session, Tool, common::fit_icon, draw_type_path_search, inspector::SimilarMatchKind};
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
        .filter(|(_, location)| location.coord == coord)
        .fold((0.0_f32, 0.0_f32), |(name_width, path_width), (prefab, _)| {
            let name = display_name(session.tree(), prefab);
            (
                name_width.max(ui.calc_text_size(&name)[0]),
                path_width.max(ui.calc_text_size(format!("[{}]", prefab.path))[0]),
            )
        });
    let icon_width = ui.calc_text_size("    ")[0];
    let path_column = icon_width + name_width + 20.0;
    let label_width = path_column + path_width + 36.0;
    let space_width = ui.calc_text_size(" ")[0].max(1.0);
    for instance in target.atoms.iter().copied() {
        let Some((prefab, location)) = session
            .state
            .active_document()
            .and_then(|document| document.prefab_instance(instance))
        else {
            continue;
        };
        if location.coord != coord {
            continue;
        }
        let prefab = prefab.clone();
        let name = display_name(session.tree(), &prefab);
        let path_text = format!("[{}]", prefab.path);
        let visible = format!("    {name}");
        let padding = ((label_width - ui.calc_text_size(&visible)[0]) / space_width).ceil() as usize;
        let label = format!("{visible}{}##atom-{}", " ".repeat(padding), instance.get());
        let thumbnail = session.prefab_thumbnail(&prefab);
        let editable = session.can_edit_instance(instance);
        let reorderable = editable
            && session
                .tree()
                .is_some_and(|tree| context_placement_group(tree, &prefab.path) == Some(0));
        let draw = ui.get_window_draw_list();
        let submenu = ui.begin_menu(&label);
        let row_min = ui.item_rect_min();
        let row_max = ui.item_rect_max();
        let extent = ui.text_line_height().min(row_max[1] - row_min[1]);
        if let Some(thumbnail) = thumbnail {
            let size = fit_icon(thumbnail.texture.width, thumbnail.texture.height, extent);
            let min = [
                row_min[0] + (extent - size[0]) * 0.5,
                row_min[1] + (extent - size[1]) * 0.5,
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
            draw.add_text(
                row_min,
                ui.style_color(dear_imgui_rs::StyleColor::TextDisabled),
                super::ICON_IMAGE_BROKEN.to_string(),
            );
        }
        draw.add_text(
            [
                row_min[0] + path_column,
                row_min[1] + (row_max[1] - row_min[1] - ui.text_line_height()) * 0.5,
            ],
            ui.style_color(dear_imgui_rs::StyleColor::Text),
            path_text,
        );
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
    let name = Identifier::from("name");
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
    use core::{location::Location, path::TreePath};

    use objtree::ObjectTree;

    use crate::{session::context_placement_group, ui::search::matching_type_paths_up_to_filtered};

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
