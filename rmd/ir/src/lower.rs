use core::{
    path::{PathFlags, TreePath},
    types::{Identifier, Value},
    vars,
};

use ast::{AST, Declaration, constant::fold, intrinsic::intrinsic_marker};
use objtree::{ObjectTree, TypeId};
use prelude::Intrinsic;

use crate::{IrModuleBuilder, Module, ProcIndex, opt::OptimizationTimings};

/// walks declarations in the same order as semantic analysis, so proc ids match its walk
pub fn lower(ast: &AST, tree: &ObjectTree, optimize: bool) -> (Module, OptimizationTimings) {
    let mut lowering = Lowering {
        ast,
        tree,
        builder: IrModuleBuilder::new(ast),
        index: ProcIndex::default(),
    };

    let root = TreePath::default();
    for declaration in &ast.declarations {
        lowering.walk(declaration, &root);
    }

    lowering.finish(optimize)
}

struct Lowering<'a> {
    ast: &'a AST,
    tree: &'a ObjectTree,
    builder: IrModuleBuilder<'a>,
    index: ProcIndex,
}

impl Lowering<'_> {
    fn walk(&mut self, declaration: &Declaration, prefix: &TreePath) {
        match declaration {
            Declaration::Type { path, body, .. } => {
                let full = prefix.concat(path);
                for child in body {
                    self.walk(child, &full);
                }
            },

            Declaration::Var {
                path,
                var_type,
                dimensions,
                initializer,
                location,
                ..
            } => {
                let owner = prefix.concat(&TreePath::new(path.declaration_owner().to_vec(), path.absolute));
                let Some(name) = path.name().cloned() else {
                    return;
                };

                let Some(id) = self.tree.id_of(&owner) else {
                    return;
                };

                let declares = path.flags.contains(PathFlags::IS_VAR);
                let declared_type = var_type
                    .clone()
                    .or_else(|| (!declares).then(|| self.declared_type(id, &name)).flatten());
                let runtime_initializer = match *initializer {
                    Some(expr) => (fold(self.ast, expr) == Value::Unevaluated).then(|| {
                        self.builder
                            .lower_initializer(owner, name.clone(), declared_type, expr, *location)
                    }),
                    None => (!dimensions.is_empty()).then(|| {
                        self.builder
                            .lower_sized_initializer(owner, name.clone(), dimensions, *location)
                    }),
                };

                self.index.set_initializer(id, name, runtime_initializer);
            },

            Declaration::Proc {
                path,
                params,
                body,
                variadic,
                location,
                ..
            } => {
                let owner = prefix.concat(&TreePath::new(path.declaration_owner().to_vec(), path.absolute));
                let Some(name) = path.name().cloned() else {
                    return;
                };

                let Some(id) = self.tree.id_of(&owner) else {
                    return;
                };

                let statements = body.as_deref().unwrap_or_default();
                let intrinsic = intrinsic_marker(self.ast, statements)
                    .ok()
                    .flatten()
                    .and_then(|id| Intrinsic::try_from(id).ok());
                let proc = self.builder.lower_proc_with_intrinsic(
                    owner,
                    name.clone(),
                    params,
                    *variadic,
                    statements,
                    intrinsic,
                    *location,
                );
                if let Some(previous) = self.index.body(id, &name)
                    && let Some(procedure) = self.builder.module.procs.get_mut(proc.0 as usize)
                {
                    procedure.previous = Some(previous);
                }

                self.index.set_body(id, name, body.as_ref().map(|_| proc));
            },

            Declaration::Override { name, value, location } => {
                if name.as_str() == vars::PARENT_TYPE {
                    return;
                }

                let Some(id) = self.tree.id_of(prefix) else {
                    return;
                };

                let declared_type = self.declared_type(id, name);
                let runtime_initializer = (fold(self.ast, *value) == Value::Unevaluated).then(|| {
                    self.builder
                        .lower_initializer(prefix.clone(), name.clone(), declared_type, *value, *location)
                });

                self.index.set_initializer(id, name.clone(), runtime_initializer);
            },
        }
    }

    fn declared_type(&self, id: TypeId, name: &Identifier) -> Option<TreePath> {
        self.tree
            .var_inherited(id, name)
            .and_then(|var| var.declared_type.clone())
    }

    fn finish(mut self, optimize: bool) -> (Module, OptimizationTimings) {
        for pending in self.builder.unresolved_new().to_vec() {
            let ty = self
                .tree
                .id_of(&pending.owner)
                .and_then(|id| self.tree.var_inherited(id, &pending.name))
                .or_else(|| self.tree.var_inherited(TypeId::ROOT, &pending.name))
                .and_then(|var| var.declared_type.clone());
            if let Some(ty) = ty {
                self.builder.resolve_new(pending.node, ty);
            }
        }

        let (mut module, timings) = self.builder.finish_with_optimizations(optimize);
        module.index = self.index;

        (module, timings)
    }
}

