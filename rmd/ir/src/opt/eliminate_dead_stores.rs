use core::{types::IrNodeId, vars};

use fixedbitset::FixedBitSet;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::{IrNode, Module, Procedure};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Storage {
    Frame,
    External,
    Observable,
}

pub fn eliminate_dead_stores(module: &mut Module) {
    let mut removed = vec![false; module.nodes.len()];
    let mut is_any_removed = false;
    let mut scratch = Scratch::default();

    for proc in &module.procs {
        is_any_removed |= analyze_procedure(module, proc, &mut scratch, &mut removed);
    }

    if !is_any_removed {
        return;
    }

    for node in &mut module.nodes {
        if let IrNode::Label(instructions) = node {
            instructions.retain(|instruction| !removed[instruction.0 as usize]);
        }
    }

    for (index, is_removed) in removed.into_iter().enumerate() {
        if is_removed {
            module.nodes[index] = IrNode::Noop;
        }
    }
}

/// what an instruction does to the set of live pointers, with pointers already turned into slots
#[derive(Clone, Copy)]
enum Op {
    Other,
    Load(u32),
    /// dead when its slot is not live afterwards
    Overwrite(u32),
    ObserveExternal,
    /// `Initialize` of external storage
    ObserveExternalThenLoad(u32),
}

/// per proc buffers, these cleared but never deallocated. kind of using themn like bump allocator
#[derive(Default)]
struct Scratch {
    block_index: FxHashMap<IrNodeId, usize>,
    slots: FxHashMap<IrNodeId, u32>,
    ops: Vec<(Op, IrNodeId)>,
    op_ranges: Vec<std::ops::Range<usize>>,
    successors: Vec<Vec<usize>>,
    catches: Vec<Vec<usize>>,
    live_in: Vec<FixedBitSet>,
    live_out: Vec<FixedBitSet>,
    external: FixedBitSet,
    output: FixedBitSet,
    exceptional: FixedBitSet,
}

fn analyze_procedure(module: &Module, proc: &Procedure, scratch: &mut Scratch, removed: &mut [bool]) -> bool {
    let blocks = reachable_blocks(module, proc.body);

    scratch.block_index.clear();
    scratch
        .block_index
        .extend(blocks.iter().enumerate().map(|(index, block)| (*block, index)));

    scratch.slots.clear();
    scratch.ops.clear();
    scratch.op_ranges.clear();
    let mut external_slots = Vec::new();
    for block in &blocks {
        let start = scratch.ops.len();
        for instruction in module.block(*block).unwrap_or_default() {
            let op = classify(module, *instruction, &mut scratch.slots, &mut external_slots);
            scratch.ops.push((op, *instruction));
        }

        scratch.op_ranges.push(start..scratch.ops.len());
    }

    let empty = FixedBitSet::with_capacity(scratch.slots.len());
    scratch.external.clone_from(&empty);
    for slot in external_slots {
        scratch.external.insert(slot as usize);
    }

    resize_lists(&mut scratch.successors, blocks.len());
    for (index, block) in blocks.iter().enumerate() {
        let successors = &mut scratch.successors[index];
        for instruction in module.block(*block).unwrap_or_default() {
            append_successors(module.node(*instruction), &mut |successor| {
                if let Some(successor) = scratch.block_index.get(&successor) {
                    successors.push(*successor);
                }
            });
        }

        successors.sort_unstable();
        successors.dedup();
    }

    resize_lists(&mut scratch.catches, blocks.len());
    for block in &blocks {
        for instruction in module.block(*block).unwrap_or_default() {
            let Some(IrNode::TryCatch { body, catch, merge }) = module.node(*instruction) else {
                continue;
            };

            let Some(catch) = scratch.block_index.get(catch).copied() else {
                continue;
            };

            for protected in region_blocks(module, *body, *merge) {
                if let Some(protected) = scratch.block_index.get(&protected) {
                    scratch.catches[*protected].push(catch);
                }
            }
        }
    }

    reset_sets(&mut scratch.live_in, blocks.len(), &empty);
    reset_sets(&mut scratch.live_out, blocks.len(), &empty);

    let Scratch {
        ops,
        op_ranges,
        successors,
        catches,
        live_in,
        live_out,
        external,
        output,
        exceptional,
        ..
    } = scratch;

    loop {
        let mut is_changed = false;
        for block in (0..blocks.len()).rev() {
            output.clone_from(&empty);
            for successor in &successors[block] {
                output.union_with(&live_in[*successor]);
            }

            if successors[block].is_empty() {
                output.union_with(external);
            }

            gather_exceptional(exceptional, &catches[block], live_in, &empty);

            if live_out[block] != *output {
                live_out[block].clone_from(output);
                is_changed = true;
            }

            transfer(&ops[op_ranges[block].clone()], output, exceptional, external, None);

            if live_in[block] != *output {
                live_in[block].clone_from(output);
                is_changed = true;
            }
        }

        if !is_changed {
            break;
        }
    }

    let mut is_any_removed = false;
    for block in 0..blocks.len() {
        gather_exceptional(exceptional, &catches[block], live_in, &empty);
        output.clone_from(&live_out[block]);
        transfer(
            &ops[op_ranges[block].clone()],
            output,
            exceptional,
            external,
            Some((removed, &mut is_any_removed)),
        );
    }

    is_any_removed
}

