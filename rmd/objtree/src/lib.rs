use core::{
    location::Location,
    path::TreePath,
    types::{Identifier, Value, VarModifiers},
};
use std::collections::HashMap;

pub struct ObjectTree {
    types: Vec<TypeDecl>,
    /// Keyed by path segments
    by_path: HashMap<Vec<Identifier>, TypeId>,
    roots: Roots,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Roots {
    pub datum: Option<TypeId>,
    pub atom: Option<TypeId>,
    /// `/atom/movable`
    pub movable: Option<TypeId>,
    pub obj: Option<TypeId>,
    pub mob: Option<TypeId>,
    pub turf: Option<TypeId>,
    pub area: Option<TypeId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TypeId(pub u32);

impl TypeId {
    pub const ROOT: Self = Self(0);
}

#[derive(Debug, Clone)]
pub struct TypeDecl {
    pub id: TypeId,
    pub path: TreePath,
    pub parent: Option<TypeId>,
    /// `parent_type = /some/path`
    pub parent_type: Option<TreePath>,
    pub children: Vec<TypeId>,
    pub vars: HashMap<Identifier, VarDecl>,
    pub procs: HashMap<Identifier, ProcDecl>,
    pub location: Location,
}

#[derive(Debug, Clone)]
pub struct VarDecl {
    pub name: Identifier,
    /// `var/mob/living/target`
    pub declared_type: Option<TreePath>,
    pub modifiers: VarModifiers,
    pub value: Value,
    pub location: Location,
}

#[derive(Debug, Clone)]
pub struct ProcDecl {
    pub name: Identifier,
    pub params: Vec<Identifier>,
    pub is_verb: bool,
    pub location: Location,
}

impl Default for ObjectTree {
    fn default() -> Self { Self::new() }
}

impl ObjectTree {
    pub fn new() -> Self {
        let root = TypeDecl {
            id: TypeId::ROOT,
            path: TreePath::default(),
            parent: None,
            parent_type: None,
            children: Vec::new(),
            vars: HashMap::new(),
            procs: HashMap::new(),
            location: Location::default(),
        };

        Self {
            by_path: HashMap::from([(Vec::new(), TypeId::ROOT)]),
            types: vec![root],
            roots: Roots::default(),
        }
    }

    pub fn roots(&self) -> Roots { self.roots }

    pub fn get(&self, id: TypeId) -> Option<&TypeDecl> { self.types.get(id.0 as usize) }

    pub fn get_mut(&mut self, id: TypeId) -> Option<&mut TypeDecl> { self.types.get_mut(id.0 as usize) }

    pub fn get_by_path(&self, path: &TreePath) -> Option<&TypeDecl> {
        self.by_path.get(&path.segments).and_then(|id| self.get(*id))
    }

    pub fn id_of(&self, path: &TreePath) -> Option<TypeId> { self.by_path.get(&path.segments).copied() }

    pub fn len(&self) -> usize { self.types.len() }

    pub fn is_empty(&self) -> bool { self.types.len() <= 1 }

    pub fn iter(&self) -> impl Iterator<Item = &TypeDecl> { self.types.iter() }

    /// `/obj/item/weapon/sword`
    pub fn register(&mut self, path: &TreePath, location: Location) -> TypeId {
        if let Some(id) = self.by_path.get(&path.segments) {
            return *id;
        }

        let mut parent = TypeId::ROOT;
        let mut current = TreePath::default();

        for segment in &path.segments {
            current = current.join(segment.clone());
            parent = match self.by_path.get(&current.segments) {
                Some(id) => *id,
                None => {
                    let id = TypeId(self.types.len() as u32);
                    self.types.push(TypeDecl {
                        id,
                        path: current.clone(),
                        parent: Some(parent),
                        parent_type: None,
                        children: Vec::new(),
                        vars: HashMap::new(),
                        procs: HashMap::new(),
                        location,
                    });

                    self.types[parent.0 as usize].children.push(id);
                    self.by_path.insert(current.segments.clone(), id);
                    self.note_root(id, &current.segments);

                    id
                },
            };
        }

        parent
    }

    fn note_root(&mut self, id: TypeId, segments: &[Identifier]) {
        let slot = match segments {
            [datum] if datum.0 == "datum" => &mut self.roots.datum,
            [atom] if atom.0 == "atom" => &mut self.roots.atom,
            [obj] if obj.0 == "obj" => &mut self.roots.obj,
            [mob] if mob.0 == "mob" => &mut self.roots.mob,
            [turf] if turf.0 == "turf" => &mut self.roots.turf,
            [area] if area.0 == "area" => &mut self.roots.area,
            [atom, movable] if atom.0 == "atom" && movable.0 == "movable" => &mut self.roots.movable,
            _ => return,
        };

        *slot = Some(id);
    }

    /// `parent_type = /some/path`
    pub fn resolve_parent_types(&mut self) {
        let overrides: Vec<(TypeId, TreePath)> = self
            .types
            .iter()
            .filter_map(|decl| decl.parent_type.clone().map(|path| (decl.id, path)))
            .collect();

        for (id, path) in overrides {
            let Some(new_parent) = self.by_path.get(&path.segments).copied() else {
                continue;
            };

            if let Some(old_parent) = self.types[id.0 as usize].parent {
                self.types[old_parent.0 as usize].children.retain(|child| *child != id);
            }

            self.types[id.0 as usize].parent = Some(new_parent);
            self.types[new_parent.0 as usize].children.push(id);
        }
    }

    pub fn ancestors(&self, id: TypeId) -> impl Iterator<Item = &TypeDecl> {
        let mut next = Some(id);

        std::iter::from_fn(move || {
            let decl = self.get(next?)?;
            next = decl.parent;

            Some(decl)
        })
    }

    pub fn var(&self, id: TypeId, name: &Identifier) -> Option<&VarDecl> { self.get(id)?.vars.get(name) }

    pub fn var_inherited(&self, id: TypeId, name: &Identifier) -> Option<&VarDecl> {
        self.ancestors(id).find_map(|decl| decl.vars.get(name))
    }

    pub fn proc_inherited(&self, id: TypeId, name: &Identifier) -> Option<&ProcDecl> {
        self.ancestors(id).find_map(|decl| decl.procs.get(name))
    }

    pub fn is_subtype_of(&self, id: TypeId, ancestor: TypeId) -> bool {
        self.ancestors(id).any(|decl| decl.id == ancestor)
    }

    pub fn descendants(&self, id: TypeId) -> Vec<TypeId> {
        let mut out = Vec::new();
        let mut stack = vec![id];

        while let Some(current) = stack.pop() {
            out.push(current);

            if let Some(decl) = self.get(current) {
                stack.extend(decl.children.iter().rev().copied());
            }
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use core::{location::Location, path::TreePath};

    use crate::{ObjectTree, Roots};

    #[test]
    fn registers_intermediate_types() {
        let mut tree = ObjectTree::new();
        let id = tree.register(&TreePath::parse("/obj/item/sword"), Location::default());

        assert!(tree.get_by_path(&TreePath::parse("/obj")).is_some());
        assert!(tree.get_by_path(&TreePath::parse("/obj/item")).is_some());
        assert_eq!(tree.ancestors(id).count(), 4);
    }

    #[test]
    fn notes_builtin_roots_as_they_appear() {
        let mut tree = ObjectTree::new();
        assert_eq!(tree.roots(), Roots::default());

        let station = tree.register(&TreePath::parse("/area/station/hallway"), Location::default());
        let sword = tree.register(&TreePath::parse("/obj/item/sword"), Location::default());
        tree.register(&TreePath::parse("/atom/movable"), Location::default());

        let roots = tree.roots();
        assert_eq!(roots.area, tree.id_of(&TreePath::parse("/area")));
        assert_eq!(roots.obj, tree.id_of(&TreePath::parse("/obj")));
        assert_eq!(roots.movable, tree.id_of(&TreePath::parse("/atom/movable")));
        assert_eq!(roots.mob, None);

        assert!(roots.area.is_some_and(|area| tree.is_subtype_of(station, area)));
        assert!(roots.area.is_some_and(|area| !tree.is_subtype_of(sword, area)));
    }

    /// `/obj/fake_area { parent_type = /area }`
    #[test]
    fn a_reparented_type_is_still_seen_as_its_new_root() {
        let mut tree = ObjectTree::new();
        tree.register(&TreePath::parse("/area"), Location::default());

        let fake = tree.register(&TreePath::parse("/obj/fake_area"), Location::default());
        if let Some(decl) = tree.get_mut(fake) {
            decl.parent_type = Some(TreePath::parse("/area"));
        }

        tree.resolve_parent_types();

        let roots = tree.roots();
        assert!(roots.area.is_some_and(|area| tree.is_subtype_of(fake, area)));
        assert!(roots.obj.is_some_and(|obj| !tree.is_subtype_of(fake, obj)));
    }
}
