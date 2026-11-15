use base::{path::TreePath, types::Identifier, vars};
use std::collections::{HashMap, HashSet};

use dear_imgui_rs::{ListClipper, StyleColor, TableFlags, TableSizingPolicy, Ui, WindowKey, WindowKeyError};
use objtree::{ObjectTree, TypeId};
use ui::{
    icons::materialdesignicons::{ICON_COG, ICON_EYE, ICON_EYE_OFF, ICON_FILTER},
    widgets::{dpi, focus_window_on_hover},
};

use crate::{
    app::external_editor::SourceLocation,
    palette::panel::thumbnail::draw_thumbnail,
    session::Session,
    settings::{
        ObjectTreeFilterOptions, ObjectTreeSearchOptions, Settings,
        panel::{draw_object_tree_filter_settings, draw_object_tree_search_settings},
    },
};

pub(crate) mod type_search;

const OBJECT_TREE_FILTER_OPTIONS_POPUP: &str = "object-tree-filter-options-popup";
const OBJECT_TREE_OPTIONS_POPUP: &str = "object-tree-options-popup";
const OBJECT_TREE_LINE_COLORS: [[f32; 4]; 4] = [
    [254.0 / 255.0, 112.0 / 255.0, 246.0 / 255.0, 1.0],
    [142.0 / 255.0, 112.0 / 255.0, 254.0 / 255.0, 1.0],
    [112.0 / 255.0, 180.0 / 255.0, 254.0 / 255.0, 1.0],
    [48.0 / 255.0, 134.0 / 255.0, 198.0 / 255.0, 1.0],
];
const OBJECT_TREE_LINE_THICKNESS: f32 = 1.5;
const OBJECT_TREE_BRANCH_LENGTH: f32 = 9.0;
const OBJECT_TREE_LEAF_BRANCH_LENGTH: f32 = 18.0;