#[cfg(test)]
mod tests {
    macro_rules! fixture {
        ($path:literal) => {
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/", $path))
        };
    }

    use core::{path::TreePath, types::Value};

    use lexer::token::Token;
    use objtree::{ObjectTree, TypeId};
    use prelude::Intrinsic;

    use super::lower;
    use crate::{IrNode, Module};

    fn lower_source(source: &str) -> (ObjectTree, Module) {
        let (tokens, lexer_errors) = lexer::tokenize(source);
        assert!(lexer_errors.is_empty(), "{lexer_errors:?}");
        let tokens = tokens
            .into_iter()
            .map(|(token, location)| match token {
                Token::Identifier("TRUE") => (Token::True, location),
                Token::Identifier("FALSE") => (Token::False, location),
                token => (token, location),
            })
            .collect::<Vec<_>>();
        let ast = ast::parse(&tokens).expect("fixture should parse");
        let (tree, errors) = sema::analyze(&ast);
        assert!(errors.is_empty(), "{errors:?}");
        let (module, _) = lower(&ast, &tree, true);

        (tree, module)
    }

    #[test]
    fn retains_override_chains_and_runtime_initializers() {
        let (tree, module) = lower_source(fixture!("programs/retains_override_chains_and_runtime_initializers.dm"));
        let object = tree.id_of(&TreePath::parse("/obj")).expect("/obj type");
        let initializer = module
            .index
            .initializer(object, &"result".into())
            .expect("runtime initializer");
        let latest = module
            .index
            .inherited_body(&tree, object, &"source".into())
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
    fn intrinsic_markers_become_typed_ir_metadata() {
        let (tree, module) = lower_source(fixture!("programs/intrinsic_markers_become_typed_ir_metadata.dm"));
        let proc = module
            .index
            .inherited_body(&tree, TypeId::ROOT, &"test".into())
            .and_then(|proc| module.proc(proc))
            .expect("test proc");

        assert_eq!(proc.intrinsic, Some(Intrinsic::ListAdd));
    }

    fn new_type_of(source: &str, owner: &str, proc_name: &str) -> Option<String> {
        let (tokens, errors) = lexer::tokenize(source);
        assert!(errors.is_empty(), "{errors:?}");
        let ast = ast::parse(&tokens).expect("fixture should parse");
        let (tree, _) = sema::analyze(&ast);
        let (module, _) = lower(&ast, &tree, true);

        let body = tree
            .id_of(&TreePath::parse(owner))
            .and_then(|id| module.index.inherited_body(&tree, id, &proc_name.into()))
            .expect("fixture proc should exist");
        let blocks = module.proc(body).map(|proc| proc.body).expect("proc body");

        module
            .block(blocks)
            .expect("entry block")
            .iter()
            .find_map(|id| match module.node(*id) {
                Some(IrNode::New { ty, .. }) => Some(*ty),
                _ => None,
            })
            .expect("fixture should lower a new")
            .and_then(|ty| match module.node(ty) {
                Some(IrNode::Constant(value)) if let Value::Path(path) = &**value => Some(path.to_string()),
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
                fixture!("programs/untyped_new_resolves_against_the_finished_tree-4.dm"),
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
