pub mod constant;
pub mod error;

use core::{
    path::TreePath,
    types::{Identifier, Value},
};

use ast::{AST, Declaration};
use objtree::{ObjectTree, ProcDecl, VarDecl};

use crate::{
    constant::fold,
    error::{SemaError, SemaErrorKind},
};

/// `AST` to `ObjectTree` and executable IR.
pub struct Analyzer<'a> {
    ast: &'a AST,
    tree: ObjectTree,
    module: ir::IrModuleBuilder<'a>,
    errors: Vec<SemaError>,
}

impl<'a> Analyzer<'a> {
    pub fn new(ast: &'a AST) -> Self {
        Self {
            ast,
            tree: ObjectTree::new(),
            module: ir::IrModuleBuilder::new(ast),
            errors: Vec::new(),
        }
    }

    pub fn errors(&self) -> &[SemaError] { &self.errors }

    pub fn add(&mut self) {
        let ast = self.ast;
        let root = TreePath::default();
        for declaration in &ast.declarations {
            self.walk(declaration, &root);
        }
    }

    pub fn finish(mut self) -> (ObjectTree, ir::Module, Vec<SemaError>) {
        self.tree.resolve_parent_types();

        (self.tree, self.module.finish(), self.errors)
    }

    fn walk(&mut self, declaration: &Declaration, prefix: &TreePath) {
        match declaration {
            Declaration::Type { path, body, location } => {
                let full = prefix.concat(path);
                self.tree.register(&full, *location);

                for child in body {
                    self.walk(child, &full);
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

                let value = initializer.map(|expr| fold(self.ast, expr)).unwrap_or(Value::Null);
                let runtime_initializer = initializer.filter(|_| value == Value::Unevaluated).map(|expr| {
                    self.module
                        .lower_initializer(owner.clone(), name.clone(), var_type.clone(), expr, *location)
                });
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
                        initializer: runtime_initializer,
                        location: *location,
                    },
                );
            },

            Declaration::Proc {
                path,
                kind,
                params,
                body,
                variadic,
                return_type,
                location,
            } => {
                let owner = prefix.concat(&TreePath::new(path.declaration_owner().to_vec(), path.absolute));
                let id = self.tree.register(&owner, *location);

                let Some(name) = path.name().cloned() else {
                    return;
                };

                let proc_id = self.module.lower_proc(
                    owner,
                    name.clone(),
                    params,
                    *variadic,
                    body.as_deref().unwrap_or_default(),
                    *location,
                );
                if let Some(previous) = self
                    .tree
                    .get(id)
                    .and_then(|decl| decl.procs.get(&name))
                    .and_then(|proc| proc.body)
                    && let Some(proc) = self.module.module.procs.get_mut(proc_id.0 as usize)
                {
                    proc.previous = Some(previous);
                }
                let lowered_params = self
                    .module
                    .module
                    .proc(proc_id)
                    .map(|proc| proc.params.clone())
                    .unwrap_or_default();
                let Some(decl) = self.tree.get_mut(id) else {
                    return;
                };

                decl.procs.insert(
                    name.clone(),
                    ProcDecl {
                        name,
                        params: lowered_params,
                        body: body.as_ref().map(|_| proc_id),
                        kind: *kind,
                        variadic: *variadic,
                        return_type: return_type.clone(),
                        location: *location,
                    },
                );
            },

            Declaration::Override { name, value, location } => {
                let id = self.tree.register(prefix, *location);
                let folded = fold(self.ast, *value);

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

                let runtime_initializer = (folded == Value::Unevaluated).then(|| {
                    self.module.lower_initializer(
                        prefix.clone(),
                        name.clone(),
                        declared_type.clone(),
                        *value,
                        *location,
                    )
                });
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
                        initializer: runtime_initializer,
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

pub fn analyze(ast: &AST) -> (ObjectTree, ir::Module, Vec<SemaError>) {
    let mut analyzer = Analyzer::new(ast);
    analyzer.add();

    analyzer.finish()
}

pub fn lookup_var<'a>(tree: &'a ObjectTree, path: &TreePath, name: &Identifier) -> Option<&'a Value> {
    let id = tree.id_of(path)?;

    tree.var_inherited(id, name).map(|var| &var.value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retains_override_chains_and_runtime_initializers() {
        let source = "/obj\n\tvar/result = source()\n\tproc/source()\n\t\treturn 1\n\tproc/source()\n\t\treturn 2\n";
        let (tokens, lexer_errors) = lexer::tokenize(source);
        assert!(lexer_errors.is_empty());
        let ast = ast::parse(&tokens).expect("fixture should parse");

        let (tree, module, sema_errors) = analyze(&ast);
        assert!(sema_errors.is_empty());
        let object = tree.id_of(&TreePath::parse("/obj")).expect("/obj type");
        let initializer = tree
            .var(object, &"result".into())
            .and_then(|var| var.initializer)
            .expect("runtime initializer");
        let latest = tree
            .proc_inherited(object, &"source".into())
            .and_then(|proc| proc.body)
            .expect("latest proc body");
        let previous = module
            .proc(latest)
            .and_then(|proc| proc.previous)
            .expect("previous proc body");

        assert!(module.proc(initializer).is_some());
        assert_eq!(
            module.proc(previous).map(|proc| &proc.name),
            module.proc(latest).map(|proc| &proc.name)
        );
    }
}