fn classify(
    module: &Module, instruction: IrNodeId, slots: &mut FxHashMap<IrNodeId, u32>, external_slots: &mut Vec<u32>,
) -> Op {
    let Some(node) = module.node(instruction) else {
        return Op::Other;
    };

    let mut slot = |pointer: IrNodeId, storage: Storage| {
        let next = slots.len() as u32;
        let slot = *slots.entry(pointer).or_insert(next);
        if storage == Storage::External {
            external_slots.push(slot);
        }

        slot
    };

    match node {
        IrNode::Load { pointer } => match storage(module, *pointer) {
            Some(storage @ (Storage::Frame | Storage::External)) => Op::Load(slot(*pointer, storage)),
            _ => Op::Other,
        },
        IrNode::Store { pointer, .. } => match storage(module, *pointer) {
            Some(storage @ (Storage::Frame | Storage::External)) => Op::Overwrite(slot(*pointer, storage)),
            Some(Storage::Observable) | None => Op::ObserveExternal,
        },
        IrNode::Initialize { pointer, .. } => match storage(module, *pointer) {
            Some(Storage::Frame) => Op::Overwrite(slot(*pointer, Storage::Frame)),
            Some(Storage::External) => Op::ObserveExternalThenLoad(slot(*pointer, Storage::External)),
            Some(Storage::Observable) | None => Op::ObserveExternal,
        },
        _ if observes_external_state(node) => Op::ObserveExternal,
        _ => Op::Other,
    }
}

fn transfer(
    ops: &[(Op, IrNodeId)], live: &mut FixedBitSet, exceptional: &FixedBitSet, external: &FixedBitSet,
    mut removed: Option<(&mut [bool], &mut bool)>,
) {
    let is_protected = !exceptional.is_clear();

    for (op, instruction) in ops.iter().rev() {
        if is_protected {
            live.union_with(exceptional);
        }

        match *op {
            Op::Other => {},
            Op::Load(slot) => live.insert(slot as usize),
            Op::Overwrite(slot) => {
                if !live.contains(slot as usize)
                    && let Some((removed, is_any_removed)) = removed.as_mut()
                {
                    removed[instruction.0 as usize] = true;
                    **is_any_removed = true;
                }

                live.remove(slot as usize);
            },
            Op::ObserveExternal => live.union_with(external),
            Op::ObserveExternalThenLoad(slot) => {
                live.union_with(external);
                live.insert(slot as usize);
            },
        }
    }
}

fn gather_exceptional(exceptional: &mut FixedBitSet, catches: &[usize], live_in: &[FixedBitSet], empty: &FixedBitSet) {
    exceptional.clone_from(empty);
    for catch in catches {
        exceptional.union_with(&live_in[*catch]);
    }
}

fn reset_sets(sets: &mut Vec<FixedBitSet>, len: usize, empty: &FixedBitSet) {
    if sets.len() < len {
        sets.resize_with(len, FixedBitSet::new);
    }

    for set in &mut sets[..len] {
        set.clone_from(empty);
    }
}

