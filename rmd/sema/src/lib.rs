pub mod constant;
pub mod error;

use core::{
    path::{PathFlags, TreePath},
    types::{Identifier, Value},
};

use ast::{AST, Declaration, Expression, Literal, SettingMode, Statement};
use objtree::{ObjectTree, ProcDecl, TypeId, VarDecl};
use prelude::Intrinsic;

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

    pub fn finish(self, optimize: bool) -> (ObjectTree, ir::Module, Vec<SemaError>) {
        let (tree, module, errors, _) = self.finish_with_optimizations(optimize);

        (tree, module, errors)
    }

    pub fn finish_with_optimizations(
        mut self, enabled: bool,
    ) -> (ObjectTree, ir::Module, Vec<SemaError>, ir::opt::OptimizationTimings) {
        self.tree.resolve_parent_types();
        self.resolve_new_types();
        let (module, timings) = self.module.finish_with_optimizations(enabled);

        (self.tree, module, self.errors, timings)
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
                dimensions,
                initializer,
                location,
                ..
            } => {
                let owner = prefix.concat(&TreePath::new(path.declaration_owner().to_vec(), path.absolute));
                let id = self.tree.register(&owner, *location);

                let Some(name) = path.name().cloned() else {
                    return;
                };

                let declares = path.flags.contains(PathFlags::IS_VAR);
                let inherited = (!declares)
                    .then(|| self.tree.var_inherited(id, &name).cloned())
                    .flatten();
                let declared_type = var_type
                    .clone()
                    .or_else(|| inherited.as_ref().and_then(|var| var.declared_type.clone()));
                let modifiers = if declares {
                    *modifiers
                } else {
                    inherited.as_ref().map(|var| var.modifiers).unwrap_or(*modifiers)
                };
                let sized = initializer.is_none() && !dimensions.is_empty();
                let value = match *initializer {
                    Some(expr) => fold(self.ast, expr),
                    None if sized => Value::Unevaluated,
                    None => Value::Null,
                };
                let runtime_initializer = match *initializer {
                    Some(expr) => (value == Value::Unevaluated).then(|| {
                        self.module.lower_initializer(
                            owner.clone(),
                            name.clone(),
                            declared_type.clone(),
                            expr,
                            *location,
                        )
                    }),
                    None => sized.then(|| {
                        self.module
                            .lower_sized_initializer(owner.clone(), name.clone(), dimensions, *location)
                    }),
                };
                let Some(decl) = self.tree.get_mut(id) else {
                    return;
                };

                if declares && decl.vars.contains_key(&name) {
                    self.errors
                        .push(SemaError::new(SemaErrorKind::DuplicateVar(name.clone()), *location));
                }

                let already_declared = decl.vars.get(&name).is_some_and(|var| var.declared);

                decl.vars.insert(
                    name.clone(),
                    VarDecl {
                        name,
                        declared_type,
                        modifiers,
                        value,
                        initializer: runtime_initializer,
                        declared: declares || already_declared,
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
                let body_statements = body.as_deref().unwrap_or_default();
                let intrinsic = self.intrinsic(body_statements, *location);

                let proc_id = self.module.lower_proc_with_intrinsic(
                    owner,
                    name.clone(),
                    params,
                    *variadic,
                    body_statements,
                    intrinsic,
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

                let inherited = self.tree.var_inherited(id, name).cloned();
                let declared_type = inherited.as_ref().and_then(|var| var.declared_type.clone());
                let modifiers = inherited.as_ref().map(|var| var.modifiers).unwrap_or_default();

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

                let already_declared = decl.vars.get(name).is_some_and(|var| var.declared);

                decl.vars.insert(
                    name.clone(),
                    VarDecl {
                        name: name.clone(),
                        declared_type,
                        modifiers,
                        value: folded,
                        initializer: runtime_initializer,
                        declared: already_declared,
                        location: *location,
                    },
                );
            },
        }
    }

    fn intrinsic(&mut self, body: &[Statement], location: core::location::Location) -> Option<Intrinsic> {
        let markers = body
            .iter()
            .filter_map(|statement| match statement {
                Statement::Setting { name, mode, value } if name.as_str() == "__demir_intrin" => Some((*mode, *value)),
                _ => None,
            })
            .collect::<Vec<_>>();

        if markers.len() > 1 {
            self.errors
                .push(SemaError::new(SemaErrorKind::DuplicateIntrinsic, location));
            return None;
        }

        let Some((SettingMode::Assign, id)) = markers.first() else {
            if !markers.is_empty() {
                self.errors
                    .push(SemaError::new(SemaErrorKind::InvalidIntrinsic, location));
            }
            return None;
        };
        let expression = self.ast.get_expr(*id)?;
        let Expression::Literal(Literal::Num(number)) = expression else {
            self.errors
                .push(SemaError::new(SemaErrorKind::InvalidIntrinsic, location));
            return None;
        };
        if !number.is_finite() || *number < 0.0 || number.fract() != 0.0 || *number > u16::MAX as f32 {
            self.errors
                .push(SemaError::new(SemaErrorKind::InvalidIntrinsic, location));
            return None;
        }

        let id = *number as u16;
        match Intrinsic::try_from(id) {
            Ok(intrinsic) => Some(intrinsic),
            Err(_) => {
                self.errors
                    .push(SemaError::new(SemaErrorKind::UnknownIntrinsic(id), location));
                None
            },
        }
    }
}

