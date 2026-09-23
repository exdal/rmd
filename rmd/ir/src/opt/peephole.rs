use core::types::{IrNodeId, Value};
use std::collections::HashMap;

use super::{
    simplify_phis::{replace_all_uses, replace_operands},
    value_facts::ValueFacts,
};
use crate::{BinaryOp, IrNode, Module, UnaryOp};

pub fn peephole(module: &mut Module) {
    let facts = ValueFacts::analyze(module);
    let mut replacements = HashMap::<IrNodeId, IrNodeId>::new();

    for index in 0..module.nodes.len() {
        let id = IrNodeId(index as u32);
        let mut node = module.nodes[index].clone();
        replace_operands(&mut node, &replacements);

        match rewrite(module, &facts, &node) {
            Some(Rewrite::Replace(replacement)) => {
                let replacement = resolve(&replacements, replacement);
                if replacement != id {
                    replacements.insert(id, replacement);
                }
                module.nodes[index] = node;
            },
            Some(Rewrite::Node(replacement)) => module.nodes[index] = replacement,
            None => module.nodes[index] = node,
        }
    }

    if replacements.is_empty() {
        return;
    }

    replace_all_uses(module, &replacements);

    for node in &mut module.nodes {
        if let IrNode::Label(instructions) = node {
            instructions.retain(|instruction| !replacements.contains_key(instruction));
        }
    }

    for replaced in replacements.keys() {
        module.nodes[replaced.0 as usize] = IrNode::Noop;
    }
}

enum Rewrite {
    Replace(IrNodeId),
    Node(IrNode),
}

fn rewrite(module: &Module, facts: &ValueFacts, node: &IrNode) -> Option<Rewrite> {
    match node {
        IrNode::Unary { op, operand } => rewrite_unary(module, facts, *op, *operand),
        IrNode::Binary { op, lhs, rhs } => rewrite_binary(module, facts, *op, *lhs, *rhs, false),
        IrNode::CompoundBinary { op, lhs, rhs } => rewrite_binary(module, facts, *op, *lhs, *rhs, true),
        IrNode::ConditionalBranch {
            condition,
            true_block,
            false_block,
        } => {
            let Some(IrNode::Unary {
                op: UnaryOp::Not,
                operand,
            }) = module.node(*condition)
            else {
                return None;
            };

            Some(Rewrite::Node(IrNode::ConditionalBranch {
                condition: *operand,
                true_block: *false_block,
                false_block: *true_block,
            }))
        },
        _ => None,
    }
}

fn rewrite_unary(module: &Module, facts: &ValueFacts, op: UnaryOp, operand: IrNodeId) -> Option<Rewrite> {
    match op {
        UnaryOp::Not => match module.node(operand) {
            Some(IrNode::Unary {
                op: UnaryOp::Not,
                operand: inner,
            }) if facts.is_boolean(*inner) => Some(Rewrite::Replace(*inner)),
            Some(IrNode::Binary { op, lhs, rhs }) => invert_equality(*op).map(|op| {
                Rewrite::Node(IrNode::Binary {
                    op,
                    lhs: *lhs,
                    rhs: *rhs,
                })
            }),
            _ => None,
        },
        UnaryOp::Neg => match module.node(operand) {
            Some(IrNode::Unary {
                op: UnaryOp::Neg,
                operand: inner,
            }) if facts.is_number(*inner) => Some(Rewrite::Replace(*inner)),
            _ => None,
        },
        UnaryOp::BitNot => match module.node(operand) {
            Some(IrNode::Unary {
                op: UnaryOp::BitNot,
                operand: inner,
            }) if facts.is_u24(*inner) => Some(Rewrite::Replace(*inner)),
            _ => None,
        },
        _ => None,
    }
}

fn rewrite_binary(
    module: &Module, facts: &ValueFacts, op: BinaryOp, lhs: IrNodeId, rhs: IrNodeId, compound: bool,
) -> Option<Rewrite> {
    use BinaryOp::*;

    let identity = match op {
        Mul if is_one(module, rhs) && facts.is_number(lhs) => Some(lhs),
        Mul if is_one(module, lhs) && facts.is_number(rhs) => Some(rhs),
        Div if is_one(module, rhs) && facts.is_number(lhs) => Some(lhs),
        Sub if is_positive_zero(module, rhs) && facts.is_number(lhs) => Some(lhs),
        _ => None,
    };

    if let Some(identity) = identity {
        return Some(Rewrite::Replace(identity));
    }

    if compound {
        return None;
    }

    if matches!(op, LogicalAnd | LogicalOr) && lhs == rhs {
        return Some(Rewrite::Replace(lhs));
    }

    let (boolean, constant) = if facts.is_boolean(lhs) {
        boolean_constant(module, rhs).map(|constant| (lhs, constant))
    } else {
        None
    }
    .or_else(|| {
        facts
            .is_boolean(rhs)
            .then(|| boolean_constant(module, lhs).map(|constant| (rhs, constant)))
            .flatten()
    })?;

    let negated = match op {
        CompEq | CompEquiv => !constant,
        CompNotEq | CompNotEquiv => constant,
        _ => return None,
    };

    if negated {
        Some(Rewrite::Node(IrNode::Unary {
            op: UnaryOp::Not,
            operand: boolean,
        }))
    } else {
        Some(Rewrite::Replace(boolean))
    }
}

