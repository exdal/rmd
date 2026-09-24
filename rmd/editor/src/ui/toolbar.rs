use core::path::TreePath;

use dear_imgui_rs::{StyleColor, StyleVar, Ui};
use editor::{
    icons::materialdesignicons::{
        ICON_CIRCLE_SMALL,
        ICON_ERASER,
        ICON_EYEDROPPER,
        ICON_FORMAT_COLOR_FILL,
        ICON_MENU_DOWN,
        ICON_PENCIL,
        ICON_SELECT_DRAG,
        ICON_VECTOR_POLYLINE,
    },
    tool::{FillMode, Tool},
};

use super::{
    BlockSelectionOptions,
    MAX_CUSTOM_FILL_SEARCH_RESULTS,
    NewLevelDialog,
    OVERLAY_PADDING,
    OverlayRect,
    draw_overlay_underlay,
    draw_selection_mode_controls,
    draw_type_path_search,
};
use crate::{
    session::{LevelChange, Session},
    settings::{KeyBindings, KeybindAction},
};

pub(super) const DEFAULT_CUSTOM_FILL_BOUNDARY: &str = "/turf/closed/wall";

pub(super) fn request_level_change(
    session: &mut Session, delta: i32, dialog: &mut Option<NewLevelDialog>, remembered_type_path: &str,
) {
    if session.change_level(delta) != LevelChange::NewLevelRequested {
        return;
    }
    let Some(document) = session.state.active() else {
        return;
    };

    *dialog = Some(NewLevelDialog {
        document,
        type_path: remembered_type_path.to_owned(),
        error: None,
        open: true,
    });
}

fn draw_z_levels(ui: &Ui, session: &mut Session, dialog: &mut Option<NewLevelDialog>, remembered_type_path: &str) {
    let current = session.z();
    let button_size = ui.frame_height();
    ui.align_text_to_frame_padding();
    ui.text("Z:");
    ui.same_line();

    let down = {
        let _disabled = ui.begin_disabled_with_cond(!session.can_change_level(-1));

        ui.button_with_size("<##z-level-down", [button_size, button_size])
    };
    ui.same_line();
    ui.align_text_to_frame_padding();
    ui.text(current.to_string());
    ui.same_line();
    let up = {
        let _disabled = ui.begin_disabled_with_cond(!session.can_change_level(1));

        ui.button_with_size(">##z-level-up", [button_size, button_size])
    };

    if down {
        request_level_change(session, -1, dialog, remembered_type_path);
    } else if up {
        request_level_change(session, 1, dialog, remembered_type_path);
    }
}

pub(super) struct TopOverlayState<'a> {
    pub(super) keybindings: KeyBindings,
    pub(super) selection_busy: bool,
    pub(super) block_selection_options: &'a mut BlockSelectionOptions,
    pub(super) fill_mode: &'a mut FillMode,
    pub(super) custom_fill_boundaries: &'a mut Vec<TreePath>,
    pub(super) custom_fill_search: &'a mut String,
    pub(super) new_level_dialog: &'a mut Option<NewLevelDialog>,
    pub(super) new_level_type_path: &'a str,
}

pub(super) fn draw_top_overlay(ui: &Ui, session: &mut Session, bounds: OverlayRect, state: TopOverlayState<'_>) {
    let TopOverlayState {
        keybindings,
        selection_busy,
        block_selection_options,
        fill_mode,
        custom_fill_boundaries,
        custom_fill_search,
        new_level_dialog,
        new_level_type_path,
    } = state;
    draw_overlay_underlay(ui, bounds);

    ui.set_cursor_screen_pos([bounds.min[0] + OVERLAY_PADDING, bounds.min[1] + OVERLAY_PADDING]);

    draw_tool_button(ui, session, Tool::Place, ICON_PENCIL);
    ui.same_line();
    draw_tool_button(ui, session, Tool::Select, ICON_EYEDROPPER);
    if session.node_tool_available() {
        ui.same_line();
        draw_tool_button(ui, session, Tool::Node, ICON_VECTOR_POLYLINE);
    }
    ui.same_line();
    draw_block_select_tool_button(ui, session, block_selection_options, keybindings, selection_busy);
    ui.same_line();
    draw_tool_button(ui, session, Tool::Delete, ICON_ERASER);
    ui.same_line();
    let tools_end = draw_fill_tool_button(
        ui,
        session,
        fill_mode,
        custom_fill_boundaries,
        custom_fill_search,
        keybindings,
    );

    let levels_width = z_level_width(ui, session.level_count());
    let levels_x = (bounds.max[0] - OVERLAY_PADDING - levels_width).max(tools_end + OVERLAY_PADDING);
    ui.set_cursor_screen_pos([levels_x, bounds.min[1] + OVERLAY_PADDING]);
    draw_z_levels(ui, session, new_level_dialog, new_level_type_path);
}