pub fn check_undeclared_overrides(tree: &ObjectTree) -> Vec<SemaError> {
    let mut errors = Vec::new();

    for decl in tree.iter() {
        let Some(parent) = decl.parent else {
            continue;
        };

        for var in decl.vars.values() {
            if !var.declared && tree.var_inherited(parent, &var.name).is_none() {
                errors.push(SemaError::new(
                    SemaErrorKind::UndeclaredVar(var.name.clone()),
                    var.location,
                ));
            }
        }
    }

    errors
}

pub fn analyze(ast: &AST, optimize: bool) -> (ObjectTree, ir::Module, Vec<SemaError>) {
    let mut analyzer = Analyzer::new(ast);
    analyzer.add();

    analyzer.finish(optimize)
}

pub fn analyze_with_optimizations(
    ast: &AST, enabled: bool,
) -> (ObjectTree, ir::Module, Vec<SemaError>, ir::opt::OptimizationTimings) {
    let mut analyzer = Analyzer::new(ast);
    analyzer.add();

    analyzer.finish_with_optimizations(enabled)
}

pub fn lookup_var<'a>(tree: &'a ObjectTree, path: &TreePath, name: &Identifier) -> Option<&'a Value> {
    let id = tree.id_of(path)?;

    tree.var_inherited(id, name).map(|var| &var.value)
}

#[cfg(test)]
mod tests {
    macro_rules! fixture {
        ($path:literal) => {
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/", $path))
        };
    }

    use super::*;

    fn analyze_source(source: &str) -> (ObjectTree, ir::Module, Vec<SemaError>) {
        let (tokens, lexer_errors) = lexer::tokenize(source);
        assert!(lexer_errors.is_empty(), "{lexer_errors:?}");
        let ast = ast::parse(&tokens).expect("fixture should parse");

        analyze(&ast, true)
    }

