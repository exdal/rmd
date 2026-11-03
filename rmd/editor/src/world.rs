use core::{
    path::TreePath,
    types::{Identifier, Value},
    vars,
};

use defines::SIDE_MAP;
use objtree::{ObjectTree, TypeId};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Perspective {
    #[default]
    TopDown,
    Side,
}

impl Perspective {
    fn new(tree: &ObjectTree) -> Self {
        let map_format = tree
            .roots()
            .world
            .and_then(|world| tree.var_inherited(world, &Identifier::from("map_format")))
            .and_then(|var| var.value.as_num());

        match map_format {
            Some(format) if format == SIDE_MAP as f32 => Self::Side,
            _ => Self::TopDown,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct World {
    pub name: Option<String>,
    pub base_tile: Option<BaseTile>,
    pub perspective: Perspective,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BaseTile {
    pub turf: TreePath,
    pub area: TreePath,
}

impl World {
    pub fn new(tree: &ObjectTree) -> Self {
        let name = tree
            .roots()
            .world
            .and_then(|world| tree.var_inherited(world, &Identifier::from(vars::NAME)))
            .and_then(|var| var.value.as_text())
            .filter(|name| !name.is_empty())
            .map(str::to_owned);

        Self {
            name,
            base_tile: BaseTile::new(tree),
            perspective: Perspective::new(tree),
        }
    }
}

impl BaseTile {
    fn new(tree: &ObjectTree) -> Option<Self> {
        let roots = tree.roots();

        Some(Self {
            turf: default_path(tree, "turf", roots.turf?)?,
            area: default_path(tree, "area", roots.area?)?,
        })
    }
}

fn default_path(tree: &ObjectTree, variable: &str, root: TypeId) -> Option<TreePath> {
    let configured = tree
        .roots()
        .world
        .and_then(|world| tree.var_inherited(world, &Identifier::from(variable)))
        .and_then(|var| match &var.value {
            Value::Path(path) => Some(path),
            _ => None,
        })
        .and_then(|path| tree.id_of(path).map(|id| (path, id)))
        .filter(|(_, id)| tree.is_subtype_of(*id, root))
        .map(|(path, _)| path.clone());

    configured.or_else(|| tree.get(root).map(|decl| TreePath::parse(&decl.path.to_string())))
}

#[cfg(test)]
mod tests {
    use core::{
        location::Location,
        path::TreePath,
        types::{Identifier, Value, VarModifiers},
    };

    use objtree::{ObjectTree, VarDecl};

    use super::{Perspective, World};

    fn tree_with_map_format(format: Option<f32>) -> ObjectTree {
        let mut tree = ObjectTree::new();
        let id = tree.register(&TreePath::parse("/world"), Location::default());
        if let Some(format) = format {
            let name = Identifier::from("map_format");
            tree.get_mut(id).unwrap().vars.insert(
                name.clone(),
                VarDecl {
                    name,
                    declared_type: None,
                    modifiers: VarModifiers::default(),
                    value: Value::Num(format),
                    declared: true,
                    location: Location::default(),
                    resolved_type: None,
                    shared: None,
                },
            );
        }

        tree
    }

    #[test]
    fn side_map_format_gives_a_side_perspective() {
        assert_eq!(
            World::new(&tree_with_map_format(Some(2.0))).perspective,
            Perspective::Side
        );
    }

    #[test]
    fn other_or_missing_map_formats_stay_top_down() {
        assert_eq!(
            World::new(&tree_with_map_format(None)).perspective,
            Perspective::TopDown
        );
        assert_eq!(
            World::new(&tree_with_map_format(Some(0.0))).perspective,
            Perspective::TopDown
        );
        assert_eq!(
            World::new(&tree_with_map_format(Some(5.0))).perspective,
            Perspective::TopDown
        );
    }
}