fn resize_lists(lists: &mut Vec<Vec<usize>>, len: usize) {
    for list in lists.iter_mut() {
        list.clear();
    }

    lists.resize_with(len.max(lists.len()), Vec::new);
}

fn storage(module: &Module, pointer: IrNodeId) -> Option<Storage> {
    let IrNode::Variable(name) = module.node(pointer)? else {
        return None;
    };
    let name = name.as_str();
    if name.starts_with("__dmed_frame_local_") {
        Some(Storage::Frame)
    } else if is_observable_name(name) {
        Some(Storage::Observable)
    } else {
        Some(Storage::External)
    }
}

fn is_observable_name(name: &str) -> bool {
    matches!(
        name,
        vars::LEN
            | vars::LOC
            | vars::CONTENTS
            | vars::X
            | vars::Y
            | vars::Z
            | vars::TYPE
            | vars::PARENT_TYPE
            | vars::APPEARANCE
            | vars::OVERLAYS
            | vars::UNDERLAYS
    )
}

fn observes_external_state(node: &IrNode) -> bool {
    matches!(
        node,
        IrNode::AccessField { .. }
            | IrNode::Initial { .. }
            | IrNode::Index { .. }
            | IrNode::Call { .. }
            | IrNode::FunctionCall { .. }
            | IrNode::Super { .. }
            | IrNode::New { .. }
            | IrNode::IterInit { .. }
            | IrNode::IterNext(_)
            | IrNode::IterValue(_)
            | IrNode::IterKey(_)
            | IrNode::SetField { .. }
            | IrNode::SetIndex { .. }
            | IrNode::StoreBuiltin { .. }
            | IrNode::CatchValue
            | IrNode::Del(_)
            | IrNode::Throw(_)
            | IrNode::Output { .. }
            | IrNode::TryCatch { .. }
            | IrNode::Blocked(_)
            | IrNode::Trap { .. }
    )
}

fn reachable_blocks(module: &Module, entry: IrNodeId) -> Vec<IrNodeId> {
    let mut blocks = Vec::new();
    let mut seen = FxHashSet::default();
    let mut pending = vec![entry];

    while let Some(block) = pending.pop() {
        if !seen.insert(block) {
            continue;
        }
        let Some(instructions) = module.block(block) else {
            continue;
        };
        blocks.push(block);
        for instruction in instructions {
            append_successors(module.node(*instruction), &mut |block| pending.push(block));
        }
    }

    blocks
}

fn append_successors(node: Option<&IrNode>, push: &mut impl FnMut(IrNodeId)) {
    match node {
        Some(IrNode::Branch(target)) => push(*target),
        Some(IrNode::ConditionalBranch {
            true_block,
            false_block,
            ..
        }) => {
            push(*true_block);
            push(*false_block);
        },
        Some(IrNode::TryCatch { body, catch, merge }) => {
            push(*body);
            push(*catch);
            push(*merge);
        },
        _ => {},
    }
}

fn region_blocks(module: &Module, entry: IrNodeId, stop: IrNodeId) -> Vec<IrNodeId> {
    let mut blocks = Vec::new();
    let mut seen = FxHashSet::default();
    let mut pending = vec![entry];

    while let Some(block) = pending.pop() {
        if block == stop || !seen.insert(block) {
            continue;
        }
        let Some(instructions) = module.block(block) else {
            continue;
        };
        blocks.push(block);
        for instruction in instructions {
            append_successors(module.node(*instruction), &mut |block| pending.push(block));
        }
    }

    blocks
}

#[cfg(test)]
mod tests {
    use core::{
        location::Location,
        path::TreePath,
        types::{IrNodeId, ProcId, Value},
    };

