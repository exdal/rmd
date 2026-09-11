pub mod constant;
pub mod error;

use core::{
    path::TreePath,
    types::{Identifier, Value},
};

use ast::{AST, Declaration, ProcKind};
use objtree::{ObjectTree, ProcDecl, VarDecl};

use crate::{
    constant::fold,
    error::{SemaError, SemaErrorKind},
};

/// `AST` to `ObjectTree`
#[derive(Default)]
pub struct Analyzer {
    tree: ObjectTree,
    errors: Vec<SemaError>,
}

impl Analyzer {
    pub fn new() -> Self {
        Self {
            tree: ObjectTree::new(),
            errors: Vec::new(),
        }
    }

    pub fn errors(&self) -> &[SemaError] { &self.errors }

    pub fn add(&mut self, ast: &AST) {
        let root = TreePath::default();
        for declaration in &ast.declarations {
            self.walk(ast, declaration, &root);
        }
    }

    pub fn finish(mut self) -> (ObjectTree, Vec<SemaError>) {
        self.tree.resolve_parent_types();

        (self.tree, self.errors)
    }

    fn walk(&mut self, ast: &AST, declaration: &Declaration, prefix: &TreePath) {
        match declaration {
            Declaration::Type { path, body, location } => {
                let full = prefix.concat(path);
                self.tree.register(&full, *location);

                for child in body {
                    self.walk(ast, child, &full);
                }
            },

            Declaration::Var {
                path,
                var_type,
                modifiers,
                initializer,
                location,
                ..
            } => {
                let owner = prefix.concat(&TreePath::new(path.declaration_owner().to_vec(), path.absolute));
                let id = self.tree.register(&owner, *location);

                let Some(name) = path.name().cloned() else {
                    return;
                };

                let value = initializer.map(|expr| fold(ast, expr)).unwrap_or(Value::Null);
                let Some(decl) = self.tree.get_mut(id) else {
                    return;
                };

                if decl.vars.contains_key(&name) {
                    self.errors
                        .push(SemaError::new(SemaErrorKind::DuplicateVar(name.clone()), *location));
                }

                decl.vars.insert(
                    name.clone(),
                    VarDecl {
                        name,
                        declared_type: var_type.clone(),
                        modifiers: *modifiers,
                        value,
                        location: *location,
                    },
                );
            },

            Declaration::Proc {
                path,
                kind,
                params,
                location,
                ..
            } => {
                let owner = prefix.concat(&TreePath::new(path.declaration_owner().to_vec(), path.absolute));
                let id = self.tree.register(&owner, *location);

                let Some(name) = path.name().cloned() else {
                    return;
                };

                let Some(decl) = self.tree.get_mut(id) else {
                    return;
                };

                decl.procs.insert(
                    name.clone(),
                    ProcDecl {
                        name,
                        params: params.iter().map(|param| param.spec.name.clone()).collect(),
                        is_verb: *kind == ProcKind::Verb,
                        location: *location,
                    },
                );
            },

            Declaration::Override { name, value, location } => {
                let id = self.tree.register(prefix, *location);
                let folded = fold(ast, *value);

                // `parent_type = /some/path`
                if name.as_str() == "parent_type" {
                    if let Value::Path(path) = &folded
                        && let Some(decl) = self.tree.get_mut(id)
                    {
                        decl.parent_type = Some(path.clone());
                    }

                    return;
                }

                let declared_type = self
                    .tree
                    .var_inherited(id, name)
                    .and_then(|var| var.declared_type.clone());

                let Some(decl) = self.tree.get_mut(id) else {
                    return;
                };

                decl.vars.insert(
                    name.clone(),
                    VarDecl {
                        name: name.clone(),
                        declared_type,
                        modifiers: Default::default(),
                        value: folded,
                        location: *location,
                    },
                );
            },
        }
    }
}

pub fn check_undeclared_overrides(tree: &ObjectTree) -> Vec<SemaError> {
    let _ = tree;

    // TODO: needs `VarDecl` to remember whether it was a declaration or an override

    Vec::new()
}

pub fn analyze(ast: &AST) -> (ObjectTree, Vec<SemaError>) {
    let mut analyzer = Analyzer::new();
    analyzer.add(ast);

    analyzer.finish()
}

pub fn lookup_var<'a>(tree: &'a ObjectTree, path: &TreePath, name: &Identifier) -> Option<&'a Value> {
    let id = tree.id_of(path)?;

    tree.var_inherited(id, name).map(|var| &var.value)
}
