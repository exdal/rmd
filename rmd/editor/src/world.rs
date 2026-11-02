use core::{
    path::TreePath,
    types::{Identifier, Value},
    vars,
};

use objtree::{ObjectTree, TypeId};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct World {
    pub name: Option<String>,
    pub base_tile: Option<BaseTile>,
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