    use super::eliminate_dead_stores;
    use crate::{IrNode, Module, Procedure};

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
    fn removes_an_overwritten_external_store() {
        let mut module = Module {
            nodes: vec![
                IrNode::Function(ProcId(0)),
                IrNode::Variable("value".into()),
                IrNode::Constant(Box::new(Value::Num(1.0))),
                IrNode::Constant(Box::new(Value::Num(2.0))),
                IrNode::Label(vec![IrNodeId(1), IrNodeId(5), IrNodeId(6), IrNodeId(7), IrNodeId(8)]),
                IrNode::Store {
                    pointer: IrNodeId(1),
                    value: IrNodeId(2),
                },
                IrNode::Store {
                    pointer: IrNodeId(1),
                    value: IrNodeId(3),
                },
                IrNode::Load { pointer: IrNodeId(1) },
                IrNode::Return(Some(IrNodeId(7))),
            ],
            constants: vec![IrNodeId(2), IrNodeId(3)],
            procs: vec![procedure(IrNodeId(4))],
            ..Module::default()
        };

        eliminate_dead_stores(&mut module);

        assert_eq!(
            module.block(IrNodeId(4)),
            Some([IrNodeId(1), IrNodeId(6), IrNodeId(7), IrNodeId(8)].as_slice())
        );
        assert!(matches!(module.node(IrNodeId(5)), Some(IrNode::Noop)));
    }

    #[test]
    fn preserves_a_store_observable_by_a_call() {
        let mut module = Module {
            nodes: vec![
                IrNode::Function(ProcId(0)),
                IrNode::Variable("value".into()),
                IrNode::Constant(Box::new(Value::Num(1.0))),
                IrNode::Constant(Box::new(Value::Num(2.0))),
                IrNode::Label(vec![IrNodeId(1), IrNodeId(5), IrNodeId(6), IrNodeId(7), IrNodeId(8)]),
                IrNode::Store {
                    pointer: IrNodeId(1),
                    value: IrNodeId(2),
                },
                IrNode::FunctionCall {
                    function: IrNodeId(0),
                    args: Vec::new(),
                },
                IrNode::Store {
                    pointer: IrNodeId(1),
                    value: IrNodeId(3),
                },
                IrNode::Return(None),
            ],
            constants: vec![IrNodeId(2), IrNodeId(3)],
            procs: vec![procedure(IrNodeId(4))],
            ..Module::default()
        };

        eliminate_dead_stores(&mut module);

        assert!(matches!(module.node(IrNodeId(5)), Some(IrNode::Store { .. })));
        assert!(matches!(module.node(IrNodeId(7)), Some(IrNode::Store { .. })));
    }

    #[test]
    fn removes_an_unread_frame_initialization() {
        let mut module = Module {
            nodes: vec![
                IrNode::Function(ProcId(0)),
                IrNode::Variable("__dmed_frame_local_0_0".into()),
                IrNode::Constant(Box::new(Value::Num(1.0))),
                IrNode::Label(vec![IrNodeId(1), IrNodeId(4), IrNodeId(5)]),
                IrNode::Initialize {
                    pointer: IrNodeId(1),
                    value: IrNodeId(2),
                },
                IrNode::Return(None),
            ],
            constants: vec![IrNodeId(2)],
            procs: vec![procedure(IrNodeId(3))],
            ..Module::default()
        };

        eliminate_dead_stores(&mut module);

        assert_eq!(module.block(IrNodeId(3)), Some([IrNodeId(1), IrNodeId(5)].as_slice()));
        assert!(matches!(module.node(IrNodeId(4)), Some(IrNode::Noop)));
    }

    #[test]
    fn preserves_external_initialization_and_special_writes() {
        let mut module = Module {
            nodes: vec![
                IrNode::Function(ProcId(0)),
                IrNode::Variable("value".into()),
                IrNode::Variable("loc".into()),
                IrNode::Constant(Box::new(Value::Num(1.0))),
                IrNode::Label(vec![IrNodeId(1), IrNodeId(2), IrNodeId(5), IrNodeId(6), IrNodeId(7)]),
                IrNode::Initialize {
                    pointer: IrNodeId(1),
                    value: IrNodeId(3),
                },
                IrNode::Store {
                    pointer: IrNodeId(2),
                    value: IrNodeId(3),
                },
                IrNode::Return(None),
            ],
            constants: vec![IrNodeId(3)],
            procs: vec![procedure(IrNodeId(4))],
            ..Module::default()
        };

        eliminate_dead_stores(&mut module);

        assert!(matches!(module.node(IrNodeId(5)), Some(IrNode::Initialize { .. })));
        assert!(matches!(module.node(IrNodeId(6)), Some(IrNode::Store { .. })));
    }
}
