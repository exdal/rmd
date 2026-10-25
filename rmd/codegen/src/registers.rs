use core::types::IrNodeId;
use std::collections::hash_map::Entry;

use fixedbitset::FixedBitSet;
use ir::{IrNode, Procedure};
use rustc_hash::FxHashMap;

use crate::{CodegenError, RegisterId, produces_value};

#[derive(Default)]
pub(crate) struct Allocation {
    pub(crate) registers: FxHashMap<IrNodeId, RegisterId>,
    pub(crate) phi_temporaries: FxHashMap<IrNodeId, RegisterId>,
    pub(crate) register_count: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct BlockIndex(u32);

impl BlockIndex {
    fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct ValueIndex(u32);

impl ValueIndex {
    fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Default)]
pub(crate) struct Scratch {
    block_index: FxHashMap<IrNodeId, BlockIndex>,
    value_index: FxHashMap<IrNodeId, ValueIndex>,
    values: Vec<IrNodeId>,
    successors: Vec<Vec<BlockIndex>>,
    defs: Vec<FixedBitSet>,
    uses: Vec<FixedBitSet>,
    /// phi sources a block hands to its successors, live at the end of this block only
    edge_uses: Vec<FixedBitSet>,
    live_in: Vec<FixedBitSet>,
    live_out: Vec<FixedBitSet>,
}

impl Scratch {
    fn clear(&mut self, block_count: usize) {
        self.block_index.clear();
        self.value_index.clear();
        self.values.clear();

        if self.successors.len() < block_count {
            self.successors.resize_with(block_count, Vec::new);
        }

        for successors in &mut self.successors {
            successors.clear();
        }
    }

    fn number(
        &mut self, module: &ir::Module, proc: &Procedure, blocks: &[IrNodeId], constants: &[IrNodeId],
    ) -> Result<(), CodegenError> {
        for (index, block) in blocks.iter().enumerate() {
            self.block_index.insert(*block, BlockIndex(index as u32));
        }

        for parameter in &proc.parameters {
            self.add_value(*parameter);
        }

        for constant in constants {
            self.add_value(*constant);
        }

        for block in blocks {
            for instruction in module.block(*block).ok_or(CodegenError::ExpectedBlock(*block))? {
                let node = module
                    .node(*instruction)
                    .ok_or(CodegenError::MissingNode(*instruction))?;
                if produces_value(node) {
                    self.add_value(*instruction);
                }
            }
        }

        Ok(())
    }

    fn add_value(&mut self, node: IrNodeId) {
        if let Entry::Vacant(entry) = self.value_index.entry(node) {
            entry.insert(ValueIndex(self.values.len() as u32));
            self.values.push(node);
        }
    }

    fn collect_successors(&mut self, module: &ir::Module, blocks: &[IrNodeId]) -> Result<(), CodegenError> {
        for (index, block) in blocks.iter().enumerate() {
            let successors = &mut self.successors[index];
            for instruction in module.block(*block).ok_or(CodegenError::ExpectedBlock(*block))? {
                let node = module
                    .node(*instruction)
                    .ok_or(CodegenError::MissingNode(*instruction))?;
                node.for_each_successor(|successor| {
                    if let Some(successor) = self.block_index.get(&successor) {
                        successors.push(*successor);
                    }
                });
            }

            successors.sort_unstable();
            successors.dedup();
        }

        Ok(())
    }

