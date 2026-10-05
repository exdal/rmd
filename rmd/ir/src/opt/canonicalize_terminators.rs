// https://sunfishcode.github.io/blog/2018/10/22/Canonicalization.html

use core::types::IrNodeId;

use rustc_hash::FxHashSet;

use super::Adjacency;
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
        let Some(IrNode::Label(instructions)) = module.nodes.get_mut(block.0 as usize) else {
            continue;
        };
        let mut retained = std::mem::take(instructions);
        let end = retained
            .iter()
            .position(|instruction| module.node(*instruction).is_some_and(IrNode::is_terminator))
            .map_or(retained.len(), |index| index + 1);

        // retained instructions after a terminator
        removed.extend_from_slice(&retained[end..]);
        retained.truncate(end);

        let supports_merge = retained
            .last()
            .and_then(|instruction| module.node(*instruction))
            .is_some_and(|node| matches!(node, IrNode::Branch(_) | IrNode::ConditionalBranch { .. }));
        if !supports_merge {
            retained.retain(|instruction| {
                let keep = !module.node(*instruction).is_some_and(IrNode::is_merge);
                if !keep {
                    removed.push(*instruction);
                }
                keep
            });
        }

        if let Some(IrNode::Label(instructions)) = module.nodes.get_mut(block.0 as usize) {
            *instructions = retained;
        }
    }

    let removed = removed.into_iter().collect::<FxHashSet<_>>();
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

    let predecessors = Adjacency::new(module.nodes.len(), |edge| {
        for block in &blocks {
            for target in successors(module, *block).into_iter().flatten() {
                edge(target.0, *block);
            }
        }
    });
    let mut expected = Vec::new();
    let mut remaining = Vec::new();
    for block in blocks {
        let Some(instructions) = module.block(block) else {
            continue;
        };
        let phi_count = instructions
            .iter()
            .take_while(|instruction| matches!(module.node(**instruction), Some(IrNode::Phi { .. })))
            .count();
        if phi_count == 0 {
            continue;
        }

        counts(predecessors.of(block.0), &mut expected);
        for index in 0..phi_count {
            let Some(IrNode::Label(instructions)) = module.nodes.get(block.0 as usize) else {
                break;
            };
            let phi = instructions[index];
            remaining.clone_from(&expected);
            if let Some(IrNode::Phi { operands }) = module.nodes.get_mut(phi.0 as usize) {
                operands.retain(|operand| {
                    let Ok(slot) = remaining.binary_search_by_key(&operand.block.0, |(block, _)| block.0) else {
                        return false;
                    };
                    let count = &mut remaining[slot].1;
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

fn successors(module: &Module, block: IrNodeId) -> [Option<IrNodeId>; 2] {
    let terminator = module
        .block(block)
        .and_then(<[IrNodeId]>::last)
        .and_then(|id| module.node(*id));
    match terminator {
        Some(IrNode::Branch(target)) => [Some(*target), None],
        Some(IrNode::ConditionalBranch {
            true_block,
            false_block,
            ..
        }) => [Some(*true_block), Some(*false_block)],
        _ => [None, None],
    }
}

/// `values` as sorted `(value, occurrences)` pairs
fn counts(values: &[IrNodeId], counts: &mut Vec<(IrNodeId, usize)>) {
    counts.clear();
    counts.extend(values.iter().map(|value| (*value, 1)));
    counts.sort_unstable_by_key(|(value, _)| value.0);
    counts.dedup_by(|(value, count), (kept, kept_count)| {
        let is_same = value == kept;
        if is_same {
            *kept_count += *count;
        }

        is_same
    });
}

fn clear_removed(node: &mut Option<IrNodeId>, removed: &FxHashSet<IrNodeId>) {
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
                IrNode::Constant(Box::new(Value::Num(1.0))),
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
