use core::types::IrNodeId;

use crate::{IrNode, Module};

pub fn eliminate_unreachable_blocks(module: &mut Module) {
    let reachable = reachable_blocks(module);
    let mut obsolete_merges = vec![false; module.nodes.len()];

    for (index, node) in module.nodes.iter().enumerate() {
        if !reachable[index] {
            continue;
        }
        let IrNode::Label(instructions) = node else {
            continue;
        };

        for instruction in instructions {
            let obsolete = match module.node(*instruction) {
                Some(IrNode::SelectionMerge { merge_block }) => !is_reachable(&reachable, *merge_block),
                Some(IrNode::LoopMerge {
                    merge_block,
                    continue_block,
                }) => !is_reachable(&reachable, *merge_block) || !is_reachable(&reachable, *continue_block),
                _ => false,
            };
            if obsolete {
                obsolete_merges[instruction.0 as usize] = true;
            }
        }
    }

    let surviving_phis = module
        .nodes
        .iter()
        .enumerate()
        .filter(|(index, _)| reachable[*index])
        .filter_map(|(_, node)| match node {
            IrNode::Label(instructions) => Some(
                instructions
                    .iter()
                    .copied()
                    .take_while(|instruction| matches!(module.node(*instruction), Some(IrNode::Phi { .. }))),
            ),
            _ => None,
        })
        .flatten()
        .collect::<Vec<_>>();

    for phi in surviving_phis {
        if let Some(IrNode::Phi { operands }) = module.nodes.get_mut(phi.0 as usize) {
            operands.retain(|operand| is_reachable(&reachable, operand.block));
        }
    }

    for (index, node) in module.nodes.iter_mut().enumerate() {
        if let IrNode::Label(instructions) = node
            && reachable[index]
        {
            instructions.retain(|instruction| !obsolete_merges[instruction.0 as usize]);
        }
    }

    for (index, obsolete) in obsolete_merges.into_iter().enumerate() {
        if obsolete {
            module.nodes[index] = IrNode::Noop;
        }
    }

    for (index, block_reachable) in reachable.iter().copied().enumerate() {
        if block_reachable || !matches!(module.nodes[index], IrNode::Label(_)) {
            continue;
        }

        let IrNode::Label(instructions) = std::mem::replace(&mut module.nodes[index], IrNode::Noop) else {
            unreachable!();
        };

        for instruction in instructions {
            module.nodes[instruction.0 as usize] = IrNode::Noop;
        }
    }
}

fn reachable_blocks(module: &Module) -> Vec<bool> {
    let mut reachable = vec![false; module.nodes.len()];
    let mut pending = module.procs.iter().map(|proc| proc.body).collect::<Vec<_>>();

    while let Some(block) = pending.pop() {
        let Some(node) = module.node(block) else {
            continue;
        };

        let index = block.0 as usize;

        if reachable[index] {
            continue;
        }

        let IrNode::Label(instructions) = node else {
            continue;
        };

        reachable[index] = true;

        for instruction in instructions {
            match module.node(*instruction) {
                Some(IrNode::Branch(target)) => pending.push(*target),
                Some(IrNode::ConditionalBranch {
                    true_block,
                    false_block,
                    ..
                }) => {
                    pending.push(*true_block);
                    pending.push(*false_block);
                },
                Some(IrNode::TryCatch { body, catch, .. }) => {
                    pending.push(*body);
                    pending.push(*catch);
                },
                _ => {},
            }
        }
    }

    reachable
}

