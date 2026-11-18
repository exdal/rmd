use base::{coord::Coord, path::TreePath, types::Identifier, vars};

use dear_imgui_rs::{Key, MouseButton, PopupToken, Ui, WindowHoveredFlags};
use objtree::ObjectTree;
use prefab::{Prefab, PrefabInstanceId};

use super::edit::EditCommand;
use crate::{
    document::DocumentId,
    git::{blame::BlameCell, conflict::Side},
    object_tree::panel::type_search::draw_type_path_search,
    palette::panel::thumbnail::draw_thumbnail,
    search::panel::SimilarMatchKind,
    session::Session,
    settings::{Settings, keybind::KeybindAction},
    tool::{SelectionRotation, SelectionTransform, Tool, session::context_placement_group},
};

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
    pub(super) region_selected: bool,
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
    Selection(EditCommand),
    Select(PrefabInstanceId),
    DeleteAtom(PrefabInstanceId),
    Reorder(PrefabInstanceId, bool),
    Reset(PrefabInstanceId),
    Replace(PrefabInstanceId, TreePath),
    Search(PrefabInstanceId, SimilarMatchKind),
    Node(NodeContext),
    Mirror(SelectionTransform),
}

pub(crate) fn begin_map_popup<'ui>(
    ui: &'ui Ui, session: &Session, name: &str, document: DocumentId,
) -> Option<PopupToken<'ui>> {
    let popup = ui.begin_popup(name)?;
    if session.state.active() != Some(document) {
        return None;
    }

    let is_middle_clicked_outside =
        ui.is_mouse_clicked(MouseButton::Middle) && !ui.is_window_hovered_with_flags(WindowHoveredFlags::CHILD_WINDOWS);
    if is_middle_clicked_outside || ui.is_key_pressed(Key::Escape) {
        ui.close_current_popup();

        return None;
    }

    Some(popup)
}

pub(super) fn draw_popup(ui: &Ui, session: &mut Session, settings: &Settings, target: &mut Target) -> Option<Action> {
    let _popup = begin_map_popup(ui, session, POPUP, target.document)?;
    let coord = target.coord;
    ui.text_disabled(format!("X: {}, Y: {}, Z: {}", coord.x, coord.y, coord.z));
    // every section starts with its own separator so none of them double up
    let mut action = None;
    action = draw_conflict_actions(ui, session, target, coord, action);

    action = draw_restore_action(ui, session, target, coord, action);

    action = draw_blame_menu(ui, session, target, coord, action);

    action = draw_node_action(ui, session, target, action);

    action = draw_history_actions(ui, session, settings, action);
    action = draw_clipboard_actions(ui, session, settings, target, coord, action);

    action = draw_mirror_actions(ui, session, target, action);

    action = draw_atom_menus(ui, session, target, coord, action);

    action
}

fn draw_conflict_actions(
    ui: &Ui, session: &Session, target: &Target, coord: Coord, mut action: Option<Action>,
) -> Option<Action> {
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

        if let Some(hunk) = session
            .conflict_hunks(target.document, Some(coord.z))
            .iter()
            .find(|hunk| hunk.tiles.contains(&coord))
        {
            for side in [Side::Ours, Side::Theirs] {
                if ui.menu_item(format!("Take {} for hunk", conflicts.side_label(side))) {
                    action = Some(Action::Conflict(hunk.tiles.clone(), side));
                }
            }
        }

        if target.region_selected
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
    action
}

fn draw_restore_action(
    ui: &Ui, session: &Session, target: &Target, coord: Coord, mut action: Option<Action>,
) -> Option<Action> {
    if let Some(state) = session
        .git_state(target.document)
        .and_then(|git| git.diff.as_ref())
        .filter(|state| state.restorable())
        && let Some(diff) = session.diff(target.document)
    {
        let mut coords = vec![coord];
        if target.region_selected
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
    action
}

fn draw_blame_menu(
    ui: &Ui, session: &Session, target: &Target, coord: Coord, mut action: Option<Action>,
) -> Option<Action> {
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
                    _ => ui.text_disabled(crate::git::blame::pending_note(cell, changed).unwrap_or_default()),
                }
            } else if git.blame.is_none() && ui.menu_item("Run blame") {
                action = Some(Action::BlameRun);
            }
        }
    }
    action
}