fn invert_equality(op: BinaryOp) -> Option<BinaryOp> {
    match op {
        BinaryOp::CompEq => Some(BinaryOp::CompNotEq),
        BinaryOp::CompNotEq => Some(BinaryOp::CompEq),
        BinaryOp::CompEquiv => Some(BinaryOp::CompNotEquiv),
        BinaryOp::CompNotEquiv => Some(BinaryOp::CompEquiv),
        _ => None,
    }
}

fn number(module: &Module, id: IrNodeId) -> Option<f32> {
    match module.node(id) {
        Some(IrNode::Constant(Value::Num(value))) => Some(*value),
        _ => None,
    }
}

fn is_one(module: &Module, id: IrNodeId) -> bool {
    number(module, id).is_some_and(|value| value.to_bits() == 1.0_f32.to_bits())
}

fn is_positive_zero(module: &Module, id: IrNodeId) -> bool {
    number(module, id).is_some_and(|value| value.to_bits() == 0.0_f32.to_bits())
}

fn boolean_constant(module: &Module, id: IrNodeId) -> Option<bool> {
    let value = number(module, id)?;
    if value == 0.0 {
        Some(false)
    } else if value == 1.0 {
        Some(true)
    } else {
        None
    }
}

fn resolve(replacements: &HashMap<IrNodeId, IrNodeId>, mut value: IrNodeId) -> IrNodeId {
    while let Some(replacement) = replacements.get(&value) {
        value = *replacement;
    }
    value
}

#[cfg(test)]
mod tests {
    macro_rules! fixture {
        ($path:literal) => {
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/", $path))
        };
    }

    use core::{
        location::Location,
        path::TreePath,
        types::{IrNodeId, ProcId, Value},
    };

    use super::peephole;
    use crate::{BinaryOp, IrModuleBuilder, IrNode, Module, Procedure, UnaryOp};

    fn procedure(body: IrNodeId, parameters: Vec<IrNodeId>) -> Procedure {
        Procedure {
            function: IrNodeId(0),
            parameters,
            previous: None,
            owner: TreePath::default(),
            name: "test".into(),
            params: Vec::new(),
            variadic: false,
            vars: Vec::new(),
            body,
            intrinsic: None,
            location: Location::default(),
        }
    }

    fn lower(source: &str) -> Module {
        let (tokens, _) = lexer::tokenize(source);
        let ast = ast::parse(&tokens).expect("fixture should parse");
        let mut builder = IrModuleBuilder::new(&ast);
        for declaration in &ast.declarations {
            let ast::Declaration::Proc { path, params, body, .. } = declaration else {
                continue;
            };
            builder.lower_proc(
                TreePath::default(),
                path.name().cloned().unwrap_or_else(|| "anonymous".into()),
                params,
                false,
                body.as_deref().unwrap_or_default(),
                Location::default(),
            );
        }
        let mut module = builder.finish();
        peephole(&mut module);
        crate::opt::eliminate_dead_code(&mut module);
        crate::verify(&module).expect("optimized module should verify");
        module
    }

    fn count(module: &Module, predicate: impl Fn(&IrNode) -> bool) -> usize {
        module.nodes.iter().filter(|node| predicate(node)).count()
    }

    #[test]
    fn removes_numeric_identities_after_numeric_producers() {
        let module = lower(fixture!(
            "programs/removes_numeric_identities_after_numeric_producers.dm"
        ));

        assert_eq!(
            count(&module, |node| matches!(
                node,
                IrNode::Binary {
                    op: BinaryOp::Mul | BinaryOp::Div,
                    ..
                } | IrNode::CompoundBinary {
                    op: BinaryOp::Mul | BinaryOp::Div,
                    ..
                }
            )),
            0
        );
        assert_eq!(
            count(&module, |node| matches!(node, IrNode::Unary { op: UnaryOp::Neg, .. })),
            1
        );
    }