fn is_reachable(reachable: &[bool], block: IrNodeId) -> bool {
    reachable.get(block.0 as usize).copied().unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use core::{
        location::Location,
        path::TreePath,
        types::{IrNodeId, ProcId, Value},
    };

    use super::eliminate_unreachable_blocks;
    use crate::{IrNode, Module, PhiOperand, Procedure};

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
    fn removes_a_dead_predecessor_and_its_phi_input() {
        let mut module = Module {
            nodes: vec![
                IrNode::Function(ProcId(0)),
                IrNode::Constant(Value::Num(10.0)),
                IrNode::Constant(Value::Num(20.0)),
                IrNode::Label(vec![IrNodeId(4)]),
                IrNode::Branch(IrNodeId(7)),
                IrNode::Label(vec![IrNodeId(6)]),
                IrNode::Branch(IrNodeId(7)),
                IrNode::Label(vec![IrNodeId(8), IrNodeId(9)]),
                IrNode::Phi {
                    operands: vec![
                        PhiOperand {
                            block: IrNodeId(3),
                            value: IrNodeId(1),
                        },
                        PhiOperand {
                            block: IrNodeId(5),
                            value: IrNodeId(2),
                        },
                    ],
                },
                IrNode::Return(Some(IrNodeId(8))),
            ],
            constants: vec![IrNodeId(1), IrNodeId(2)],
            procs: vec![procedure(IrNodeId(3))],
            ..Module::default()
        };

        eliminate_unreachable_blocks(&mut module);

        assert!(matches!(module.node(IrNodeId(5)), Some(IrNode::Noop)));
        assert!(matches!(module.node(IrNodeId(6)), Some(IrNode::Noop)));
        assert!(matches!(
            module.node(IrNodeId(8)),
            Some(IrNode::Phi { operands })
                if operands.len() == 1
                    && operands[0].block == IrNodeId(3)
                    && operands[0].value == IrNodeId(1)
        ));
        crate::verify(&module).expect("module without its unreachable predecessor should verify");
    }

    #[test]
    fn removes_a_loop_marker_when_its_continue_block_is_dead() {
        let mut module = Module {
            nodes: vec![
                IrNode::Function(ProcId(0)),
                IrNode::Label(vec![IrNodeId(2), IrNodeId(3)]),
                IrNode::LoopMerge {
                    merge_block: IrNodeId(6),
                    continue_block: IrNodeId(4),
                },
                IrNode::Branch(IrNodeId(6)),
                IrNode::Label(vec![IrNodeId(5)]),
                IrNode::Branch(IrNodeId(1)),
                IrNode::Label(vec![IrNodeId(7)]),
                IrNode::Return(None),
            ],
            procs: vec![procedure(IrNodeId(1))],
            ..Module::default()
        };

        eliminate_unreachable_blocks(&mut module);

        assert_eq!(module.block(IrNodeId(1)), Some([IrNodeId(3)].as_slice()));
        assert!(matches!(module.node(IrNodeId(2)), Some(IrNode::Noop)));
        assert!(matches!(module.node(IrNodeId(4)), Some(IrNode::Noop)));
        crate::verify(&module).expect("simplified loop should verify");
    }

    #[test]
    fn keeps_try_and_catch_regions_reachable() {
        let mut module = Module {
            nodes: vec![
                IrNode::Function(ProcId(0)),
                IrNode::Label(vec![IrNodeId(2), IrNodeId(3)]),
                IrNode::TryCatch {
                    body: IrNodeId(4),
                    catch: IrNodeId(6),
                    merge: IrNodeId(8),
                },
                IrNode::Branch(IrNodeId(8)),
                IrNode::Label(vec![IrNodeId(5)]),
                IrNode::Return(None),
                IrNode::Label(vec![IrNodeId(7)]),
                IrNode::Return(None),
                IrNode::Label(vec![IrNodeId(9)]),
                IrNode::Return(None),
            ],
            procs: vec![procedure(IrNodeId(1))],
            ..Module::default()
        };

        eliminate_unreachable_blocks(&mut module);

        for block in [IrNodeId(1), IrNodeId(4), IrNodeId(6), IrNodeId(8)] {
            assert!(matches!(module.node(block), Some(IrNode::Label(_))));
        }
        crate::verify(&module).expect("try/catch regions should survive CFG cleanup");
    }
}