fn draw_block_select_tool_button(
    ui: &Ui, session: &mut Session, options: &mut BlockSelectionOptions, keybindings: KeyBindings, selection_busy: bool,
) {
    const POPUP: &str = "block-selection-options-popup";

    let active_color = ui.style_color(StyleColor::PlotHistogramHovered);
    let active = (session.tool() == Tool::BlockSelect).then(|| ui.push_style_color(StyleColor::Button, active_color));
    let spacing = ui.clone_style().item_spacing();
    let connected = ui.push_style_var(StyleVar::ItemSpacing([0.0, spacing[1]]));

    if ui.button(format!("{ICON_SELECT_DRAG}##block-select-tool")) {
        session.set_tool(Tool::BlockSelect);
    }
    let separator_x = ui.item_rect_max()[0];
    let separator_min_y = ui.item_rect_min()[1];
    let separator_max_y = ui.item_rect_max()[1];
    ui.set_item_tooltip(format!(
        "Rectangle selection ({})\nDrag: {} {ICON_CIRCLE_SMALL} Hold Shift while drawing: Border\nShift+gizmo or edge \
         handles: resize",
        keybindings.get(KeybindAction::BlockSelectTool).label(ui),
        options.label()
    ));
    ui.same_line();

    let button_size = ui.frame_height();
    let arrow_width = (button_size * 0.65)
        .ceil()
        .max((ui.calc_text_size(ICON_MENU_DOWN.to_string())[0] + 2.0).ceil());
    let frame_padding = ui.clone_style().frame_padding();
    let arrow_clicked = {
        let _padding = ui.push_style_var(StyleVar::FramePadding([0.0, frame_padding[1]]));
        let _alignment = ui.push_style_var(StyleVar::ButtonTextAlign([0.5, 0.5]));
        ui.button_with_size(
            format!("{ICON_MENU_DOWN}##block-fill-options"),
            [arrow_width, button_size],
        )
    };
    if arrow_clicked {
        ui.open_popup(POPUP);
    }
    ui.set_item_tooltip(format!("Block selection: {}", options.label()));
    ui.get_window_draw_list().add_line_v(
        separator_x,
        separator_min_y + frame_padding[1],
        separator_max_y - frame_padding[1],
        [0.0, 0.0, 0.0, 0.7],
        1.0,
    );

    drop(connected);
    drop(active);

    if let Some(_popup) = ui.begin_popup(POPUP) {
        ui.with_disabled_if(selection_busy, || draw_selection_mode_controls(ui, session, options));
        ui.text("Hold Shift while drawing for Border.");
        ui.text("Drag edge handles or Shift+gizmo to resize.");
        if selection_busy {
            ui.text("Finish or cancel the current gesture to change mode.");
        }
    }
}

