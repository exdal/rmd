// https://sunfishcode.github.io/blog/2018/10/22/Canonicalization.html

use core::types::IrNodeId;
use std::collections::{HashMap, HashSet};

use crate::{IrNode, Module};

pub fn canonicalize_terminators(module: &mut Module) {
    let blocks = module
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(index, node)| matches!(node, IrNode::Label(_)).then_some(IrNodeId(index as u32)))
        .collect::<Vec<_>>();
    let mut removed = Vec::new();

    for block in &blocks {
        let Some(instructions) = module.block(*block).map(<[IrNodeId]>::to_vec) else {
            continue;
        };
        let end = instructions
            .iter()
            .position(|instruction| module.node(*instruction).is_some_and(IrNode::is_terminator))
            .map_or(instructions.len(), |index| index + 1);

        // retained instructions after a terminator
        let mut retained = instructions[..end].to_vec();
        removed.extend_from_slice(&instructions[end..]);

        let supports_merge = retained
            .last()
            .and_then(|instruction| module.node(*instruction))
            .is_some_and(|node| matches!(node, IrNode::Branch(_) | IrNode::ConditionalBranch { .. }));
        if !supports_merge {
            let mut discarded_merges = Vec::new();
            retained.retain(|instruction| {
                let keep = !module.node(*instruction).is_some_and(IrNode::is_merge);
                if !keep {
                    discarded_merges.push(*instruction);
                }
                keep
            });
            removed.extend(discarded_merges);
        }

        if let Some(IrNode::Label(instructions)) = module.nodes.get_mut(block.0 as usize) {
            *instructions = retained;
        }
    }

    let removed = removed.into_iter().collect::<HashSet<_>>();
    for instruction in &removed {
        if let Some(node) = module.nodes.get_mut(instruction.0 as usize) {
            *node = IrNode::Noop;
        }
    }

    for proc in &mut module.procs {
        for parameter in &mut proc.params {
            clear_removed(&mut parameter.default, &removed);
            clear_removed(&mut parameter.in_list, &removed);
            for dimension in &mut parameter.spec.dimensions {
                clear_removed(dimension, &removed);
            }
        }
        for variable in &mut proc.vars {
            for dimension in &mut variable.dimensions {
                clear_removed(dimension, &removed);
            }
        }
    }

    let predecessors = predecessors(module, &blocks);
    for block in blocks {
        let expected = predecessors.get(&block).cloned().unwrap_or_default();
        let phis = module
            .block(block)
            .unwrap_or_default()
            .iter()
            .copied()
            .take_while(|instruction| matches!(module.node(*instruction), Some(IrNode::Phi { .. })))
            .collect::<Vec<_>>();

        for phi in phis {
            let mut remaining = counts(&expected);
            if let Some(IrNode::Phi { operands }) = module.nodes.get_mut(phi.0 as usize) {
                operands.retain(|operand| {
                    let Some(count) = remaining.get_mut(&operand.block) else {
                        return false;
                    };
                    if *count == 0 {
                        return false;
                    }
                    *count -= 1;

                    true
                });
            }
        }
    }
}

fn predecessors(module: &Module, blocks: &[IrNodeId]) -> HashMap<IrNodeId, Vec<IrNodeId>> {
    let mut predecessors = HashMap::<IrNodeId, Vec<IrNodeId>>::new();

    for block in blocks {
        let Some(terminator) = module
            .block(*block)
            .and_then(<[IrNodeId]>::last)
            .and_then(|id| module.node(*id))
        else {
            continue;
        };

        match terminator {
            IrNode::Branch(target) => predecessors.entry(*target).or_default().push(*block),
            IrNode::ConditionalBranch {
                true_block,
                false_block,
                ..
            } => {
                predecessors.entry(*true_block).or_default().push(*block);
                predecessors.entry(*false_block).or_default().push(*block);
            },
            _ => {},
        }
    }

    predecessors
}

fn counts(values: &[IrNodeId]) -> HashMap<IrNodeId, usize> {
    let mut counts = HashMap::new();
    for value in values {
        *counts.entry(*value).or_default() += 1;
    }

    counts
}

fn clear_removed(node: &mut Option<IrNodeId>, removed: &HashSet<IrNodeId>) {
    if node.is_some_and(|node| removed.contains(&node)) {
        *node = None;
    }
}

#[cfg(test)]
mod tests {
    use core::types::Value;

    use super::canonicalize_terminators;
    use crate::{IrNode, Module, PhiOperand};

    #[test]
    fn removes_instructions_and_phi_edges_after_throw() {
        let mut module = Module {
            nodes: vec![
                IrNode::Constant(Value::Num(1.0)),
                IrNode::Label(vec![core::types::IrNodeId(2), core::types::IrNodeId(3)]),
                IrNode::Throw(core::types::IrNodeId(0)),
                IrNode::Branch(core::types::IrNodeId(4)),
                IrNode::Label(vec![core::types::IrNodeId(5), core::types::IrNodeId(6)]),
                IrNode::Phi {
                    operands: vec![PhiOperand {
                        block: core::types::IrNodeId(1),
                        value: core::types::IrNodeId(0),
                    }],
                },
                IrNode::Return(Some(core::types::IrNodeId(5))),
            ],
            constants: vec![core::types::IrNodeId(0)],
            ..Module::default()
        };

        canonicalize_terminators(&mut module);

        assert_eq!(
            module.block(core::types::IrNodeId(1)),
            Some([core::types::IrNodeId(2)].as_slice())
        );
        assert!(matches!(module.node(core::types::IrNodeId(3)), Some(IrNode::Noop)));
        assert!(matches!(
            module.node(core::types::IrNodeId(5)),
            Some(IrNode::Phi { operands }) if operands.is_empty()
        ));
        crate::verify(&module).expect("canonicalized module should verify");
    }
}