    fn liveness(&mut self, module: &ir::Module, blocks: &[IrNodeId]) -> Result<(), CodegenError> {
        let empty = FixedBitSet::with_capacity(self.values.len());
        for sets in [
            &mut self.defs,
            &mut self.uses,
            &mut self.edge_uses,
            &mut self.live_in,
            &mut self.live_out,
        ] {
            if sets.len() < blocks.len() {
                sets.resize_with(blocks.len(), FixedBitSet::new);
            }

            for set in &mut sets[..blocks.len()] {
                set.clone_from(&empty);
            }
        }

        for (index, block) in blocks.iter().enumerate() {
            for instruction in module.block(*block).ok_or(CodegenError::ExpectedBlock(*block))? {
                let node = module
                    .node(*instruction)
                    .ok_or(CodegenError::MissingNode(*instruction))?;
                if let IrNode::Phi { operands } = node {
                    for operand in operands {
                        if let (Some(predecessor), Some(value)) = (
                            self.block_index.get(&operand.block),
                            self.value_index.get(&operand.value),
                        ) {
                            self.edge_uses[predecessor.index()].insert(value.index());
                        }
                    }
                } else {
                    node.for_each_operand(|operand| {
                        if let Some(value) = self.value_index.get(&operand)
                            && !self.defs[index].contains(value.index())
                        {
                            self.uses[index].insert(value.index());
                        }
                    });
                }

                if let Some(value) = self.value_index.get(instruction) {
                    self.defs[index].insert(value.index());
                }
            }
        }

        let mut output = empty;
        loop {
            let mut is_changed = false;
            for block in (0..blocks.len()).rev() {
                output.clone_from(&self.edge_uses[block]);
                for successor in &self.successors[block] {
                    output.union_with(&self.live_in[successor.index()]);
                }

                if self.live_out[block] != output {
                    self.live_out[block].clone_from(&output);
                    is_changed = true;
                }

                output.difference_with(&self.defs[block]);
                output.union_with(&self.uses[block]);
                if self.live_in[block] != output {
                    self.live_in[block].clone_from(&output);
                    is_changed = true;
                }
            }

            if !is_changed {
                break;
            }
        }

        Ok(())
    }
}

impl Allocation {
    fn fresh(&mut self) -> Result<RegisterId, CodegenError> {
        let register = RegisterId(self.register_count);
        if register.is_invalid() {
            return Err(CodegenError::TooManyRegisters);
        }

        self.register_count += 1;

        Ok(register)
    }

    fn reserve(&mut self, node: IrNodeId) -> Result<RegisterId, CodegenError> {
        if let Some(register) = self.registers.get(&node) {
            return Ok(*register);
        }

        let register = self.fresh()?;
        self.registers.insert(node, register);

        Ok(register)
    }
}

pub(crate) fn allocate(
    scratch: &mut Scratch, module: &ir::Module, proc: &Procedure, blocks: &[IrNodeId], constants: &[IrNodeId],
) -> Result<Allocation, CodegenError> {
    // try and spawn bodies run as nested execute_until calls that resume after the
    // instruction, so the IR control flow does not describe where values stay live
    if has_nested_execution(module, blocks)? {
        return one_per_value(module, proc, blocks, constants);
    }

    greedy(scratch, module, proc, blocks, constants)
}

fn greedy(
    scratch: &mut Scratch, module: &ir::Module, proc: &Procedure, blocks: &[IrNodeId], constants: &[IrNodeId],
) -> Result<Allocation, CodegenError> {
    scratch.clear(blocks.len());
    scratch.number(module, proc, blocks, constants)?;
    scratch.collect_successors(module, blocks)?;
    scratch.liveness(module, blocks)?;

    todo!()
}

fn one_per_value(
    module: &ir::Module, proc: &Procedure, blocks: &[IrNodeId], constants: &[IrNodeId],
) -> Result<Allocation, CodegenError> {
    let mut allocation = Allocation::default();
    for parameter in &proc.parameters {
        allocation.reserve(*parameter)?;
    }

    for constant in constants {
        allocation.reserve(*constant)?;
    }

    for block in blocks {
        for instruction in module.block(*block).ok_or(CodegenError::ExpectedBlock(*block))? {
            let node = module
                .node(*instruction)
                .ok_or(CodegenError::MissingNode(*instruction))?;
            if produces_value(node) {
                allocation.reserve(*instruction)?;
            }

            if matches!(node, IrNode::Phi { .. }) {
                let temporary = allocation.fresh()?;
                allocation.phi_temporaries.insert(*instruction, temporary);
            }
        }
    }

    Ok(allocation)
}

fn has_nested_execution(module: &ir::Module, blocks: &[IrNodeId]) -> Result<bool, CodegenError> {
    for block in blocks {
        for instruction in module.block(*block).ok_or(CodegenError::ExpectedBlock(*block))? {
            if matches!(
                module.node(*instruction),
                Some(IrNode::TryCatch { .. } | IrNode::Spawn { .. })
            ) {
                return Ok(true);
            }
        }
    }

    Ok(false)
}

#[cfg(test)]
mod tests {
    macro_rules! fixture {
        ($path:literal) => {
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/", $path))
        };
    }