    #[test]
    fn retains_override_chains_and_runtime_initializers() {
        let source = fixture!("programs/retains_override_chains_and_runtime_initializers.dm");
        let (tree, module, sema_errors) = analyze_source(source);
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

    #[test]
    fn absolute_var_assignment_overrides_an_existing_declaration() {
        let source = fixture!("programs/absolute-var-assignment-overrides-an-existing-declaration.dm");
        let (tree, _, errors) = analyze_source(source);
        assert!(errors.is_empty(), "{errors:?}");
        let client = tree.id_of(&TreePath::parse("/client")).expect("/client type");
        let script = tree.var(client, &"script".into()).expect("script var");

        assert_eq!(script.value, Value::Text("override".into()));
    }

    #[test]
    fn repeated_var_declarations_remain_an_error() {
        let source = fixture!("programs/repeated-var-declarations-remain-an-error.dm");
        let (_, _, errors) = analyze_source(source);

        assert!(matches!(
            errors.as_slice(),
            [SemaError {
                kind: SemaErrorKind::DuplicateVar(name),
                ..
            }] if name.as_str() == "script"
        ));
    }

    #[test]
    fn undeclared_overrides_are_reported() {
        let source = fixture!("programs/undeclared_overrides_are_reported.dm");
        let (tree, _, errors) = analyze_source(source);
        assert!(errors.is_empty(), "{errors:?}");

        let errors = check_undeclared_overrides(&tree);
        assert!(
            matches!(
                errors.as_slice(),
                [SemaError {
                    kind: SemaErrorKind::UndeclaredVar(name),
                    ..
                }] if name.as_str() == "nonexistent"
            ),
            "{errors:?}"
        );
    }

    #[test]
    fn intrinsic_markers_become_typed_ir_metadata() {
        let (tree, module, errors) = analyze_source(fixture!("programs/intrinsic_markers_become_typed_ir_metadata.dm"));
        assert!(errors.is_empty(), "{errors:?}");
        let proc = tree
            .proc_inherited(TypeId::ROOT, &"test".into())
            .and_then(|proc| proc.body)
            .and_then(|proc| module.proc(proc))
            .expect("test proc");

        assert_eq!(proc.intrinsic, Some(Intrinsic::ListAdd));
    }

    #[test]
    fn intrinsic_markers_reject_malformed_unknown_and_duplicate_ids() {
        let (_, _, malformed) = analyze_source(fixture!(
            "programs/intrinsic_markers_reject_malformed_unknown_and_duplicate_ids.dm"
        ));
        assert!(matches!(
            malformed.as_slice(),
            [SemaError {
                kind: SemaErrorKind::InvalidIntrinsic,
                ..
            }]
        ));

        let (_, _, wrong_mode) = analyze_source(fixture!(
            "programs/intrinsic_markers_reject_malformed_unknown_and_duplicate_ids-2.dm"
        ));
        assert!(matches!(
            wrong_mode.as_slice(),
            [SemaError {
                kind: SemaErrorKind::InvalidIntrinsic,
                ..
            }]
        ));

        let (_, _, unknown) = analyze_source(fixture!(
            "programs/intrinsic_markers_reject_malformed_unknown_and_duplicate_ids-3.dm"
        ));
        assert!(matches!(
            unknown.as_slice(),
            [SemaError {
                kind: SemaErrorKind::UnknownIntrinsic(65535),
                ..
            }]
        ));

        let (_, _, duplicate) = analyze_source(fixture!(
            "programs/intrinsic_markers_reject_malformed_unknown_and_duplicate_ids-4.dm"
        ));
        assert!(matches!(
            duplicate.as_slice(),
            [SemaError {
                kind: SemaErrorKind::DuplicateIntrinsic,
                ..
            }]
        ));
    }

    fn new_type_of(source: &str, owner: &str, proc_name: &str) -> Option<String> {
        let (tokens, errors) = lexer::tokenize(source);
        assert!(errors.is_empty(), "{errors:?}");
        let ast = ast::parse(&tokens).expect("fixture should parse");
        let (tree, module, _) = analyze(&ast, true);

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
                fixture!("programs/untyped_new_resolves_against_the_finished_tree.dm"),
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
                fixture!("programs/untyped_new_resolves_against_the_finished_tree-2.dm"),
                "/datum/holder",
                "go",
            ),
            Some(tracy.clone())
        );

        // `src.x`, whose type lives on the owner the same way a bare name's does.
        assert_eq!(
            new_type_of(
                fixture!("programs/untyped_new_resolves_against_the_finished_tree-3.dm"),
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
                fixture!("programs/untyped_new_leaves_unresolvable_targets_alone.dm"),
                "/datum/holder",
                "go",
            ),
            Some(String::from("/datum/tracy"))
        );

        assert_eq!(
            new_type_of(
                fixture!("programs/untyped_new_leaves_unresolvable_targets_alone-2.dm"),
                "/datum/holder",
                "go"
            ),
            None
        );
    }
}