fn draw_fill_tool_button(
    ui: &Ui, session: &mut Session, fill_mode: &mut FillMode, custom_fill_boundaries: &mut Vec<TreePath>,
    custom_fill_search: &mut String, keybindings: KeyBindings,
) -> f32 {
    const POPUP: &str = "fill-mode-popup";

    let active_color = ui.style_color(StyleColor::PlotHistogramHovered);
    let active = (session.tool() == Tool::Fill).then(|| ui.push_style_color(StyleColor::Button, active_color));
    let spacing = ui.clone_style().item_spacing();
    let connected = ui.push_style_var(StyleVar::ItemSpacing([0.0, spacing[1]]));

    if ui.button(format!("{ICON_FORMAT_COLOR_FILL}##fill-tool")) {
        session.set_tool(Tool::Fill);
    }
    let separator_x = ui.item_rect_max()[0];
    let separator_min_y = ui.item_rect_min()[1];
    let separator_max_y = ui.item_rect_max()[1];
    ui.set_item_tooltip(format!(
        "Bucket ({}) {ICON_CIRCLE_SMALL} {}\n{}",
        keybindings.get(KeybindAction::FillTool).label(ui),
        fill_mode.label(),
        if session.selection().is_some() {
            "Fills connected selected tiles. Clear selection to fill elsewhere"
        } else {
            "Click to fill a connected region"
        }
    ));
    ui.same_line();

    let button_size = ui.frame_height();
    let arrow_width = (button_size * 0.65)
        .ceil()
        .max((ui.calc_text_size(ICON_MENU_DOWN.to_string())[0] + 2.0).ceil());
    let frame_padding = ui.clone_style().frame_padding();
    let arrow_clicked = {
        let _padding = ui.push_style_var(StyleVar::FramePadding([0.0, frame_padding[1]]));
        let _alignment = ui.push_style_var(StyleVar::ButtonTextAlign([0.5, 0.5]));
        ui.button_with_size(format!("{ICON_MENU_DOWN}##fill-mode"), [arrow_width, button_size])
    };
    if arrow_clicked {
        ui.open_popup(POPUP);
    }
    let tools_end = ui.item_rect_max()[0];
    ui.set_item_tooltip(format!("Fill mode: {}", fill_mode.label()));
    ui.get_window_draw_list().add_line_v(
        separator_x,
        separator_min_y + frame_padding[1],
        separator_max_y - frame_padding[1],
        [0.0, 0.0, 0.0, 0.7],
        1.0,
    );

    drop(connected);
    drop(active);

    if let Some(_popup) = ui.begin_popup(POPUP) {
        for mode in [FillMode::Wall, FillMode::EntireArea, FillMode::Custom] {
            if ui.menu_item_enabled_selected_no_shortcut(mode.label(), *fill_mode == mode, true) {
                *fill_mode = mode;
                ui.close_current_popup();
            }
        }

        ui.separator();
        let selected = session.palette().map(|prefab| prefab.path.clone());
        let can_add = selected
            .as_ref()
            .is_some_and(|path| !custom_fill_boundaries.contains(path));
        let add_label = selected
            .as_ref()
            .map_or_else(|| "Add selected type".to_owned(), |path| format!("Add {path}"));
        if ui.menu_item_enabled_selected_no_shortcut(add_label, false, can_add)
            && let Some(path) = selected
        {
            custom_fill_boundaries.push(path);
            *fill_mode = FillMode::Custom;
        }

        let mut searched_path = None;
        if let Some(_menu) = ui.begin_menu("Search type paths") {
            searched_path = draw_type_path_search(
                ui,
                session.tree(),
                custom_fill_search,
                "##custom-fill-boundary-search",
                MAX_CUSTOM_FILL_SEARCH_RESULTS,
                |_, _| true,
                |path| !custom_fill_boundaries.contains(path),
            );
        }

        if let Some(path) = searched_path {
            custom_fill_boundaries.push(path);
            *fill_mode = FillMode::Custom;
        }

        let mut remove = None;
        if let Some(_menu) = ui.begin_menu_with_enabled(
            format!("Remove boundary ({})", custom_fill_boundaries.len()),
            !custom_fill_boundaries.is_empty(),
        ) {
            for (index, path) in custom_fill_boundaries.iter().enumerate() {
                if ui.menu_item(format!("{path}##custom-fill-boundary-{index}")) {
                    remove = Some(index);
                }
            }
        }
        if let Some(index) = remove {
            custom_fill_boundaries.remove(index);
        }

        let default = TreePath::parse(DEFAULT_CUSTOM_FILL_BOUNDARY);
        let is_default = custom_fill_boundaries.as_slice() == [default.clone()];
        if ui.menu_item_enabled_selected_no_shortcut("Reset custom boundaries", false, !is_default) {
            custom_fill_boundaries.clear();
            custom_fill_boundaries.push(default);
        }
    }

    tools_end
}

fn draw_tool_button(ui: &Ui, session: &mut Session, tool: Tool, icon: char) {
    let color = match tool {
        Tool::Delete => [1.0, 0.0, 0.0, 1.0],
        _ => ui.style_color(StyleColor::PlotHistogramHovered),
    };

    let _color = (session.tool() == tool).then(|| ui.push_style_color(StyleColor::Button, color));
    let clicked = ui.button(icon.to_string());
    ui.set_item_tooltip(match tool {
        Tool::BlockSelect => "Block Select",
        Tool::Node => {
            "Node tool\nDouble-click a node to select its network; drag a handle to connect.\nRight-click a connection \
             or isolated node to delete it; Shift+right-click for the map menu."
        },
        _ => tool.label(),
    });
    if clicked {
        session.set_tool(tool);
    }
}

fn z_level_width(ui: &Ui, levels: u32) -> f32 {
    let style = ui.clone_style();
    let item_spacing = style.item_spacing()[0];
    let button = ui.frame_height();
    let label = ui.calc_text_size("Z:")[0];
    let level = ui.calc_text_size(levels.to_string())[0];

    label + item_spacing * 3.0 + button * 2.0 + level
}
