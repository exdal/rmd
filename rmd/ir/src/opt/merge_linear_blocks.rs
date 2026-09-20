use core::types::IrNodeId;

use crate::{IrNode, Module};

pub fn merge_linear_blocks(module: &mut Module) {
    let mut predecessor_count = vec![0_u32; module.nodes.len()];
    let mut protected = vec![false; module.nodes.len()];

    for proc in &module.procs {
        protect(&mut protected, proc.body);
    }

    let blocks = module
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(index, node)| matches!(node, IrNode::Label(_)).then_some(IrNodeId(index as u32)))
        .collect::<Vec<_>>();
    for block in &blocks {
        for instruction in module.block(*block).unwrap_or_default() {
            match module.node(*instruction) {
                Some(IrNode::Branch(target)) => add_predecessor(&mut predecessor_count, *target),
                Some(IrNode::ConditionalBranch {
                    true_block,
                    false_block,
                    ..
                }) => {
                    add_predecessor(&mut predecessor_count, *true_block);
                    add_predecessor(&mut predecessor_count, *false_block);
                },
                Some(IrNode::SelectionMerge { merge_block }) => protect(&mut protected, *merge_block),
                Some(IrNode::LoopMerge {
                    merge_block,
                    continue_block,
                }) => {
                    protect(&mut protected, *merge_block);
                    protect(&mut protected, *continue_block);
                },
                Some(IrNode::TryCatch { body, catch, merge }) => {
                    protect(&mut protected, *body);
                    protect(&mut protected, *catch);
                    protect(&mut protected, *merge);
                },
                _ => {},
            }
        }
    }

    for block in blocks {
        while merge_successor(module, block, &mut predecessor_count, &protected) {}
    }
}

fn merge_successor(module: &mut Module, block: IrNodeId, predecessor_count: &mut [u32], protected: &[bool]) -> bool {
    let Some(instructions) = module.block(block) else {
        return false;
    };
    if instructions
        .iter()
        .any(|instruction| module.node(*instruction).is_some_and(IrNode::is_merge))
    {
        return false;
    }
    let Some(branch) = instructions.last().copied() else {
        return false;
    };
    let Some(IrNode::Branch(successor)) = module.node(branch) else {
        return false;
    };
    let successor = *successor;
    if successor == block
        || protected.get(successor.0 as usize).copied().unwrap_or(true)
        || predecessor_count.get(successor.0 as usize).copied() != Some(1)
    {
        return false;
    }

    let Some(successor_instructions) = module.block(successor) else {
        return false;
    };
    if successor_instructions
        .first()
        .is_some_and(|instruction| matches!(module.node(*instruction), Some(IrNode::Phi { .. })))
    {
        return false;
    }
    let outgoing = ordinary_successors(module, successor_instructions);

    let IrNode::Label(successor_instructions) =
        std::mem::replace(&mut module.nodes[successor.0 as usize], IrNode::Noop)
    else {
        unreachable!();
    };
    let Some(IrNode::Label(instructions)) = module.nodes.get_mut(block.0 as usize) else {
        unreachable!();
    };
    let removed_branch = instructions.pop();
    assert_eq!(
        removed_branch,
        Some(branch),
        "linear block terminator changed during merge"
    );
    instructions.extend(successor_instructions);
    module.nodes[branch.0 as usize] = IrNode::Noop;
    predecessor_count[successor.0 as usize] = 0;

    for target in outgoing {
        replace_phi_predecessor(module, target, successor, block);
    }

    true
}

fn ordinary_successors(module: &Module, instructions: &[IrNodeId]) -> Vec<IrNodeId> {
    match instructions.last().and_then(|instruction| module.node(*instruction)) {
        Some(IrNode::Branch(target)) => vec![*target],
        Some(IrNode::ConditionalBranch {
            true_block,
            false_block,
            ..
        }) => vec![*true_block, *false_block],
        _ => Vec::new(),
    }
}