fn draw_node_action(ui: &Ui, session: &Session, target: &Target, mut action: Option<Action>) -> Option<Action> {
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
    action
}

fn draw_history_actions(ui: &Ui, session: &Session, settings: &Settings, mut action: Option<Action>) -> Option<Action> {
    ui.separator();
    let undo = session.undo_label();
    let undo_label = undo.map_or_else(|| "Undo".to_owned(), |label| format!("Undo {label}"));
    if ui.menu_item_enabled_selected_with_shortcut(
        undo_label,
        settings.keybindings.get(KeybindAction::Undo).label(ui),
        false,
        undo.is_some(),
    ) {
        action = Some(Action::Undo);
    }
    let redo = session.redo_label();
    let redo_label = redo.map_or_else(|| "Redo".to_owned(), |label| format!("Redo {label}"));
    if ui.menu_item_enabled_selected_with_shortcut(
        redo_label,
        settings.keybindings.get(KeybindAction::Redo).label(ui),
        false,
        redo.is_some(),
    ) {
        action = Some(Action::Redo);
    }
    action
}

fn draw_clipboard_actions(
    ui: &Ui, session: &Session, settings: &Settings, target: &Target, coord: Coord, mut action: Option<Action>,
) -> Option<Action> {
    ui.separator();
    let is_picking = !target.region_selected && session.tool() == Tool::Pick && session.picked_instance().is_some();
    let is_group_command = target.region_selected || is_picking;
    let can_copy = is_group_command || session.map().is_some_and(|map| map.tile_at(coord).is_some());
    if ui.menu_item_enabled_selected_with_shortcut(
        "Copy",
        settings.keybindings.get(KeybindAction::Copy).label(ui),
        false,
        can_copy,
    ) {
        action = Some(if is_group_command {
            Action::Selection(EditCommand::Copy)
        } else {
            Action::Copy(coord)
        });
    }
    let can_paste = session.can_paste_clipboard(coord, SelectionRotation::Original);
    if ui.menu_item_enabled_selected_with_shortcut(
        "Paste",
        settings.keybindings.get(KeybindAction::Paste).label(ui),
        false,
        can_paste,
    ) {
        action = Some(Action::Paste(coord));
    }
    let can_clear = if is_picking {
        session
            .picked_instances()
            .into_iter()
            .any(|id| session.can_edit_instance(id))
    } else if target.region_selected {
        session.can_clear_selection()
    } else {
        session.can_clear_tile(coord)
    };
    if ui.menu_item_enabled_selected_no_shortcut("Cut", false, can_clear) {
        action = Some(if is_group_command {
            Action::Selection(EditCommand::Cut)
        } else {
            Action::Cut(coord)
        });
    }
    if ui.menu_item_enabled_selected_no_shortcut("Delete", false, can_clear) {
        action = Some(if is_group_command {
            Action::Selection(EditCommand::Delete)
        } else {
            Action::Delete(coord)
        });
    }
    action
}

fn draw_mirror_actions(ui: &Ui, session: &Session, target: &Target, mut action: Option<Action>) -> Option<Action> {
    if target.region_selected {
        ui.separator();

        let mode = session.selection_mode();
        for (label, transform) in [
            ("Mirror selection horizontally", SelectionTransform::MirrorHorizontal),
            ("Mirror selection vertically", SelectionTransform::MirrorVertical),
        ] {
            if ui.menu_item_enabled_selected_no_shortcut(
                label,
                false,
                session.can_transform_selected_region_with_mode(transform, mode),
            ) {
                action = Some(Action::Mirror(transform));
            }
        }
    }
    action
}

fn draw_atom_menus(
    ui: &Ui, session: &mut Session, target: &mut Target, coord: Coord, mut action: Option<Action>,
) -> Option<Action> {
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
        draw_thumbnail(ui, thumbnail, origin, icon_extent, None);
        let draw = ui.get_window_draw_list();
        let text_color = ui.style_color(dear_imgui_rs::StyleColor::Text);
        draw.add_text([origin[0] + icon_width, origin[1]], text_color, name);
        draw.add_text([origin[0] + path_column, origin[1]], text_color, path_text);
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
mod tests;
