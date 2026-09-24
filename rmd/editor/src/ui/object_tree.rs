use core::{path::TreePath, types::Identifier};
use std::collections::{HashMap, HashSet};

use dear_imgui_rs::{ListClipper, StyleColor, TableFlags, TableSizingPolicy, Ui, WindowKey, WindowKeyError};
use editor::icons::materialdesignicons::{ICON_COG, ICON_EYE, ICON_EYE_OFF, ICON_FILTER, ICON_IMAGE_BROKEN};
use objtree::{ObjectTree, TypeId};
use render::Renderer;

use super::{
    common::{fit_icon, focus_window_on_hover},
    settings::{draw_object_tree_filter_settings, draw_object_tree_search_settings},
};
use crate::{
    external_editor::SourceLocation,
    session::Session,
    settings::{ObjectTreeFilterOptions, ObjectTreeSearchOptions, Settings},
};

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
    reveal: Option<TypeId>,
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

    fn is_empty(&self) -> bool { self.roots.is_empty() }

    fn contains(&self, id: TypeId) -> bool { self.matches.contains(&id) }

    fn children(&self, id: TypeId) -> &[TypeId] { self.children.get(&id).map_or(&[], Vec::as_slice) }
}

pub(super) struct ObjectTreePanel {
    window: WindowKey,
    selected: Option<TypeId>,
    reveal: Option<TypeId>,
    search: String,
    filter: Option<ObjectTreeFilter>,
    filter_revision: u64,
}

impl ObjectTreePanel {
    pub(super) fn new() -> Result<Self, WindowKeyError> {
        Ok(Self {
            window: WindowKey::new("object-tree", "Object tree")?,
            selected: None,
            reveal: None,
            search: String::new(),
            filter: None,
            filter_revision: 0,
        })
    }

    pub(super) const fn window(&self) -> &WindowKey { &self.window }

    pub(super) fn reveal_selected_instance(&mut self, session: &Session) {
        let Some(selected) = session
            .selected_prefab()
            .and_then(|prefab| session.tree()?.id_of(&prefab.path))
        else {
            return;
        };

        self.selected = Some(selected);
        self.reveal = Some(selected);
    }

    pub(super) fn invalidate_filter(&mut self) { self.filter_revision = u64::MAX; }

    /// Draws the panel, `focus` brings its tab to the front. Returns a source location to open and
    /// whether the window sits in a dock node.
    pub(super) fn draw(
        &mut self, ui: &Ui, session: &mut Session, settings: &mut Settings, focus: bool,
    ) -> (Option<SourceLocation>, bool) {
        let mut open_source = None;
        let mut docked = false;
        ui.window(&self.window).focused(focus).build(|| {
            docked = ui.is_window_docked();
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
            open_source = output.open_source;
        });

        (open_source, docked)
    }
}

fn matching_object_types(tree: &ObjectTree, query: &str, options: ObjectTreeSearchOptions) -> Vec<TypeId> {
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return Vec::new();
    }

    let name = Identifier::from("name");
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
        if let Some(_popup) = ui.begin_popup_context_item()
            && ui.menu_item_enabled_selected_no_shortcut("Open in editor", false, source.is_some())
        {
            output.open_source = source;
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
            .thickness(OBJECT_TREE_LINE_THICKNESS)
            .build();
        }
    }

    if depth > 0 {
        let x = cursor[0] - indent + arrow_center_offset;
        let length = if row.leaf {
            OBJECT_TREE_LEAF_BRANCH_LENGTH
        } else {
            OBJECT_TREE_BRANCH_LENGTH
        };
        draw.add_line(
            [x, midpoint],
            [cursor[0] + length, midpoint],
            OBJECT_TREE_LINE_COLORS[(depth - 1) % OBJECT_TREE_LINE_COLORS.len()],
        )
        .thickness(OBJECT_TREE_LINE_THICKNESS)
        .build();
    }

    if opened && !row.leaf {
        let x = cursor[0] + arrow_center_offset;
        draw.add_line(
            [x, midpoint],
            [x, node_rect.max[1]],
            OBJECT_TREE_LINE_COLORS[depth % OBJECT_TREE_LINE_COLORS.len()],
        )
        .thickness(OBJECT_TREE_LINE_THICKNESS)
        .build();
    }
}

