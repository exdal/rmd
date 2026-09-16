pub mod constant;
pub mod error;

use core::{
    path::TreePath,
    types::{Identifier, Value},
};

use ast::{AST, Declaration};
use objtree::{ObjectTree, ProcDecl, TypeId, VarDecl};

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
        self.resolve_new_types();

        (self.tree, self.module.finish(), self.errors)
    }

    fn resolve_new_types(&mut self) {
        for pending in self.module.unresolved_new().to_vec() {
            let Some(ty) = self.declared_type(&pending.owner, &pending.name) else {
                continue;
            };

            self.module.resolve_new(pending.node, ty);
        }
    }

    fn declared_type(&self, owner: &TreePath, name: &Identifier) -> Option<TreePath> {
        self.tree
            .id_of(owner)
            .and_then(|id| self.tree.var_inherited(id, name))
            .or_else(|| self.tree.var_inherited(TypeId::ROOT, name))
            .and_then(|var| var.declared_type.clone())
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

    fn new_type_of(source: &str, owner: &str, proc_name: &str) -> Option<String> {
        let (tokens, errors) = lexer::tokenize(source);
        assert!(errors.is_empty(), "{errors:?}");
        let ast = ast::parse(&tokens).expect("fixture should parse");
        let (tree, module, _) = analyze(&ast);

        let body = tree
            .id_of(&TreePath::parse(owner))
            .and_then(|id| tree.proc_inherited(id, &proc_name.into()))
            .and_then(|proc| proc.body)
            .expect("fixture proc should exist");
        let blocks = module.proc(body).map(|proc| proc.body).expect("proc body");

        module
            .block(blocks)
            .expect("entry block")
            .iter()
            .find_map(|id| match module.node(*id) {
                Some(ir::IrNode::New { ty, .. }) => Some(*ty),
                _ => None,
            })
            .expect("fixture should lower a new")
            .and_then(|ty| match module.node(ty) {
                Some(ir::IrNode::Constant(Value::Path(path))) => Some(path.to_string()),
                _ => None,
            })
    }

    /// `x = new` reads its type off `x`, which for an object var or a global is only knowable once
    /// every file has reopened every type.
    #[test]
    fn untyped_new_resolves_against_the_finished_tree() {
        // `TreePath::parse` would carry `IS_DATUM`, and the tree keys on segments alone.
        let tracy = String::from("/datum/tracy");

        // An object var, declared after the proc that assigns it.
        assert_eq!(
            new_type_of(
                "/datum/tracy\n/datum/holder\n\tproc/go()\n\t\tchild = new\n/datum/holder\n\tvar/datum/tracy/child\n",
                "/datum/holder",
                "go",
            ),
            Some(tracy.clone())
        );

        // An inherited object var, reachable only after `parent_type` is resolved.
        assert_eq!(
            new_type_of(
                "/datum/tracy\n/datum/base\n\tvar/datum/tracy/inherited\n/datum/holder\n\tparent_type = \
                 /datum/base\n\tproc/go()\n\t\tinherited = new\n",
                "/datum/holder",
                "go",
            ),
            Some(tracy.clone())
        );

        // A file-scope global.
        assert_eq!(
            new_type_of(
                "/datum/tracy\nvar/global/datum/tracy/Tracy\n/datum/holder/proc/go()\n\t\tTracy = new\n",
                "/datum/holder",
                "go",
            ),
            Some(tracy.clone())
        );

        // `src.x`, whose type lives on the owner the same way a bare name's does.
        assert_eq!(
            new_type_of(
                "/datum/tracy\n/datum/holder\n\tvar/datum/tracy/child\n\tproc/go()\n\t\tsrc.child = new\n",
                "/datum/holder",
                "go",
            ),
            Some(tracy)
        );
    }

    /// A local still resolves during lowering, and a name that is no var at all stays untyped rather
    /// than picking up someone else's type.
    #[test]
    fn untyped_new_leaves_unresolvable_targets_alone() {
        assert_eq!(
            new_type_of(
                "/datum/tracy\n/datum/holder/proc/go()\n\t\tvar/datum/tracy/local = new\n",
                "/datum/holder",
                "go",
            ),
            Some(String::from("/datum/tracy"))
        );

        assert_eq!(
            new_type_of("/datum/holder/proc/go()\n\t\tunknown = new\n", "/datum/holder", "go"),
            None
        );
    }
}