    use super::*;
    use crate::{proc_constants, reachable_blocks, tests::lower};

    fn analyzed(module: &ir::Module) -> (Scratch, Vec<IrNodeId>) {
        let proc = &module.procs[0];
        let blocks = reachable_blocks(module, proc.body).expect("blocks");
        let constants = proc_constants(module, proc, &blocks).expect("constants");
        let mut scratch = Scratch::default();
        scratch.clear(blocks.len());
        scratch.number(module, proc, &blocks, &constants).expect("number");
        scratch.collect_successors(module, &blocks).expect("successors");
        scratch.liveness(module, &blocks).expect("liveness");

        (scratch, blocks)
    }

    fn block_with(
        module: &ir::Module, scratch: &Scratch, blocks: &[IrNodeId], is_match: impl Fn(&IrNode) -> bool,
    ) -> BlockIndex {
        let block = blocks
            .iter()
            .find(|block| {
                module
                    .block(**block)
                    .unwrap_or_default()
                    .iter()
                    .any(|instruction| module.node(*instruction).is_some_and(&is_match))
            })
            .expect("block");

        scratch.block_index[block]
    }

    fn value_with(module: &ir::Module, scratch: &Scratch, is_match: impl Fn(&IrNode) -> bool) -> IrNodeId {
        scratch
            .values
            .iter()
            .copied()
            .find(|value| module.node(*value).is_some_and(&is_match))
            .expect("value")
    }

    fn constant(module: &ir::Module, scratch: &Scratch, number: f32) -> IrNodeId {
        value_with(
            module,
            scratch,
            |node| matches!(node, IrNode::Constant(value) if **value == core::types::Value::Num(number)),
        )
    }

    fn sum_of(module: &ir::Module, scratch: &Scratch, lhs: IrNodeId) -> IrNodeId {
        value_with(
            module,
            scratch,
            |node| matches!(node, IrNode::Binary { op: ir::BinaryOp::Add, lhs: operand, .. } if *operand == lhs),
        )
    }

    fn live(scratch: &Scratch, set: &FixedBitSet) -> Vec<IrNodeId> {
        let mut values = set.ones().map(|index| scratch.values[index]).collect::<Vec<_>>();
        values.sort_unstable();
        values
    }

    fn sorted<const N: usize>(mut values: [IrNodeId; N]) -> Vec<IrNodeId> {
        values.sort_unstable();
        values.to_vec()
    }

    #[test]
    fn parameters_come_first_and_every_value_is_numbered_once() {
        let module = lower(fixture!(
            "programs/parameters_come_first_and_every_value_is_numbered_once.dm"
        ));
        let (scratch, blocks) = analyzed(&module);
        let proc = &module.procs[0];

        assert_eq!(proc.parameters.len(), 3);
        assert_eq!(&scratch.values[..3], proc.parameters.as_slice());

        let mut expected = proc_constants(&module, proc, &blocks).expect("constants");
        expected.extend(&proc.parameters);
        for block in &blocks {
            for instruction in module.block(*block).unwrap_or_default() {
                if module.node(*instruction).is_some_and(produces_value) {
                    expected.push(*instruction);
                }
            }
        }

        expected.sort_unstable();
        expected.dedup();
        let mut values = scratch.values.clone();
        values.sort_unstable();
        assert_eq!(values, expected);

        for (index, value) in scratch.values.iter().enumerate() {
            assert_eq!(scratch.value_index[value], ValueIndex(index as u32));
        }
    }