    #[test]
    fn preserves_identities_for_dynamic_values_and_exact_float_cases() {
        let module = lower(fixture!(
            "programs/preserves_identities_for_dynamic_values_and_exact_float_cases.dm"
        ));

        assert_eq!(
            count(&module, |node| matches!(node, IrNode::Binary { op: BinaryOp::Mul, .. })),
            3
        );
        for op in [BinaryOp::Add, BinaryOp::Pow, BinaryOp::CompLess] {
            assert_eq!(
                count(
                    &module,
                    |node| matches!(node, IrNode::Binary { op: actual, .. } if *actual == op)
                ),
                1
            );
        }
        assert_eq!(
            count(&module, |node| matches!(node, IrNode::Unary { op: UnaryOp::Not, .. })),
            1
        );
    }

    #[test]
    fn collapses_exact_unary_chains() {
        let module = lower(fixture!("programs/collapses_exact_unary_chains.dm"));

        assert_eq!(
            count(&module, |node| matches!(node, IrNode::Unary { op: UnaryOp::Neg, .. })),
            1
        );
        assert_eq!(
            count(&module, |node| matches!(
                node,
                IrNode::Unary {
                    op: UnaryOp::BitNot,
                    ..
                }
            )),
            3
        );
    }

    #[test]
    fn simplifies_boolean_operations_and_inverts_equality() {
        let module = lower(fixture!(
            "programs/simplifies_boolean_operations_and_inverts_equality.dm"
        ));

        assert_eq!(
            count(&module, |node| matches!(
                node,
                IrNode::Binary {
                    op: BinaryOp::CompEq | BinaryOp::CompNotEquiv,
                    ..
                }
            )),
            2
        );
        assert_eq!(
            count(&module, |node| matches!(node, IrNode::Unary { op: UnaryOp::Not, .. })),
            1
        );
    }

    #[test]
    fn swaps_a_branch_conditioned_on_not() {
        let mut module = Module {
            nodes: vec![
                IrNode::Function(ProcId(0)),
                IrNode::FunctionParameter(0),
                IrNode::Label(vec![IrNodeId(3), IrNodeId(4)]),
                IrNode::Unary {
                    op: UnaryOp::Not,
                    operand: IrNodeId(1),
                },
                IrNode::ConditionalBranch {
                    condition: IrNodeId(3),
                    true_block: IrNodeId(5),
                    false_block: IrNodeId(7),
                },
                IrNode::Label(vec![IrNodeId(6)]),
                IrNode::Return(Some(IrNodeId(9))),
                IrNode::Label(vec![IrNodeId(8)]),
                IrNode::Return(Some(IrNodeId(10))),
                IrNode::Constant(Value::Num(1.0)),
                IrNode::Constant(Value::Num(2.0)),
            ],
            constants: vec![IrNodeId(9), IrNodeId(10)],
            procs: vec![procedure(IrNodeId(2), vec![IrNodeId(1)])],
            ..Module::default()
        };

        peephole(&mut module);
        crate::opt::eliminate_dead_code(&mut module);

        assert!(matches!(
            module.node(IrNodeId(4)),
            Some(IrNode::ConditionalBranch {
                condition: IrNodeId(1),
                true_block: IrNodeId(7),
                false_block: IrNodeId(5),
            })
        ));
        assert!(matches!(module.node(IrNodeId(3)), Some(IrNode::Noop)));
        crate::verify(&module).expect("rewritten branch should verify");
    }

    #[test]
    fn removes_idempotent_logical_operations() {
        for op in [BinaryOp::LogicalAnd, BinaryOp::LogicalOr] {
            let mut module = Module {
                nodes: vec![
                    IrNode::Function(ProcId(0)),
                    IrNode::FunctionParameter(0),
                    IrNode::Label(vec![IrNodeId(3), IrNodeId(4)]),
                    IrNode::Binary {
                        op,
                        lhs: IrNodeId(1),
                        rhs: IrNodeId(1),
                    },
                    IrNode::Return(Some(IrNodeId(3))),
                ],
                procs: vec![procedure(IrNodeId(2), vec![IrNodeId(1)])],
                ..Module::default()
            };

            peephole(&mut module);

            assert!(matches!(module.node(IrNodeId(3)), Some(IrNode::Noop)));
            assert!(matches!(
                module.node(IrNodeId(4)),
                Some(IrNode::Return(Some(IrNodeId(1))))
            ));
            crate::verify(&module).expect("logical identity should verify");
        }
    }

    #[test]
    fn updates_parameter_default_metadata() {
        let module = lower(fixture!("programs/updates_parameter_default_metadata.dm"));
        let default = module.procs[0].params[0].default.expect("default value");

        assert!(matches!(
            module.node(default),
            Some(IrNode::Unary { op: UnaryOp::Neg, .. })
        ));
        assert!(
            !module
                .nodes
                .iter()
                .any(|node| matches!(node, IrNode::Binary { op: BinaryOp::Mul, .. }))
        );
    }
}
