use dear_imgui_rs::{
    DockLayout,
    DockLayoutApply,
    DockNodeFlags,
    DockSplit,
    DockspaceError,
    Key,
    MouseButton,
    Ui,
    WindowKey,
    WindowKeyError,
};
use objtree::{ObjectTree, TypeId};
use render::{Renderer, ViewportInteraction};

use crate::{camera::Controller, session::Session};

/// How far down the "Blur below" menu goes. The option itself takes any depth.
const MAX_UNDERLAY_DEPTH: u32 = 3;

pub struct UiOutput {
    pub exit: bool,
    pub viewport: (u32, u32),
    pub interaction: ViewportInteraction,
}

pub struct UiState {
    object_tree: WindowKey,
    viewport_window: WindowKey,
    layout: DockLayout,
    selected: Option<TypeId>,
    viewport: (u32, u32),
    initial_refit: bool,
}

impl UiState {
    pub fn new() -> Result<Self, WindowKeyError> {
        let object_tree = WindowKey::new("object-tree", "Object tree")?;
        let viewport_window = WindowKey::new("viewport", "Viewport")?;
        let layout = DockLayout::split(
            DockSplit::Left,
            0.25,
            DockLayout::tabs([&object_tree]),
            DockLayout::tabs([&viewport_window]),
        );

        Ok(Self {
            object_tree,
            viewport_window,
            layout,
            selected: None,
            viewport: (1, 1),
            initial_refit: true,
        })
    }

    pub fn draw(
        &mut self, ui: &Ui, session: &mut Session, camera: &mut Controller,
    ) -> Result<UiOutput, DockspaceError> {
        ui.dockspace()
            .main_viewport()
            .flags(DockNodeFlags::PASSTHRU_CENTRAL_NODE)
            .layout(&self.layout, DockLayoutApply::IfMissing)
            .build()?;

        let mut exit = false;
        let mut toggle_areas = false;
        let mut toggle_area_outlines = false;
        let mut level_delta = 0;
        let mut underlay_depth = None;
        let mut refit = false;
        let mut interaction = ViewportInteraction {
            selected: session.selected_instance(),
            ..Default::default()
        };

        ui.main_menu_bar(|| {
            ui.menu("File", || {
                if ui.menu_item_with_shortcut("Exit", "Esc") {
                    exit = true;
                }
            });
            ui.menu("View", || {
                if ui.menu_item_enabled_selected_with_shortcut("Show areas", "A", session.options.show_areas, true) {
                    toggle_areas = true;
                }
                if ui.menu_item_enabled_selected_with_shortcut(
                    "Show area outlines",
                    "O",
                    session.options.show_area_outlines,
                    true,
                ) {
                    toggle_area_outlines = true;
                }
                if ui.menu_item_with_shortcut("Z up", "Page Up") {
                    level_delta += 1;
                }
                if ui.menu_item_with_shortcut("Z down", "Page Down") {
                    level_delta -= 1;
                }
                ui.menu("Blur below", || {
                    for depth in 0..=MAX_UNDERLAY_DEPTH {
                        let label = match depth {
                            0 => String::from("Off"),
                            depth => format!("{depth} level(s)"),
                        };

                        if ui.menu_item_enabled_selected(
                            label,
                            None::<&str>,
                            session.options.underlay_depth == depth,
                            true,
                        ) {
                            underlay_depth = Some(depth);
                        }
                    }
                });
                if ui.menu_item_with_shortcut("Refit", "Home") {
                    refit = true;
                }
            });
        });

        if toggle_areas {
            session.toggle_areas();
        }
        if toggle_area_outlines {
            session.toggle_area_outlines();
        }
        if level_delta != 0 {
            session.change_level(level_delta);
        }
        if let Some(depth) = underlay_depth {
            session.set_underlay_depth(depth);
        }

        self.draw_object_tree(ui, session);
        self.draw_viewport(ui, session, camera, &mut interaction, &mut exit, &mut refit);

        Ok(UiOutput {
            exit,
            viewport: self.viewport,
            interaction,
        })
    }