    #[test]
    fn successors_follow_the_loop() {
        let module = lower(fixture!("programs/counting_loop.dm"));
        let (scratch, blocks) = analyzed(&module);
        let entry = scratch.block_index[&module.procs[0].body];
        let header = block_with(&module, &scratch, &blocks, |node| matches!(node, IrNode::Phi { .. }));
        let body = block_with(&module, &scratch, &blocks, |node| {
            matches!(
                node,
                IrNode::Binary {
                    op: ir::BinaryOp::Add,
                    ..
                }
            )
        });
        let exit = block_with(&module, &scratch, &blocks, |node| matches!(node, IrNode::Return(_)));
        let successors = |block: BlockIndex| scratch.successors[block.0 as usize].clone();

        assert_eq!(blocks.len(), 4);
        assert_eq!(successors(entry), [header]);
        let mut expected = vec![body, exit];
        expected.sort_unstable();
        assert_eq!(successors(header), expected);
        assert_eq!(successors(body), [header]);
        assert!(successors(exit).is_empty());
    }

    #[test]
    fn loop_values_stay_live_around_the_back_edge() {
        let module = lower(fixture!("programs/counting_loop.dm"));
        let (scratch, blocks) = analyzed(&module);
        let n = module.procs[0].parameters[0];
        let zero = constant(&module, &scratch, 0.0);
        let one = constant(&module, &scratch, 1.0);
        let i = value_with(&module, &scratch, |node| matches!(node, IrNode::Phi { .. }));
        let next = sum_of(&module, &scratch, i);
        let entry = scratch.block_index[&module.procs[0].body].index();
        let header = block_with(&module, &scratch, &blocks, |node| matches!(node, IrNode::Phi { .. })).index();
        let body = block_with(&module, &scratch, &blocks, |node| {
            matches!(
                node,
                IrNode::Binary {
                    op: ir::BinaryOp::Add,
                    ..
                }
            )
        })
        .index();
        let exit = block_with(&module, &scratch, &blocks, |node| matches!(node, IrNode::Return(_))).index();

        assert_eq!(live(&scratch, &scratch.live_in[entry]), sorted([n, zero, one]));
        assert_eq!(live(&scratch, &scratch.live_out[entry]), sorted([n, zero, one]));
        assert_eq!(live(&scratch, &scratch.live_in[header]), sorted([n, one]));
        assert_eq!(live(&scratch, &scratch.live_out[header]), sorted([n, one, i]));
        assert_eq!(live(&scratch, &scratch.live_in[body]), sorted([n, one, i]));
        assert_eq!(live(&scratch, &scratch.live_out[body]), sorted([n, one, next]));
        assert_eq!(live(&scratch, &scratch.live_in[exit]), [i]);
        assert!(scratch.live_out[exit].is_clear());
    }

    #[test]
    fn phi_sources_are_live_only_on_their_own_edge() {
        let module = lower(fixture!("programs/phi_sources_are_live_only_on_their_own_edge.dm"));
        let (scratch, blocks) = analyzed(&module);
        let [condition, a, b] = module.procs[0].parameters[..] else {
            panic!("three parameters");
        };
        let one = constant(&module, &scratch, 1.0);
        let two = constant(&module, &scratch, 2.0);
        let left = sum_of(&module, &scratch, a);
        let right = sum_of(&module, &scratch, b);
        let entry = scratch.block_index[&module.procs[0].body].index();
        let then = block_with(
            &module,
            &scratch,
            &blocks,
            |node| matches!(node, IrNode::Binary { lhs, .. } if *lhs == a),
        )
        .index();
        let otherwise = block_with(
            &module,
            &scratch,
            &blocks,
            |node| matches!(node, IrNode::Binary { lhs, .. } if *lhs == b),
        )
        .index();
        let merge = block_with(&module, &scratch, &blocks, |node| matches!(node, IrNode::Phi { .. })).index();

        assert_eq!(
            live(&scratch, &scratch.live_in[entry]),
            sorted([condition, a, b, one, two])
        );
        assert_eq!(live(&scratch, &scratch.live_out[entry]), sorted([a, b, one, two]));
        assert_eq!(live(&scratch, &scratch.live_in[then]), sorted([a, one]));
        assert_eq!(live(&scratch, &scratch.live_out[then]), [left]);
        assert_eq!(live(&scratch, &scratch.live_in[otherwise]), sorted([b, two]));
        assert_eq!(live(&scratch, &scratch.live_out[otherwise]), [right]);
        assert!(scratch.live_in[merge].is_clear());
        assert!(scratch.live_out[merge].is_clear());
    }
}
