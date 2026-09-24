use core::path::TreePath;

use dear_imgui_rs::Ui;
use objtree::ObjectTree;

pub(super) const MAX_CUSTOM_FILL_SEARCH_RESULTS: usize = 50;

pub(super) fn draw_type_path_search(
    ui: &Ui, tree: Option<&ObjectTree>, query: &mut String, input_id: &str, limit: usize,
    allowed: impl Fn(&ObjectTree, &TreePath) -> bool, enabled: impl Fn(&TreePath) -> bool,
) -> Option<TreePath> {
    ui.set_next_item_width(320.0);
    ui.input_text(input_id, query).hint("Search type paths").build();
    if query.trim().is_empty() {
        ui.text_disabled("Type a path to search");
        return None;
    }
    let Some(tree) = tree else {
        ui.text_disabled("No environment loaded");
        return None;
    };
    let matches = matching_type_paths_up_to_filtered(tree, query, limit, |path| allowed(tree, path));
    if matches.is_empty() {
        ui.text_disabled("No matching types");
    }
    matches
        .into_iter()
        .find(|path| ui.menu_item_enabled_selected_no_shortcut(path.to_string(), false, enabled(path)))
}

pub(super) fn matching_type_paths(tree: &ObjectTree, query: &str) -> Vec<TreePath> {
    matching_type_paths_up_to(tree, query, MAX_CUSTOM_FILL_SEARCH_RESULTS)
}

// im not sure why every single fucking icons appear not centered fuck you

fn matching_type_paths_up_to(tree: &ObjectTree, query: &str, limit: usize) -> Vec<TreePath> {
    matching_type_paths_up_to_filtered(tree, query, limit, |_| true)
}

pub(super) fn matching_type_paths_up_to_filtered(
    tree: &ObjectTree, query: &str, limit: usize, allowed: impl Fn(&TreePath) -> bool,
) -> Vec<TreePath> {
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return Vec::new();
    }

    let mut matches = tree
        .iter()
        .filter(|decl| allowed(&decl.path))
        .filter(|decl| decl.path.to_string().to_ascii_lowercase().contains(&query))
        .map(|decl| decl.path.clone())
        .take(limit)
        .collect::<Vec<_>>();
    matches.sort_by_key(ToString::to_string);

    matches
}

#[cfg(test)]
mod tests {
    use core::{location::Location, path::TreePath};

    use objtree::ObjectTree;

    use super::matching_type_paths;

    #[test]
    fn custom_fill_search_includes_parent_types_and_ignores_case() {
        let mut tree = ObjectTree::new();
        tree.register(&TreePath::parse("/obj/structure/table"), Location::default());
        tree.register(&TreePath::parse("/turf/closed/wall"), Location::default());

        assert_eq!(
            matching_type_paths(&tree, "STRUCT")
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ["/obj/structure", "/obj/structure/table"]
        );
        assert_eq!(
            matching_type_paths(&tree, "wall")
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ["/turf/closed/wall"]
        );
        assert!(matching_type_paths(&tree, "  ").is_empty());
    }
}