    fn draw_object_tree(&mut self, ui: &Ui, session: &Session) {
        ui.window(&self.object_tree).build(|| {
            let Some(tree) = session.tree() else {
                ui.text_disabled("No environment loaded");

                return;
            };

            for child in sorted_children(tree, TypeId::ROOT) {
                draw_type(ui, tree, child, &mut self.selected);
            }

            ui.separator();
            let Some(selected) = self.selected.and_then(|id| tree.get(id)) else {
                ui.text_disabled("No type selected");

                return;
            };

            ui.text(selected.path.to_string());
            ui.text(format!("{} variables", selected.vars.len()));
            ui.text(format!("{} procedures", selected.procs.len()));
        });
    }

    fn draw_viewport(
        &mut self, ui: &Ui, session: &mut Session, camera: &mut Controller, interaction: &mut ViewportInteraction,
        exit: &mut bool, refit: &mut bool,
    ) {
        ui.window(&self.viewport_window).build(|| {
            draw_z_levels(ui, session);

            let (image_size, viewport) = panel_extent(ui.content_region_avail());
            self.viewport = viewport;
            camera.resize(viewport.0, viewport.1);
            if self.initial_refit || *refit {
                let (width, height) = session.extent_px();
                camera.frame_map(width, height);
                self.initial_refit = false;
                *refit = false;
            }
            ui.image(Renderer::VIEWPORT_TEXTURE, image_size);

            let hovered = ui.is_item_hovered();
            if hovered {
                let io = ui.io();
                if ui.is_mouse_down(MouseButton::Middle) {
                    camera.pan_by(io.mouse_delta());
                }

                let wheel = io.mouse_wheel();
                if wheel != 0.0 {
                    let mouse = io.mouse_pos();
                    let origin = ui.item_rect_min();
                    camera.zoom_by(wheel, [mouse[0] - origin[0], mouse[1] - origin[1]]);
                }

                if ui.is_key_pressed(Key::A) {
                    session.toggle_areas();
                }
                if ui.is_key_pressed(Key::O) {
                    session.toggle_area_outlines();
                }
                if ui.is_key_pressed(Key::PageUp) {
                    session.change_level(1);
                }
                if ui.is_key_pressed(Key::PageDown) {
                    session.change_level(-1);
                }
                if ui.is_key_pressed(Key::Home) {
                    *refit = true;
                }

                if *refit {
                    let (width, height) = session.extent_px();
                    camera.frame_map(width, height);
                    *refit = false;
                }

                let mouse = io.mouse_pos();
                let origin = ui.item_rect_min();
                let cursor = [mouse[0] - origin[0], mouse[1] - origin[1]];
                if cursor.iter().all(|value| value.is_finite() && *value >= 0.0)
                    && cursor[0] < viewport.0 as f32
                    && cursor[1] < viewport.1 as f32
                {
                    interaction.cursor = Some([cursor[0].floor() as u32, cursor[1].floor() as u32]);
                    interaction.pick = ui.is_mouse_clicked(MouseButton::Left);

                    if let Some(size) = session.map().map(|map| map.size)
                        && let Some(coord) = camera.screen_to_tile(cursor, size, session.options.tile_size, session.z())
                    {
                        interaction.hovered_area = session.area_at(coord);
                    }
                }
            }

            if ui.is_key_pressed(Key::Escape) {
                *exit = true;
            }
        });
    }
}