#[derive(Debug, Default)]
struct ObjectTreeFilter {
    roots: Vec<TypeId>,
    children: HashMap<TypeId, Vec<TypeId>>,
    matches: HashSet<TypeId>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct ObjectTreeTypeFilter {
    atom: Option<TypeId>,
    movable: Option<TypeId>,
    obj: Option<TypeId>,
    turf: Option<TypeId>,
    custom: Option<TypeId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ObjectTreeFilterAction {
    Keep,
    Prune,
}

impl ObjectTreeTypeFilter {
    fn new(tree: &ObjectTree, options: &ObjectTreeFilterOptions) -> Self {
        let roots = tree.roots();
        let custom = options
            .custom_enabled
            .then(|| options.custom_type_path.trim())
            .filter(|path| !path.is_empty())
            .and_then(|path| tree.id_of(&TreePath::parse(path)));

        Self {
            atom: options.atom.then_some(roots.atom).flatten(),
            movable: options.movable.then_some(roots.movable).flatten(),
            obj: options.obj.then_some(roots.obj).flatten(),
            turf: options.turf.then_some(roots.turf).flatten(),
            custom,
        }
    }

    fn action(self, tree: &ObjectTree, id: TypeId) -> ObjectTreeFilterAction {
        let filtered_by_builtin = [self.atom, self.movable, self.obj, self.turf]
            .into_iter()
            .flatten()
            .any(|filtered| tree.is_subtype_of(id, filtered));
        if filtered_by_builtin || self.custom.is_some_and(|custom| tree.is_subtype_of(id, custom)) {
            ObjectTreeFilterAction::Prune
        } else {
            ObjectTreeFilterAction::Keep
        }
    }
}

impl ObjectTreeSearchOptions {
    const fn hint(self) -> &'static str {
        match (self.type_paths, self.names) {
            (true, true) => "Search type paths or names",
            (true, false) => "Search type paths",
            (false, true) => "Search names",
            (false, false) => "Search types",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ObjectTreeRow {
    id: TypeId,
    parent: Option<usize>,
    leaf: bool,
    last_sibling: bool,
}

#[derive(Clone, Copy, Default)]
struct ObjectTreeRowOptions<'a> {
    filter: Option<&'a ObjectTreeFilter>,
    type_filter: ObjectTreeTypeFilter,
    expand: bool,
    reveal: Option<TypeId>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct ObjectTreeOutput {
    chosen: Option<TypeId>,
    visibility_toggle: Option<TypeId>,
    open_source: Option<SourceLocation>,
    find: Option<TreePath>,
    copy_path: Option<String>,
    reveal: Option<TypeId>,
}

#[derive(Default)]
pub(crate) struct ObjectTreePanelOutput {
    pub(crate) open_source: Option<SourceLocation>,
    pub(crate) find: Option<TreePath>,
    pub(crate) copy_path: Option<String>,
    pub(crate) docked: bool,
}

impl ObjectTreeFilter {
    fn new(
        tree: &ObjectTree, atom: TypeId, query: &str, options: ObjectTreeSearchOptions,
        type_filter: ObjectTreeTypeFilter,
    ) -> Self {
        let matches = matching_object_types(tree, query, options)
            .into_iter()
            .filter(|id| tree.is_subtype_of(*id, atom))
            .filter(|id| type_filter.action(tree, *id) == ObjectTreeFilterAction::Keep)
            .collect::<Vec<_>>();
        let match_set = matches.iter().copied().collect::<HashSet<_>>();
        let mut filter = Self::default();

        for id in matches {
            let matching_parent = tree
                .ancestors(id)
                .skip(1)
                .find(|ancestor| match_set.contains(&ancestor.id))
                .map(|ancestor| ancestor.id);
            if let Some(parent) = matching_parent {
                filter.children.entry(parent).or_default().push(id);
            } else {
                filter.roots.push(id);
            }
        }
        filter.matches = match_set;

        filter
    }

    fn is_empty(&self) -> bool {
        self.roots.is_empty()
    }

    fn contains(&self, id: TypeId) -> bool {
        self.matches.contains(&id)
    }

    fn children(&self, id: TypeId) -> &[TypeId] {
        self.children.get(&id).map_or(&[], Vec::as_slice)
    }
}

pub(crate) struct ObjectTreePanel {
    window: WindowKey,
    selected: Option<TypeId>,
    reveal: Option<TypeId>,
    search: String,
    filter: Option<ObjectTreeFilter>,
    filter_revision: u64,
}

impl ObjectTreePanel {
    pub(crate) fn new() -> Result<Self, WindowKeyError> {
        Ok(Self {
            window: WindowKey::new("object-tree", "Object tree")?,
            selected: None,
            reveal: None,
            search: String::new(),
            filter: None,
            filter_revision: 0,
        })
    }

    pub(crate) const fn window(&self) -> &WindowKey {
        &self.window
    }

    pub(crate) fn reveal_picked_instance(&mut self, session: &Session) {
        let Some(selected) = session
            .picked_prefab()
            .and_then(|prefab| session.tree()?.id_of(&prefab.path))
        else {
            return;
        };

        self.selected = Some(selected);
        self.reveal = Some(selected);
    }

    pub(crate) fn invalidate_filter(&mut self) {
        self.filter_revision = u64::MAX;
    }

    pub(crate) fn draw(
        &mut self, ui: &Ui, session: &mut Session, settings: &mut Settings, focus: bool, open: &mut bool,
    ) -> ObjectTreePanelOutput {
        let mut panel = ObjectTreePanelOutput::default();
        if !*open {
            return panel;
        }

        ui.window(&self.window).opened(open).focused(focus).build(|| {
            panel.docked = ui.is_window_docked();
            if settings.focus_windows_on_hover {
                focus_window_on_hover(ui);
            }
            let mut output = ObjectTreeOutput {
                reveal: self.reveal,
                ..ObjectTreeOutput::default()
            };
            let available_width = ui.content_region_avail()[0];
            let button_size = ui.frame_height();
            let spacing = ui.clone_style().item_spacing()[0];
            ui.set_next_item_width((available_width - button_size * 2.0 - spacing * 2.0).max(1.0));
            let search_changed = ui
                .input_text("##object-tree-search", &mut self.search)
                .hint(settings.object_tree_search.hint())
                .build();
            ui.same_line();
            let filter_clicked = ui.button_with_size("##object-tree-filter-options", [button_size, button_size]);
            draw_centered_icon(ui, ICON_FILTER);
            if filter_clicked {
                ui.open_popup(OBJECT_TREE_FILTER_OPTIONS_POPUP);
            }
            ui.set_item_tooltip("Type filters");

            ui.same_line();
            let options_clicked = ui.button_with_size("##object-tree-search-options", [button_size, button_size]);
            draw_centered_icon(ui, ICON_COG);
            if options_clicked {
                ui.open_popup(OBJECT_TREE_OPTIONS_POPUP);
            }
            ui.set_item_tooltip("Object tree settings");

            let mut type_filter_changed = false;
            if let Some(_popup) = ui.begin_popup(OBJECT_TREE_FILTER_OPTIONS_POPUP) {
                type_filter_changed |= draw_object_tree_filter_settings(ui, &mut settings.object_tree_filter);
            }

            let mut search_options_changed = false;
            if let Some(_popup) = ui.begin_popup(OBJECT_TREE_OPTIONS_POPUP) {
                search_options_changed |= draw_object_tree_search_settings(ui, &mut settings.object_tree_search);

                ui.separator();
                ui.checkbox("Line indicators", &mut settings.object_tree_line_indicators);
            }

            ui.child_window("object-tree-content").build(ui, || {
                let tree_revision = session.texture_revision();
                let Some(tree) = session.tree() else {
                    ui.text_disabled("No environment loaded");

                    return;
                };
                let Some(atom) = tree.roots().atom else {
                    ui.text_disabled("No /atom type available");

                    return;
                };
                let type_filter = ObjectTreeTypeFilter::new(tree, &settings.object_tree_filter);
                let rebuild_filter = search_changed
                    || search_options_changed
                    || type_filter_changed
                    || self.filter_revision != tree_revision;
                if rebuild_filter {
                    self.filter = (!self.search.trim().is_empty()).then(|| {
                        ObjectTreeFilter::new(tree, atom, &self.search, settings.object_tree_search, type_filter)
                    });
                    self.filter_revision = tree_revision;
                }

                if self.filter.as_ref().is_some_and(ObjectTreeFilter::is_empty) {
                    ui.text_disabled("No matching types");

                    return;
                }
                let roots = self.filter.as_ref().map_or_else(
                    || visible_type_roots(tree, atom, type_filter),
                    |filter| filter.roots.clone(),
                );
                if roots.is_empty() {
                    ui.text_disabled("No types pass filters");

                    return;
                }

                ui.table("object-tree-types")
                    .flags(TableFlags::BORDERS_INNER_V | TableFlags::ROW_BG)
                    .sizing_policy(TableSizingPolicy::StretchProp)
                    .column("Type")
                    .weight(1.0)
                    .done()
                    .column("Visibility")
                    .width(ui.frame_height() * 2.0)
                    .done()
                    .build(|ui| {
                        let mut rows = Vec::new();
                        let options = ObjectTreeRowOptions {
                            filter: self.filter.as_ref(),
                            type_filter,
                            expand: rebuild_filter && self.filter.is_some(),
                            reveal: self.reveal,
                        };
                        for root in roots.iter().copied() {
                            collect_type_rows(ui, tree, root, None, true, &mut rows, &options);
                        }
                        let reveal_row = self
                            .reveal
                            .and_then(|target| rows.iter().position(|row| row.id == target));
                        let mut clipper = ListClipper::new(rows.len()).begin(ui);
                        if let Some(index) = reveal_row {
                            clipper.include_item_by_index(index);
                        }
                        for index in clipper.iter() {
                            draw_type_row(
                                ui,
                                session,
                                &rows,
                                index,
                                settings.object_tree_line_indicators,
                                &mut self.selected,
                                &mut output,
                            );
                        }
                        if reveal_row.is_some() {
                            self.reveal = None;
                        }
                    });
            });

            if let Some(chosen) = output.chosen {
                self.reveal = None;
                session.choose_type(chosen);
            }
            if let Some(id) = output.visibility_toggle {
                session.toggle_type_visibility(id);
            }
            panel.open_source = output.open_source;
            panel.find = output.find;
            panel.copy_path = output.copy_path;
        });

        panel
    }
}

fn matching_object_types(tree: &ObjectTree, query: &str, options: ObjectTreeSearchOptions) -> Vec<TypeId> {
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return Vec::new();
    }

    let name = Identifier::from(vars::NAME);
    let mut matches = tree
        .iter()
        .filter(|decl| {
            let path_matches = options.type_paths && decl.path.to_string().to_ascii_lowercase().contains(&query);
            let name_matches = options.names
                && tree
                    .var_inherited(decl.id, &name)
                    .and_then(|variable| variable.value.as_text())
                    .is_some_and(|value| value.to_ascii_lowercase().contains(&query));

            path_matches || name_matches
        })
        .map(|decl| decl.id)
        .collect::<Vec<_>>();
    matches.sort_by_key(|id| tree.get(*id).map(|decl| decl.path.to_string()).unwrap_or_default());

    matches
}

fn draw_centered_icon(ui: &Ui, icon: char) {
    let item_min = ui.item_rect_min();
    let item_max = ui.item_rect_max();
    let center = [(item_min[0] + item_max[0]) * 0.5, (item_min[1] + item_max[1]) * 0.5];
    let glyph = ui.current_baked_font().glyph(icon);
    let icon = icon.to_string();
    let position = glyph.map_or_else(
        || {
            let size = ui.calc_text_size(&icon);
            [center[0] - size[0] * 0.5, center[1] - size[1] * 0.5]
        },
        |glyph| {
            let (glyph_min, glyph_max) = glyph.position_and_size();
            [
                center[0] - (glyph_min[0] + glyph_max[0]) * 0.5,
                center[1] - (glyph_min[1] + glyph_max[1]) * 0.5,
            ]
        },
    );

    ui.get_window_draw_list()
        .add_text(position, ui.style_color(StyleColor::Text), icon);
}

#[derive(Debug, Clone, Copy)]
struct OverlayRect {
    min: [f32; 2],
    max: [f32; 2],
}

fn collect_type_rows(
    ui: &Ui, tree: &ObjectTree, id: TypeId, parent: Option<usize>, last_sibling: bool, rows: &mut Vec<ObjectTreeRow>,
    options: &ObjectTreeRowOptions<'_>,
) {
    let Some(decl) = tree.get(id) else {
        return;
    };
    let children = options.filter.map_or_else(
        || visible_type_children(tree, id, options.type_filter),
        |filter| filter.children(id).to_vec(),
    );
    let leaf = children.is_empty();
    let row = rows.len();
    rows.push(ObjectTreeRow {
        id,
        parent,
        leaf,
        last_sibling,
    });

    if leaf {
        return;
    }

    let node_id = decl.path.to_string();
    let storage_id = ui.get_id(&node_id);
    ui.with_current_state_storage(|mut storage| {
        let initialize_atom = tree.roots().atom == Some(id) && storage.get_int(storage_id, -1) == -1;
        let reveal_descendant = options.reveal.is_some_and(|target| {
            target != id
                && options.filter.is_none_or(|filter| filter.contains(target))
                && options.type_filter.action(tree, target) == ObjectTreeFilterAction::Keep
                && tree.is_subtype_of(target, id)
        });
        if options.expand || initialize_atom || reveal_descendant {
            storage.set_bool(storage_id, true);
        }
    });
    if !ui.tree_node_get_open(storage_id) {
        return;
    }

    let _id = ui.push_id(&node_id);
    let child_count = children.len();
    for (index, child) in children.into_iter().enumerate() {
        collect_type_rows(ui, tree, child, Some(row), index + 1 == child_count, rows, options);
    }
}

fn draw_type_row(
    ui: &Ui, session: &Session, rows: &[ObjectTreeRow], row_index: usize, draw_line_indicators: bool,
    selected: &mut Option<TypeId>, output: &mut ObjectTreeOutput,
) {
    let Some(tree) = session.tree() else {
        return;
    };
    let Some(row) = rows.get(row_index).copied() else {
        return;
    };
    let Some(decl) = tree.get(row.id) else {
        return;
    };

    let mut ancestors = Vec::new();
    let mut parent = row.parent;
    while let Some(index) = parent {
        let Some(ancestor) = rows.get(index) else {
            break;
        };
        ancestors.push(index);
        parent = ancestor.parent;
    }
    ancestors.reverse();
    let mut scopes = Vec::with_capacity(ancestors.len());
    for ancestor in &ancestors {
        if let Some(ancestor) = rows.get(*ancestor).and_then(|row| tree.get(row.id)) {
            scopes.push(ui.tree_push(ancestor.path.to_string()));
        }
    }

    let label = decl
        .path
        .segments
        .last()
        .map_or_else(|| decl.path.to_string(), ToString::to_string);
    let node_id = decl.path.to_string();
    {
        ui.table_next_row();
        ui.table_next_column();
        let cursor = ui.cursor_screen_pos();
        let icon_extent = ui.text_line_height();
        let icon_spacing = ui.clone_style().item_inner_spacing()[0];
        let space_width = ui.calc_text_size(" ")[0].max(1.0);
        let icon_padding = " ".repeat(((icon_extent + icon_spacing) / space_width).ceil() as usize);
        let opened = ui
            .tree_node_config(&node_id)
            .label(format!("{icon_padding}{label}"))
            .selected(*selected == Some(row.id))
            .leaf(row.leaf)
            .no_tree_push_on_open(true)
            .frame_padding(true)
            .span_avail_width(true)
            .push()
            .is_some();
        draw_type_icon(ui, session.type_thumbnail(row.id), cursor);
        if draw_line_indicators {
            let node_rect = OverlayRect {
                min: ui.item_rect_min(),
                max: ui.item_rect_max(),
            };
            draw_object_tree_lines(ui, rows, row_index, &ancestors, cursor, node_rect, opened);
        }

        if ui.is_item_clicked() {
            *selected = Some(row.id);
            output.chosen = Some(row.id);
        }
        if output.reveal == Some(row.id) {
            ui.set_scroll_here_y(0.5);
        }
        ui.set_item_tooltip(&node_id);
        let source = session.type_source(row.id);
        if let Some(_popup) = ui.begin_popup_context_item() {
            if ui.menu_item_enabled_selected_no_shortcut("Open in editor", false, source.is_some()) {
                output.open_source = source;
            }
            if ui.menu_item_enabled_selected_no_shortcut("Find on map", false, session.map().is_some()) {
                output.find = Some(decl.path.clone());
            }
            if ui.menu_item("Copy type path") {
                output.copy_path = Some(node_id.clone());
            }
        }

        ui.table_next_column();
        let visible = session.is_type_visible(row.id);
        let icon = if visible { ICON_EYE } else { ICON_EYE_OFF };
        let transparent = [0.0, 0.0, 0.0, 0.0];
        let _button = ui.push_style_color(StyleColor::Button, transparent);
        let _button_hovered = ui.push_style_color(StyleColor::ButtonHovered, transparent);
        let _button_active = ui.push_style_color(StyleColor::ButtonActive, transparent);
        let _text = (!visible).then(|| ui.push_style_color(StyleColor::Text, ui.style_color(StyleColor::TextDisabled)));
        if ui.button_with_size(
            format!("{icon}##object-type-visibility-{node_id}"),
            [ui.content_region_avail()[0], ui.frame_height()],
        ) {
            output.visibility_toggle = Some(row.id);
        }
        ui.set_item_tooltip(if visible {
            format!("Hide {node_id} and its descendants")
        } else {
            format!("Show {node_id} and its descendants")
        });
    }

    while let Some(scope) = scopes.pop() {
        scope.pop();
    }
}

fn draw_object_tree_lines(
    ui: &Ui, rows: &[ObjectTreeRow], row_index: usize, ancestors: &[usize], cursor: [f32; 2], node_rect: OverlayRect,
    opened: bool,
) {
    let Some(row) = rows.get(row_index).copied() else {
        return;
    };
    let depth = ancestors.len();
    let midpoint = (node_rect.min[1] + node_rect.max[1]) * 0.5;
    let style = ui.clone_style();
    let indent = style.indent_spacing();
    let arrow_center_offset = style.frame_padding()[0] + ui.current_font_size() * 0.5;
    let scale = dpi(ui);
    let draw = ui.get_window_draw_list();

    if depth > 0 {
        for (ancestor_depth, child) in ancestors
            .iter()
            .copied()
            .skip(1)
            .chain(std::iter::once(row_index))
            .enumerate()
        {
            let Some(child) = rows.get(child) else {
                continue;
            };
            let x = cursor[0] - indent * (depth - ancestor_depth) as f32 + arrow_center_offset;
            let end = if child.last_sibling && ancestor_depth + 1 < depth {
                continue;
            } else if child.last_sibling {
                midpoint
            } else {
                node_rect.max[1]
            };
            draw.add_line(
                [x, node_rect.min[1]],
                [x, end],
                OBJECT_TREE_LINE_COLORS[ancestor_depth % OBJECT_TREE_LINE_COLORS.len()],
            )
            .thickness(OBJECT_TREE_LINE_THICKNESS * scale)
            .build();
        }
    }

    if depth > 0 {
        let x = cursor[0] - indent + arrow_center_offset;
        let length = scale
            * if row.leaf {
                OBJECT_TREE_LEAF_BRANCH_LENGTH
            } else {
                OBJECT_TREE_BRANCH_LENGTH
            };
        draw.add_line(
            [x, midpoint],
            [cursor[0] + length, midpoint],
            OBJECT_TREE_LINE_COLORS[(depth - 1) % OBJECT_TREE_LINE_COLORS.len()],
        )
        .thickness(OBJECT_TREE_LINE_THICKNESS * scale)
        .build();
    }

    if opened && !row.leaf {
        let x = cursor[0] + arrow_center_offset;
        draw.add_line(
            [x, midpoint],
            [x, node_rect.max[1]],
            OBJECT_TREE_LINE_COLORS[depth % OBJECT_TREE_LINE_COLORS.len()],
        )
        .thickness(OBJECT_TREE_LINE_THICKNESS * scale)
        .build();
    }
}

fn draw_type_icon(ui: &Ui, thumbnail: Option<crate::palette::session::PrefabThumbnail>, cursor: [f32; 2]) {
    let row_min = ui.item_rect_min();
    let row_max = ui.item_rect_max();
    let row_height = (row_max[1] - row_min[1]).max(1.0);
    let icon_extent = ui.text_line_height().min(row_height);
    let icon_slot_x = cursor[0] + ui.tree_node_to_label_spacing();
    let min = [icon_slot_x, row_min[1] + (row_height - icon_extent) * 0.5];

    draw_thumbnail(ui, thumbnail, min, icon_extent, None);
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

fn visible_type_roots(tree: &ObjectTree, root: TypeId, filter: ObjectTreeTypeFilter) -> Vec<TypeId> {
    match filter.action(tree, root) {
        ObjectTreeFilterAction::Keep => vec![root],
        ObjectTreeFilterAction::Prune => Vec::new(),
    }
}

fn visible_type_children(tree: &ObjectTree, parent: TypeId, filter: ObjectTreeTypeFilter) -> Vec<TypeId> {
    let mut visible = Vec::new();
    for child in sorted_children(tree, parent) {
        match filter.action(tree, child) {
            ObjectTreeFilterAction::Keep => visible.push(child),
            ObjectTreeFilterAction::Prune => {},
        }
    }

    visible
}

#[cfg(test)]
mod tests;
