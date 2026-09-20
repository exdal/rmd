use core::types::IrNodeId;
use std::collections::VecDeque;

use crate::{IrNode, Module, PhiOperand};

#[derive(Clone, Copy)]
struct Candidate {
    block: IrNodeId,
    target: IrNodeId,
}

pub fn eliminate_forwarding_blocks(module: &mut Module) {
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

    // first we protect the intentional forwards, they literallym eant to do that
    for block in &blocks {
        for instruction in module.block(*block).unwrap_or_default() {
            match module.node(*instruction) {
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

    let mut candidate_slot = vec![u32::MAX; module.nodes.len()];
    let mut candidates = Vec::new();
    for block in &blocks {
        // skip protected ones
        if protected[block.0 as usize] {
            continue;
        }

        let Some([branch]) = module.block(*block) else {
            continue;
        };
        let Some(IrNode::Branch(target)) = module.node(*branch) else {
            continue;
        };
        if *target == *block {
            continue;
        }

        candidate_slot[block.0 as usize] = candidates.len() as u32;
        candidates.push(Candidate {
            block: *block,
            target: *target,
        });
    }

    let mut incoming = vec![Vec::new(); candidates.len()];
    for predecessor in &blocks {
        let Some(terminator) = module.block(*predecessor).and_then(<[IrNodeId]>::last) else {
            continue;
        };

        match module.node(*terminator) {
            Some(IrNode::Branch(target)) => add_incoming(&candidate_slot, &mut incoming, *target, *predecessor),
            Some(IrNode::ConditionalBranch {
                true_block,
                false_block,
                ..
            }) => {
                add_incoming(&candidate_slot, &mut incoming, *true_block, *predecessor);
                add_incoming(&candidate_slot, &mut incoming, *false_block, *predecessor);
            },
            _ => {},
        }
    }

    let order = successor_first_order(&candidates, &candidate_slot);
    for candidate in order {
        if incoming[candidate].is_empty() {
            continue;
        }
        eliminate_candidate(module, candidates[candidate].block, &incoming[candidate]);
    }
}

fn successor_first_order(candidates: &[Candidate], candidate_slot: &[u32]) -> Vec<usize> {
    let mut incoming_candidates = vec![0_u32; candidates.len()];
    for candidate in candidates {
        let slot = candidate_slot
            .get(candidate.target.0 as usize)
            .copied()
            .unwrap_or(u32::MAX);
        if slot != u32::MAX {
            incoming_candidates[slot as usize] += 1;
        }
    }

    let mut pending = incoming_candidates
        .iter()
        .enumerate()
        .filter_map(|(index, count)| (*count == 0).then_some(index))
        .collect::<VecDeque<_>>();
    let mut order = Vec::with_capacity(candidates.len());
    while let Some(candidate) = pending.pop_front() {
        order.push(candidate);
        let target = candidates[candidate].target;
        let slot = candidate_slot.get(target.0 as usize).copied().unwrap_or(u32::MAX);
        if slot == u32::MAX {
            continue;
        }
        let count = &mut incoming_candidates[slot as usize];
        *count -= 1;
        if *count == 0 {
            pending.push_back(slot as usize);
        }
    }
    order.reverse();

    order
}

fn eliminate_candidate(module: &mut Module, block: IrNodeId, predecessors: &[IrNodeId]) {
    let Some([branch]) = module.block(block) else {
        return;
    };
    let branch = *branch;
    let Some(IrNode::Branch(target)) = module.node(branch) else {
        return;
    };
    let target = *target;
    if predecessors.iter().any(|predecessor| {
        outgoing_edges_to(module, *predecessor, block) + outgoing_edges_to(module, *predecessor, target) > 1
    }) {
        return;
    }

    expand_phi_predecessor(module, target, block, predecessors);
    for predecessor in predecessors {
        assert!(
            redirect_one_edge(module, *predecessor, block, target),
            "forwarding-block predecessor no longer targets the block"
        );
    }

    let IrNode::Label(instructions) = std::mem::replace(&mut module.nodes[block.0 as usize], IrNode::Noop) else {
        unreachable!();
    };
    assert_eq!(
        instructions.as_slice(),
        [branch],
        "forwarding block changed during elimination"
    );
    module.nodes[branch.0 as usize] = IrNode::Noop;
}

fn outgoing_edges_to(module: &Module, block: IrNodeId, target: IrNodeId) -> u8 {
    let Some(terminator) = module.block(block).and_then(<[IrNodeId]>::last) else {
        return 0;
    };
    match module.node(*terminator) {
        Some(IrNode::Branch(successor)) => u8::from(*successor == target),
        Some(IrNode::ConditionalBranch {
            true_block,
            false_block,
            ..
        }) => u8::from(*true_block == target) + u8::from(*false_block == target),
        _ => 0,
    }
}

fn redirect_one_edge(module: &mut Module, predecessor: IrNodeId, from: IrNodeId, to: IrNodeId) -> bool {
    let Some(terminator) = module.block(predecessor).and_then(<[IrNodeId]>::last).copied() else {
        return false;
    };
    match module.nodes.get_mut(terminator.0 as usize) {
        Some(IrNode::Branch(target)) if *target == from => {
            *target = to;
            true
        },
        Some(IrNode::ConditionalBranch {
            true_block,
            false_block,
            ..
        }) if *true_block == from => {
            *true_block = to;
            true
        },
        Some(IrNode::ConditionalBranch { false_block, .. }) if *false_block == from => {
            *false_block = to;
            true
        },
        _ => false,
    }
}

fn expand_phi_predecessor(module: &mut Module, target: IrNodeId, from: IrNodeId, predecessors: &[IrNodeId]) {
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
        let Some(position) = operands.iter().position(|operand| operand.block == from) else {
            continue;
        };
        let value = operands.remove(position).value;
        operands.splice(
            position..position,
            predecessors.iter().copied().map(|block| PhiOperand { block, value }),
        );
    }
}

fn add_incoming(candidate_slot: &[u32], incoming: &mut [Vec<IrNodeId>], target: IrNodeId, predecessor: IrNodeId) {
    let slot = candidate_slot.get(target.0 as usize).copied().unwrap_or(u32::MAX);
    if slot != u32::MAX {
        incoming[slot as usize].push(predecessor);
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

    use super::eliminate_forwarding_blocks;
    use crate::{Builtin, IrNode, Module, PhiOperand, Procedure};

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
    fn redirects_multiple_predecessors_and_expands_phis() {
        let mut module = Module {
            nodes: vec![
                IrNode::Function(ProcId(0)),
                IrNode::Constant(Value::Num(1.0)),
                IrNode::Label(vec![IrNodeId(3)]),
                IrNode::ConditionalBranch {
                    condition: IrNodeId(1),
                    true_block: IrNodeId(4),
                    false_block: IrNodeId(7),
                },
                IrNode::Label(vec![IrNodeId(5), IrNodeId(6)]),
                IrNode::Builtin(Builtin::Src),
                IrNode::Branch(IrNodeId(10)),
                IrNode::Label(vec![IrNodeId(8), IrNodeId(9)]),
                IrNode::Builtin(Builtin::Usr),
                IrNode::Branch(IrNodeId(10)),
                IrNode::Label(vec![IrNodeId(11)]),
                IrNode::Branch(IrNodeId(12)),
                IrNode::Label(vec![IrNodeId(13), IrNodeId(14)]),
                IrNode::Phi {
                    operands: vec![PhiOperand {
                        block: IrNodeId(10),
                        value: IrNodeId(1),
                    }],
                },
                IrNode::Return(Some(IrNodeId(13))),
            ],
            constants: vec![IrNodeId(1)],
            procs: vec![procedure(IrNodeId(2))],
            ..Module::default()
        };

        eliminate_forwarding_blocks(&mut module);

        assert!(matches!(module.node(IrNodeId(10)), Some(IrNode::Noop)));
        assert!(matches!(module.node(IrNodeId(11)), Some(IrNode::Noop)));
        assert!(matches!(module.node(IrNodeId(6)), Some(IrNode::Branch(IrNodeId(12)))));
        assert!(matches!(module.node(IrNodeId(9)), Some(IrNode::Branch(IrNodeId(12)))));
        assert!(matches!(
            module.node(IrNodeId(13)),
            Some(IrNode::Phi { operands })
                if operands.len() == 2
                    && operands.iter().any(|operand| operand.block == IrNodeId(4))
                    && operands.iter().any(|operand| operand.block == IrNodeId(7))
        ));
        crate::verify(&module).expect("threaded predecessors and expanded phi should verify");
    }

    #[test]
    fn eliminates_forwarding_chains_from_the_end() {
        let mut module = Module {
            nodes: vec![
                IrNode::Function(ProcId(0)),
                IrNode::Label(vec![IrNodeId(2)]),
                IrNode::Branch(IrNodeId(3)),
                IrNode::Label(vec![IrNodeId(4)]),
                IrNode::Branch(IrNodeId(5)),
                IrNode::Label(vec![IrNodeId(6)]),
                IrNode::Branch(IrNodeId(7)),
                IrNode::Label(vec![IrNodeId(8)]),
                IrNode::Return(None),
            ],
            procs: vec![procedure(IrNodeId(1))],
            ..Module::default()
        };

        eliminate_forwarding_blocks(&mut module);

        assert!(matches!(module.node(IrNodeId(2)), Some(IrNode::Branch(IrNodeId(7)))));
        for removed in [IrNodeId(3), IrNodeId(4), IrNodeId(5), IrNodeId(6)] {
            assert!(matches!(module.node(removed), Some(IrNode::Noop)));
        }
        crate::verify(&module).expect("threaded forwarding chain should verify");
    }

    #[test]
    fn preserves_edge_identity_when_a_predecessor_already_targets_the_successor() {
        let mut module = Module {
            nodes: vec![
                IrNode::Function(ProcId(0)),
                IrNode::Constant(Value::Num(1.0)),
                IrNode::Constant(Value::Num(2.0)),
                IrNode::Constant(Value::Num(3.0)),
                IrNode::Label(vec![IrNodeId(5)]),
                IrNode::ConditionalBranch {
                    condition: IrNodeId(1),
                    true_block: IrNodeId(6),
                    false_block: IrNodeId(8),
                },
                IrNode::Label(vec![IrNodeId(7)]),
                IrNode::Branch(IrNodeId(8)),
                IrNode::Label(vec![IrNodeId(9), IrNodeId(10)]),
                IrNode::Phi {
                    operands: vec![
                        PhiOperand {
                            block: IrNodeId(4),
                            value: IrNodeId(2),
                        },
                        PhiOperand {
                            block: IrNodeId(6),
                            value: IrNodeId(3),
                        },
                    ],
                },
                IrNode::Return(Some(IrNodeId(9))),
            ],
            constants: vec![IrNodeId(1), IrNodeId(2), IrNodeId(3)],
            procs: vec![procedure(IrNodeId(4))],
            ..Module::default()
        };

        eliminate_forwarding_blocks(&mut module);

        assert!(matches!(module.node(IrNodeId(6)), Some(IrNode::Label(_))));
        assert!(matches!(module.node(IrNodeId(7)), Some(IrNode::Branch(IrNodeId(8)))));
        crate::verify(&module).expect("distinct conditional edges should keep distinct phi values");
    }

    #[test]
    fn preserves_structural_targets() {
        let mut module = Module {
            nodes: vec![
                IrNode::Function(ProcId(0)),
                IrNode::Label(vec![IrNodeId(2), IrNodeId(3)]),
                IrNode::SelectionMerge {
                    merge_block: IrNodeId(4),
                },
                IrNode::Branch(IrNodeId(4)),
                IrNode::Label(vec![IrNodeId(5)]),
                IrNode::Branch(IrNodeId(6)),
                IrNode::Label(vec![IrNodeId(7)]),
                IrNode::Return(None),
            ],
            procs: vec![procedure(IrNodeId(1))],
            ..Module::default()
        };

        eliminate_forwarding_blocks(&mut module);

        assert!(matches!(module.node(IrNodeId(4)), Some(IrNode::Label(_))));
        crate::verify(&module).expect("selection merge target should remain a block");
    }
}