fn draw_z_levels(ui: &Ui, session: &mut Session) {
    let current = session.z();
    let levels = session.level_count();
    let available = ui.content_region_avail();
    let cursor = ui.cursor_pos();
    let width = z_level_width(ui, levels);

    ui.set_cursor_pos_x(cursor[0] + (available[0] - width).max(0.0));
    ui.align_text_to_frame_padding();
    ui.text("Z");

    let mut selected = None;
    for z in 1..=levels {
        ui.same_line();

        if ui.radio_button(z.to_string(), current == z) {
            selected = Some(z);
        }
    }
    ui.new_line();

    if let Some(z) = selected {
        session.set_level(z);
    }
}

fn z_level_width(ui: &Ui, levels: u32) -> f32 {
    let style = ui.clone_style();
    let item_spacing = style.item_spacing()[0];
    let inner_spacing = style.item_inner_spacing()[0];
    let radio = ui.frame_height();
    let mut width = ui.calc_text_size("Z")[0];

    for z in 1..=levels {
        width += item_spacing + radio + inner_spacing + ui.calc_text_size(z.to_string())[0];
    }

    width
}

fn draw_type(ui: &Ui, tree: &ObjectTree, id: TypeId, selected: &mut Option<TypeId>) {
    let Some(decl) = tree.get(id) else {
        return;
    };

    let label = decl
        .path
        .segments
        .last()
        .map_or_else(|| decl.path.to_string(), ToString::to_string);
    let node_id = decl.path.to_string();
    let leaf = decl.children.is_empty();
    let token = ui
        .tree_node_config(node_id)
        .label(label)
        .selected(*selected == Some(id))
        .leaf(leaf)
        .no_tree_push_on_open(leaf)
        .span_avail_width(true)
        .push();

    if ui.is_item_clicked() {
        *selected = Some(id);
    }

    if !leaf && let Some(token) = token {
        for child in sorted_children(tree, id) {
            draw_type(ui, tree, child, selected);
        }

        token.pop();
    }
}

fn sorted_children(tree: &ObjectTree, parent: TypeId) -> Vec<TypeId> {
    let mut children = tree
        .get(parent)
        .into_iter()
        .flat_map(|decl| decl.children.iter().copied())
        .filter(|id| tree.get(*id).is_some())
        .collect::<Vec<_>>();

    children.sort_by(|left, right| {
        let left = tree.get(*left).map(|decl| decl.path.to_string()).unwrap_or_default();
        let right = tree.get(*right).map(|decl| decl.path.to_string()).unwrap_or_default();

        left.cmp(&right)
    });

    children
}

fn panel_extent(available: [f32; 2]) -> ([f32; 2], (u32, u32)) {
    let width = if available[0].is_finite() {
        available[0].max(1.0)
    } else {
        1.0
    };
    let height = if available[1].is_finite() {
        available[1].max(1.0)
    } else {
        1.0
    };

    ([width, height], (width.floor() as u32, height.floor() as u32))
}

#[cfg(test)]
mod tests {
    use core::{location::Location, path::TreePath};

    use super::*;

    #[test]
    fn the_default_dock_layout_is_valid() {
        let state = UiState::new().expect("valid window keys");

        assert_eq!(state.layout.validate(), Ok(()));
    }

    #[test]
    fn object_tree_children_are_sorted_by_path() {
        let mut tree = ObjectTree::new();
        tree.register(&TreePath::parse("/obj/zeta"), Location::default());
        tree.register(&TreePath::parse("/obj/alpha"), Location::default());
        let obj = tree.id_of(&TreePath::parse("/obj")).expect("registered parent");
        let paths = sorted_children(&tree, obj)
            .into_iter()
            .filter_map(|id| tree.get(id))
            .map(|decl| decl.path.to_string())
            .collect::<Vec<_>>();

        assert_eq!(paths, ["/obj/alpha", "/obj/zeta"]);
    }

    #[test]
    fn panel_extent_rejects_non_finite_or_empty_sizes() {
        assert_eq!(panel_extent([0.0, f32::NAN]), ([1.0, 1.0], (1, 1)));
        assert_eq!(panel_extent([320.75, 200.25]), ([320.75, 200.25], (320, 200)));
    }
}
