use core::types::IrNodeId;

use ast::UnaryOp;

use super::value_facts::ValueFacts;
use crate::{IrNode, SideEffect, Module};

pub fn eliminate_dead_code(module: &mut Module) {
    let facts = ValueFacts::analyze(module);
    let mut live = vec![false; module.nodes.len()];
    let mut pending = Vec::new();

    for node in &module.nodes {
        let IrNode::Label(instructions) = node else {
            continue;
        };
        for instruction in instructions {
            let Some(node) = module.node(*instruction) else {
                continue;
            };
            if !is_removable_when_unused(node, &facts) {
                mark(*instruction, &mut live, &mut pending);
            }
        }
    }

    for proc in &module.procs {
        for parameter in &proc.params {
            mark_optional(parameter.default, &mut live, &mut pending);
            mark_optional(parameter.in_list, &mut live, &mut pending);
            for dimension in &parameter.spec.dimensions {
                mark_optional(*dimension, &mut live, &mut pending);
            }
        }
        for variable in &proc.vars {
            for dimension in &variable.dimensions {
                mark_optional(*dimension, &mut live, &mut pending);
            }
        }
    }

    while let Some(instruction) = pending.pop() {
        let Some(node) = module.node(instruction) else {
            continue;
        };
        node.for_each_operand(|operand| mark(operand, &mut live, &mut pending));
    }

    let mut removed = Vec::new();
    for node in &mut module.nodes {
        if let IrNode::Label(instructions) = node {
            instructions.retain(|instruction| {
                let keep = live.get(instruction.0 as usize).copied().unwrap_or(false);
                if !keep {
                    removed.push(*instruction);
                }
                keep
            });
        }
    }
    for instruction in removed {
        module.nodes[instruction.0 as usize] = IrNode::Noop;
    }
}

fn mark(instruction: IrNodeId, live: &mut [bool], pending: &mut Vec<IrNodeId>) {
    let Some(marked) = live.get_mut(instruction.0 as usize) else {
        return;
    };
    if !*marked {
        *marked = true;
        pending.push(instruction);
    }
}

fn mark_optional(instruction: Option<IrNodeId>, live: &mut [bool], pending: &mut Vec<IrNodeId>) {
    if let Some(instruction) = instruction {
        mark(instruction, live, pending);
    }
}

fn is_removable_when_unused(node: &IrNode, facts: &ValueFacts) -> bool {
    match node {
        IrNode::Phi { .. } | IrNode::Noop => true,
        IrNode::Unary {
            op: UnaryOp::Neg | UnaryOp::BitNot,
            operand,
        } if facts.is_number(*operand) => true,
        node => node.effect() == SideEffect::Pure,
    }
}

#[cfg(test)]
mod tests {
    use core::{
        location::Location,
        path::TreePath,
        types::{IrNodeId, ProcId, Value},
    };

    use super::eliminate_dead_code;
    use crate::{BinaryOp, Builtin, IrNode, Module, PhiOperand, Procedure, UnaryOp};

    fn procedure(body: IrNodeId) -> Procedure {
        Procedure {
            function: IrNodeId(0),
            parameters: Vec::new(),
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

    #[test]
    fn removes_an_unused_phi_cycle() {
        let mut module = Module {
            nodes: vec![
                IrNode::Function(ProcId(0)),
                IrNode::Label(vec![IrNodeId(2), IrNodeId(3), IrNodeId(4)]),
                IrNode::Phi {
                    operands: vec![PhiOperand {
                        block: IrNodeId(1),
                        value: IrNodeId(3),
                    }],
                },
                IrNode::Phi {
                    operands: vec![PhiOperand {
                        block: IrNodeId(1),
                        value: IrNodeId(2),
                    }],
                },
                IrNode::Branch(IrNodeId(1)),
            ],
            procs: vec![procedure(IrNodeId(1))],
            ..Module::default()
        };

        eliminate_dead_code(&mut module);

        assert_eq!(module.block(IrNodeId(1)), Some([IrNodeId(4)].as_slice()));
        assert!(matches!(module.node(IrNodeId(2)), Some(IrNode::Noop)));
        assert!(matches!(module.node(IrNodeId(3)), Some(IrNode::Noop)));
        crate::verify(&module).expect("module without its dead phi cycle should verify");
    }

    #[test]
    fn keeps_a_live_operand_chain_and_removes_it_when_dead() {
        let mut module = Module {
            nodes: vec![
                IrNode::Function(ProcId(0)),
                IrNode::Constant(Value::Num(0.0)),
                IrNode::Label(vec![IrNodeId(3), IrNodeId(4), IrNodeId(5), IrNodeId(6)]),
                IrNode::Builtin(Builtin::Src),
                IrNode::Unary {
                    op: UnaryOp::Not,
                    operand: IrNodeId(3),
                },
                IrNode::Binary {
                    op: BinaryOp::CompEq,
                    lhs: IrNodeId(4),
                    rhs: IrNodeId(1),
                },
                IrNode::Return(Some(IrNodeId(5))),
            ],
            constants: vec![IrNodeId(1)],
            procs: vec![procedure(IrNodeId(2))],
            ..Module::default()
        };

        eliminate_dead_code(&mut module);

        assert_eq!(
            module.block(IrNodeId(2)),
            Some([IrNodeId(3), IrNodeId(4), IrNodeId(5), IrNodeId(6)].as_slice())
        );
        crate::verify(&module).expect("live operand chain should verify");

        module.nodes[6] = IrNode::Return(None);
        eliminate_dead_code(&mut module);

        assert_eq!(module.block(IrNodeId(2)), Some([IrNodeId(6)].as_slice()));
        for instruction in [IrNodeId(3), IrNodeId(4), IrNodeId(5)] {
            assert!(matches!(module.node(instruction), Some(IrNode::Noop)));
        }
        crate::verify(&module).expect("dead operand chain should be removed");
    }

    #[test]
    fn preserves_an_unused_operation_that_can_fault() {
        let mut module = Module {
            nodes: vec![
                IrNode::Function(ProcId(0)),
                IrNode::Constant(Value::Num(1.0)),
                IrNode::Constant(Value::Num(0.0)),
                IrNode::Label(vec![IrNodeId(4), IrNodeId(5)]),
                IrNode::Binary {
                    op: BinaryOp::Div,
                    lhs: IrNodeId(1),
                    rhs: IrNodeId(2),
                },
                IrNode::Return(None),
            ],
            constants: vec![IrNodeId(1), IrNodeId(2)],
            procs: vec![procedure(IrNodeId(3))],
            ..Module::default()
        };

        eliminate_dead_code(&mut module);

        assert_eq!(module.block(IrNodeId(3)), Some([IrNodeId(4), IrNodeId(5)].as_slice()));
        crate::verify(&module).expect("potentially faulting operation should remain valid");
    }

    #[test]
    fn preserves_a_call_even_when_its_result_is_unused() {
        let mut module = Module {
            nodes: vec![
                IrNode::Function(ProcId(0)),
                IrNode::Label(vec![IrNodeId(2), IrNodeId(3)]),
                IrNode::FunctionCall {
                    function: IrNodeId(0),
                    args: Vec::new(),
                },
                IrNode::Return(None),
            ],
            procs: vec![procedure(IrNodeId(1))],
            ..Module::default()
        };

        eliminate_dead_code(&mut module);

        assert_eq!(module.block(IrNodeId(1)), Some([IrNodeId(2), IrNodeId(3)].as_slice()));
        crate::verify(&module).expect("side-effecting call should remain valid");
    }
}