fn replace_phi_predecessor(module: &mut Module, target: IrNodeId, from: IrNodeId, to: IrNodeId) {
    let phis = module
        .block(target)
        .unwrap_or_default()
        .iter()
        .copied()
        .take_while(|instruction| matches!(module.node(*instruction), Some(IrNode::Phi { .. })))
        .collect::<Vec<_>>();

    for phi in phis {
        let Some(IrNode::Phi { operands }) = module.nodes.get_mut(phi.0 as usize) else {
            continue;
        };

        for operand in operands {
            if operand.block == from {
                operand.block = to;
            }
        }
    }
}

fn add_predecessor(predecessor_count: &mut [u32], block: IrNodeId) {
    if let Some(count) = predecessor_count.get_mut(block.0 as usize) {
        *count = count.saturating_add(1);
    }
}

fn protect(protected: &mut [bool], block: IrNodeId) {
    if let Some(protected) = protected.get_mut(block.0 as usize) {
        *protected = true;
    }
}

#[cfg(test)]
mod tests {
    use core::{
        location::Location,
        path::TreePath,
        types::{IrNodeId, ProcId, Value},
    };

    use super::merge_linear_blocks;
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
    fn merges_a_complete_linear_chain() {
        let mut module = Module {
            nodes: vec![
                IrNode::Function(ProcId(0)),
                IrNode::Label(vec![IrNodeId(2)]),
                IrNode::Branch(IrNodeId(3)),
                IrNode::Label(vec![IrNodeId(4)]),
                IrNode::Branch(IrNodeId(5)),
                IrNode::Label(vec![IrNodeId(6)]),
                IrNode::Return(None),
            ],
            procs: vec![procedure(IrNodeId(1))],
            ..Module::default()
        };

        merge_linear_blocks(&mut module);

        assert_eq!(module.block(IrNodeId(1)), Some([IrNodeId(6)].as_slice()));
        for removed in [IrNodeId(2), IrNodeId(3), IrNodeId(4), IrNodeId(5)] {
            assert!(matches!(module.node(removed), Some(IrNode::Noop)));
        }
        crate::verify(&module).expect("merged linear chain should verify");
    }

    #[test]
    fn repairs_phi_predecessors_after_a_merge() {
        let mut module = Module {
            nodes: vec![
                IrNode::Function(ProcId(0)),
                IrNode::Constant(Value::Num(1.0)),
                IrNode::Label(vec![IrNodeId(3)]),
                IrNode::Branch(IrNodeId(4)),
                IrNode::Label(vec![IrNodeId(5), IrNodeId(6)]),
                IrNode::SelectionMerge {
                    merge_block: IrNodeId(7),
                },
                IrNode::Branch(IrNodeId(7)),
                IrNode::Label(vec![IrNodeId(8), IrNodeId(9)]),
                IrNode::Phi {
                    operands: vec![PhiOperand {
                        block: IrNodeId(4),
                        value: IrNodeId(1),
                    }],
                },
                IrNode::Return(Some(IrNodeId(8))),
            ],
            constants: vec![IrNodeId(1)],
            procs: vec![procedure(IrNodeId(2))],
            ..Module::default()
        };

        merge_linear_blocks(&mut module);

        assert_eq!(module.block(IrNodeId(2)), Some([IrNodeId(5), IrNodeId(6)].as_slice()));
        assert!(matches!(
            module.node(IrNodeId(8)),
            Some(IrNode::Phi { operands })
                if operands.len() == 1 && operands[0].block == IrNodeId(2)
        ));
        crate::verify(&module).expect("phi should name the merged predecessor");
    }

    #[test]
    fn preserves_structural_merge_targets() {
        let mut module = Module {
            nodes: vec![
                IrNode::Function(ProcId(0)),
                IrNode::Label(vec![IrNodeId(2), IrNodeId(3)]),
                IrNode::SelectionMerge {
                    merge_block: IrNodeId(4),
                },
                IrNode::Branch(IrNodeId(4)),
                IrNode::Label(vec![IrNodeId(5)]),
                IrNode::Return(None),
            ],
            procs: vec![procedure(IrNodeId(1))],
            ..Module::default()
        };

        merge_linear_blocks(&mut module);

        assert!(matches!(module.node(IrNodeId(4)), Some(IrNode::Label(_))));
        assert_eq!(module.block(IrNodeId(1)), Some([IrNodeId(2), IrNodeId(3)].as_slice()));
        crate::verify(&module).expect("selection merge target should remain a block");
    }
}
