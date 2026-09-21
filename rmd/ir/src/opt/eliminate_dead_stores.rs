use core::types::IrNodeId;
use std::collections::{HashMap, HashSet};

use crate::{IrNode, Module, Procedure};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Storage {
    Frame,
    External,
    Observable,
}

pub fn eliminate_dead_stores(module: &mut Module) {
    let mut removed = HashSet::new();

    for proc in &module.procs {
        analyze_procedure(module, proc, &mut removed);
    }

    if removed.is_empty() {
        return;
    }

    for node in &mut module.nodes {
        if let IrNode::Label(instructions) = node {
            instructions.retain(|instruction| !removed.contains(instruction));
        }
    }
    for instruction in removed {
        module.nodes[instruction.0 as usize] = IrNode::Noop;
    }
}

fn analyze_procedure(module: &Module, proc: &Procedure, removed: &mut HashSet<IrNodeId>) {
    let blocks = reachable_blocks(module, proc.body);
    let block_set = blocks.iter().copied().collect::<HashSet<_>>();
    let successors = blocks
        .iter()
        .copied()
        .map(|block| (block, block_successors(module, block, &block_set)))
        .collect::<HashMap<_, _>>();

    let mut external = HashSet::new();
    for block in &blocks {
        for instruction in module.block(*block).unwrap_or_default() {
            let Some(node) = module.node(*instruction) else {
                continue;
            };
            match node {
                IrNode::Load { pointer } | IrNode::Store { pointer, .. } | IrNode::Initialize { pointer, .. }
                    if storage(module, *pointer) == Some(Storage::External) =>
                {
                    external.insert(*pointer);
                },
                _ => {},
            }
        }
    }

    let protected = protected_catches(module, &blocks);
    let mut live_in = blocks
        .iter()
        .copied()
        .map(|block| (block, HashSet::new()))
        .collect::<HashMap<_, _>>();
    let mut live_out = live_in.clone();

    loop {
        let mut changed = false;
        for block in blocks.iter().rev().copied() {
            let block_successors = successors.get(&block).map(Vec::as_slice).unwrap_or_default();
            let mut output = HashSet::new();
            for successor in block_successors {
                output.extend(live_in.get(successor).into_iter().flatten().copied());
            }
            if block_successors.is_empty() {
                output.extend(external.iter().copied());
            }

            let exceptional = protected
                .get(&block)
                .into_iter()
                .flatten()
                .flat_map(|catch| live_in.get(catch).into_iter().flatten().copied())
                .collect::<HashSet<_>>();
            let input = transfer_block(module, block, output.clone(), &exceptional, &external, None);

            if live_out.get(&block) != Some(&output) {
                live_out.insert(block, output);
                changed = true;
            }
            if live_in.get(&block) != Some(&input) {
                live_in.insert(block, input);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    for block in blocks {
        let exceptional = protected
            .get(&block)
            .into_iter()
            .flatten()
            .flat_map(|catch| live_in.get(catch).into_iter().flatten().copied())
            .collect::<HashSet<_>>();
        let output = live_out.remove(&block).unwrap_or_default();
        transfer_block(module, block, output, &exceptional, &external, Some(removed));
    }
}

fn transfer_block(
    module: &Module, block: IrNodeId, mut live: HashSet<IrNodeId>, exceptional: &HashSet<IrNodeId>,
    external: &HashSet<IrNodeId>, mut removed: Option<&mut HashSet<IrNodeId>>,
) -> HashSet<IrNodeId> {
    for instruction in module.block(block).unwrap_or_default().iter().rev().copied() {
        live.extend(exceptional.iter().copied());

        let Some(node) = module.node(instruction) else {
            continue;
        };
        match node {
            IrNode::Load { pointer } if tracked_storage(module, *pointer) => {
                live.insert(*pointer);
            },
            IrNode::Store { pointer, .. } => match storage(module, *pointer) {
                Some(Storage::Frame | Storage::External) => {
                    if !live.contains(pointer)
                        && let Some(removed) = removed.as_deref_mut()
                    {
                        removed.insert(instruction);
                    }
                    live.remove(pointer);
                },
                Some(Storage::Observable) | None => live.extend(external.iter().copied()),
            },
            IrNode::Initialize { pointer, .. } => match storage(module, *pointer) {
                Some(Storage::Frame) => {
                    if !live.contains(pointer)
                        && let Some(removed) = removed.as_deref_mut()
                    {
                        removed.insert(instruction);
                    }
                    live.remove(pointer);
                },
                Some(Storage::External | Storage::Observable) | None => {
                    live.extend(external.iter().copied());
                    if tracked_storage(module, *pointer) {
                        live.insert(*pointer);
                    }
                },
            },
            _ if observes_external_state(node) => live.extend(external.iter().copied()),
            _ => {},
        }
    }

    live
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

fn tracked_storage(module: &Module, pointer: IrNodeId) -> bool {
    matches!(storage(module, pointer), Some(Storage::Frame | Storage::External))
}

fn is_observable_name(name: &str) -> bool {
    matches!(
        name,
        "len" | "loc" | "contents" | "x" | "y" | "z" | "type" | "parent_type" | "appearance" | "overlays" | "underlays"
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
    let mut seen = HashSet::new();
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
            append_successors(module.node(*instruction), &mut pending);
        }
    }

    blocks
}

fn block_successors(module: &Module, block: IrNodeId, blocks: &HashSet<IrNodeId>) -> Vec<IrNodeId> {
    let mut successors = Vec::new();
    for instruction in module.block(block).unwrap_or_default() {
        append_successors(module.node(*instruction), &mut successors);
    }
    successors.retain(|successor| blocks.contains(successor));
    successors.sort_by_key(|successor| successor.0);
    successors.dedup();
    successors
}

fn append_successors(node: Option<&IrNode>, successors: &mut Vec<IrNodeId>) {
    match node {
        Some(IrNode::Branch(target)) => successors.push(*target),
        Some(IrNode::ConditionalBranch {
            true_block,
            false_block,
            ..
        }) => {
            successors.push(*true_block);
            successors.push(*false_block);
        },
        Some(IrNode::TryCatch { body, catch, merge }) => {
            successors.push(*body);
            successors.push(*catch);
            successors.push(*merge);
        },
        _ => {},
    }
}

fn protected_catches(module: &Module, blocks: &[IrNodeId]) -> HashMap<IrNodeId, Vec<IrNodeId>> {
    let mut protected = HashMap::<IrNodeId, Vec<IrNodeId>>::new();

    for block in blocks {
        for instruction in module.block(*block).unwrap_or_default() {
            let Some(IrNode::TryCatch { body, catch, merge }) = module.node(*instruction) else {
                continue;
            };
            for protected_block in region_blocks(module, *body, *merge) {
                protected.entry(protected_block).or_default().push(*catch);
            }
        }
    }

    protected
}

fn region_blocks(module: &Module, entry: IrNodeId, stop: IrNodeId) -> Vec<IrNodeId> {
    let mut blocks = Vec::new();
    let mut seen = HashSet::new();
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
            append_successors(module.node(*instruction), &mut pending);
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
                IrNode::Constant(Value::Num(1.0)),
                IrNode::Constant(Value::Num(2.0)),
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
                IrNode::Constant(Value::Num(1.0)),
                IrNode::Constant(Value::Num(2.0)),
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
                IrNode::Constant(Value::Num(1.0)),
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
                IrNode::Constant(Value::Num(1.0)),
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