fn draw_type_icon(ui: &Ui, thumbnail: Option<crate::session::PrefabThumbnail>, cursor: [f32; 2]) {
    let row_min = ui.item_rect_min();
    let row_max = ui.item_rect_max();
    let row_height = (row_max[1] - row_min[1]).max(1.0);
    let icon_extent = ui.text_line_height().min(row_height);
    let icon_slot_x = cursor[0] + ui.tree_node_to_label_spacing();
    let draw = ui.get_window_draw_list();

    match thumbnail {
        Some(thumbnail) => {
            let image_size = fit_icon(thumbnail.texture.width, thumbnail.texture.height, icon_extent);
            let image_min = [
                icon_slot_x + (icon_extent - image_size[0]) * 0.5,
                row_min[1] + (row_height - image_size[1]) * 0.5,
            ];
            let image_max = [image_min[0] + image_size[0], image_min[1] + image_size[1]];
            draw.add_image(
                Renderer::sprite_texture(thumbnail.texture.index),
                image_min,
                image_max,
                thumbnail.uv0,
                thumbnail.uv1,
                thumbnail.tint,
            );
        },
        None => {
            let fallback = ICON_IMAGE_BROKEN.to_string();
            let fallback_width = ui.calc_text_size(&fallback)[0];
            draw.add_text(
                [
                    icon_slot_x + (icon_extent - fallback_width) * 0.5,
                    row_min[1] + (row_height - ui.text_line_height()) * 0.5,
                ],
                ui.style_color(StyleColor::TextDisabled),
                fallback,
            );
        },
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
mod tests {
    use core::{
        location::Location,
        path::TreePath,
        types::{Identifier, Value, VarModifiers},
    };

    use dmm::{Coord, Map, Prefab, Size};
    use editor::{document::MapDocument, tool::Tool};
    use objtree::VarDecl;

    use super::{
        super::{IMGUI_CONTEXT, MAX_CUSTOM_FILL_SEARCH_RESULTS, matching_type_paths},
        *,
    };

    fn set_type_name(tree: &mut ObjectTree, id: TypeId, value: &str) {
        let name = Identifier::from("name");
        tree.get_mut(id).unwrap().vars.insert(
            name.clone(),
            VarDecl {
                name,
                declared_type: None,
                modifiers: VarModifiers::default(),
                value: Value::Text(value.to_owned()),
                initializer: None,
                declared: true,
                location: Location::default(),
            },
        );
    }

    #[test]
    fn object_tree_search_is_not_limited_to_custom_fill_result_count() {
        let mut tree = ObjectTree::new();
        for index in 0..60 {
            tree.register(
                &TreePath::parse(&format!("/obj/floor/type_{index}")),
                Location::default(),
            );
        }

        assert_eq!(
            matching_type_paths(&tree, "floor").len(),
            MAX_CUSTOM_FILL_SEARCH_RESULTS
        );
        assert_eq!(
            matching_object_types(&tree, "floor", ObjectTreeSearchOptions::default()).len(),
            61
        );
    }

    #[test]
    fn object_tree_search_matches_inherited_atom_names() {
        let mut tree = ObjectTree::new();
        let atom = tree.register(&TreePath::parse("/atom"), Location::default());
        let parent = tree.register(&TreePath::parse("/atom/movable/tool"), Location::default());
        let child = tree.register(&TreePath::parse("/atom/movable/tool/wrench"), Location::default());
        tree.register(&TreePath::parse("/atom/structure/table"), Location::default());
        set_type_name(&mut tree, parent, "Portable Tool");

        let options = ObjectTreeSearchOptions {
            type_paths: false,
            names: true,
        };
        let matches = matching_object_types(&tree, "PORTABLE", options);
        let filter = ObjectTreeFilter::new(&tree, atom, "PORTABLE", options, ObjectTreeTypeFilter::default());

        assert_eq!(matches, [parent, child]);
        assert_eq!(filter.roots, [parent]);
        assert_eq!(filter.children(parent), [child]);
    }

    #[test]
    fn object_tree_search_combines_enabled_fields() {
        let mut tree = ObjectTree::new();
        tree.register(&TreePath::parse("/atom"), Location::default());
        let path_match = tree.register(&TreePath::parse("/atom/movable/needle"), Location::default());
        let name_match = tree.register(&TreePath::parse("/atom/movable/scalpel"), Location::default());
        set_type_name(&mut tree, name_match, "Needle tool");

        let path_only = matching_object_types(
            &tree,
            "needle",
            ObjectTreeSearchOptions {
                type_paths: true,
                names: false,
            },
        );
        let name_only = matching_object_types(
            &tree,
            "needle",
            ObjectTreeSearchOptions {
                type_paths: false,
                names: true,
            },
        );
        let combined = matching_object_types(
            &tree,
            "needle",
            ObjectTreeSearchOptions {
                type_paths: true,
                names: true,
            },
        );

        assert_eq!(path_only, [path_match]);
        assert_eq!(name_only, [name_match]);
        assert_eq!(combined, [path_match, name_match]);
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
    fn built_in_object_tree_filters_prune_the_base_types_and_all_subtypes() {
        let mut tree = ObjectTree::new();
        let atom = tree.register(&TreePath::parse("/atom"), Location::default());
        let movable = tree.register(&TreePath::parse("/atom/movable"), Location::default());
        let item = tree.register(&TreePath::parse("/obj/item"), Location::default());
        let open_turf = tree.register(&TreePath::parse("/turf/open"), Location::default());
        let roots = tree.roots();
        tree.get_mut(roots.obj.unwrap()).unwrap().parent_type = Some(TreePath::parse("/atom/movable"));
        tree.get_mut(roots.turf.unwrap()).unwrap().parent_type = Some(TreePath::parse("/atom"));
        tree.resolve_parent_types();

        for (options, filtered_root, filtered_subtype) in [
            (
                ObjectTreeFilterOptions {
                    atom: true,
                    ..ObjectTreeFilterOptions::default()
                },
                atom,
                open_turf,
            ),
            (
                ObjectTreeFilterOptions {
                    movable: true,
                    ..ObjectTreeFilterOptions::default()
                },
                movable,
                item,
            ),
            (
                ObjectTreeFilterOptions {
                    obj: true,
                    ..ObjectTreeFilterOptions::default()
                },
                roots.obj.unwrap(),
                item,
            ),
            (
                ObjectTreeFilterOptions {
                    turf: true,
                    ..ObjectTreeFilterOptions::default()
                },
                roots.turf.unwrap(),
                open_turf,
            ),
        ] {
            let filter = ObjectTreeTypeFilter::new(&tree, &options);

            assert_eq!(filter.action(&tree, filtered_root), ObjectTreeFilterAction::Prune);
            assert_eq!(filter.action(&tree, filtered_subtype), ObjectTreeFilterAction::Prune);
        }
    }

    #[test]
    fn custom_object_tree_filter_prunes_the_type_and_all_subtypes() {
        let mut tree = ObjectTree::new();
        let atom = tree.register(&TreePath::parse("/atom"), Location::default());
        let keep = tree.register(&TreePath::parse("/atom/keep"), Location::default());
        let removed = tree.register(&TreePath::parse("/atom/remove"), Location::default());
        let removed_child = tree.register(&TreePath::parse("/atom/remove/child"), Location::default());
        let options = ObjectTreeFilterOptions {
            custom_enabled: true,
            custom_type_path: String::from("/atom/remove"),
            ..ObjectTreeFilterOptions::default()
        };
        let filter = ObjectTreeTypeFilter::new(&tree, &options);

        assert_eq!(visible_type_children(&tree, atom, filter), [keep]);
        assert_eq!(filter.action(&tree, removed), ObjectTreeFilterAction::Prune);
        assert_eq!(filter.action(&tree, removed_child), ObjectTreeFilterAction::Prune);
    }

    #[test]
    fn object_tree_search_respects_the_active_type_filters() {
        let mut tree = ObjectTree::new();
        let atom = tree.register(&TreePath::parse("/atom"), Location::default());
        let keep = tree.register(&TreePath::parse("/atom/keep_match"), Location::default());
        tree.register(&TreePath::parse("/atom/remove/keep_match"), Location::default());
        let type_filter = ObjectTreeTypeFilter::new(
            &tree,
            &ObjectTreeFilterOptions {
                custom_enabled: true,
                custom_type_path: String::from("/atom/remove"),
                ..ObjectTreeFilterOptions::default()
            },
        );

        let filter = ObjectTreeFilter::new(
            &tree,
            atom,
            "keep_match",
            ObjectTreeSearchOptions::default(),
            type_filter,
        );

        assert_eq!(filter.roots, [keep]);
        assert!(filter.children(keep).is_empty());
        assert!(filter.contains(keep));
    }

    #[test]
    fn a_selected_map_instance_queues_its_type_for_object_tree_reveal() {
        let path = TreePath::parse("/atom/structure/table");
        let mut tree = ObjectTree::new();
        let selected_type = tree.register(&path, Location::default());
        let mut map = Map::new(Size { x: 1, y: 1, z: 1 });
        let tile = map.intern_tile(vec![Prefab::new(path)]);
        map.grid[0][0][0] = tile;
        let document = MapDocument::new(map, 1);
        let selected_instance = document.instance_ids_at(Coord::new(1, 1, 1))[0];
        let mut session = Session::new();
        session.state.environment = Some(std::sync::Arc::new(editor::Environment::new(".", tree)));
        session.state.open_document(document);
        session.set_tool(Tool::Select);
        session.select_instance(Some(selected_instance));
        let mut state = ObjectTreePanel::new().expect("valid window key");

        state.reveal_selected_instance(&session);

        assert_eq!(state.selected, Some(selected_type));
        assert_eq!(state.reveal, Some(selected_type));
        assert_eq!(session.tool(), Tool::Select);
    }

    #[test]
    fn object_tree_rows_follow_expansion_and_render_a_clipped_search() {
        let _context = IMGUI_CONTEXT.lock().unwrap();
        let mut tree = ObjectTree::new();
        let thing = tree.register(&TreePath::parse("/atom/structure/thing"), Location::default());
        tree.register(&TreePath::parse("/atom/movable/item"), Location::default());
        let atom = tree.roots().atom.expect("registered /atom root");
        let mut context = dear_imgui_rs::Context::create();
        context
            .font_atlas()
            .try_claim_legacy_renderer()
            .expect("legacy renderer font atlas should be available")
            .build();
        context.io_mut().set_display_size([128.0, 128.0]);
        context.io_mut().set_delta_time(1.0 / 60.0);
        let ui = context.frame();

        let expanded = ui
            .window("object-tree-expanded")
            .build(|| {
                let mut rows = Vec::new();
                collect_type_rows(
                    ui,
                    &tree,
                    atom,
                    None,
                    true,
                    &mut rows,
                    &ObjectTreeRowOptions {
                        expand: true,
                        ..ObjectTreeRowOptions::default()
                    },
                );
                rows
            })
            .expect("test window should be visible");
        let paths = expanded
            .iter()
            .filter_map(|row| tree.get(row.id))
            .map(|decl| decl.path.to_string())
            .collect::<Vec<_>>();
        let parents = expanded.iter().map(|row| row.parent).collect::<Vec<_>>();
        let last_siblings = expanded.iter().map(|row| row.last_sibling).collect::<Vec<_>>();

        assert_eq!(
            paths,
            [
                "/atom",
                "/atom/movable",
                "/atom/movable/item",
                "/atom/structure",
                "/atom/structure/thing"
            ]
        );
        assert_eq!(parents, [None, Some(0), Some(1), Some(0), Some(3)]);
        assert_eq!(last_siblings, [true, false, true, true, true]);

        let opened_by_default = ui
            .window("object-tree-default-open")
            .build(|| {
                let mut rows = Vec::new();
                collect_type_rows(ui, &tree, atom, None, true, &mut rows, &ObjectTreeRowOptions::default());
                rows
            })
            .expect("test window should be visible");
        let default_paths = opened_by_default
            .iter()
            .filter_map(|row| tree.get(row.id))
            .map(|decl| decl.path.to_string())
            .collect::<Vec<_>>();
        assert_eq!(default_paths, ["/atom", "/atom/movable", "/atom/structure"]);

        let collapsed = ui
            .window("object-tree-collapsed")
            .build(|| {
                let atom_storage_id = ui.get_id("/atom");
                ui.with_current_state_storage(|mut storage| storage.set_bool(atom_storage_id, false));
                let mut rows = Vec::new();
                collect_type_rows(ui, &tree, atom, None, true, &mut rows, &ObjectTreeRowOptions::default());
                rows
            })
            .expect("test window should be visible");
        assert_eq!(
            collapsed,
            [ObjectTreeRow {
                id: atom,
                parent: None,
                leaf: false,
                last_sibling: true
            }]
        );

        let revealed = ui
            .window("object-tree-revealed")
            .build(|| {
                let atom_storage_id = ui.get_id("/atom");
                ui.with_current_state_storage(|mut storage| storage.set_bool(atom_storage_id, false));
                let mut rows = Vec::new();
                collect_type_rows(
                    ui,
                    &tree,
                    atom,
                    None,
                    true,
                    &mut rows,
                    &ObjectTreeRowOptions {
                        reveal: Some(thing),
                        ..ObjectTreeRowOptions::default()
                    },
                );
                rows
            })
            .expect("test window should be visible");
        let revealed_paths = revealed
            .iter()
            .filter_map(|row| tree.get(row.id))
            .map(|decl| decl.path.to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            revealed_paths,
            ["/atom", "/atom/movable", "/atom/structure", "/atom/structure/thing"]
        );

        let structure = tree.id_of(&TreePath::parse("/atom/structure")).unwrap();
        let filtered = ui
            .window("object-tree-filtered-reveal")
            .build(|| {
                let atom_storage_id = ui.get_id("/atom");
                ui.with_current_state_storage(|mut storage| storage.set_bool(atom_storage_id, false));
                let mut rows = Vec::new();
                collect_type_rows(
                    ui,
                    &tree,
                    atom,
                    None,
                    true,
                    &mut rows,
                    &ObjectTreeRowOptions {
                        type_filter: ObjectTreeTypeFilter {
                            custom: Some(structure),
                            ..ObjectTreeTypeFilter::default()
                        },
                        reveal: Some(thing),
                        ..ObjectTreeRowOptions::default()
                    },
                );
                rows
            })
            .expect("test window should be visible");
        assert_eq!(
            filtered,
            [ObjectTreeRow {
                id: atom,
                parent: None,
                leaf: false,
                last_sibling: true
            }]
        );

        let mut tree = ObjectTree::new();
        for index in 0..500 {
            tree.register(
                &TreePath::parse(&format!("/atom/common/type_{index}")),
                Location::default(),
            );
        }
        let mut session = Session::new();
        session.state.environment = Some(std::sync::Arc::new(editor::Environment::new(".", tree)));
        let mut state = ObjectTreePanel::new().expect("valid window key");
        state.search = String::from("atom");
        state.filter_revision = u64::MAX;
        let mut settings = Settings::default();

        state.draw(ui, &mut session, &mut settings, false);

        assert!(context.render_legacy().valid());
    }
}
