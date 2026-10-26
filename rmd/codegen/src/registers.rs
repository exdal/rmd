// Reference: Hack, Grund, Goos, "Register allocation for programs in SSA-form" (CC 2006)
//            https://bernsteinbear.com/assets/img/ra-programs-ssa.pdf
//            For phi coalescing (the task of eliminating useless copies) we use the
//            "coloring" method this paper talks about instead of going full on aggressive
//            graph building pass:
//            "We consider coalescing as the task of obtaining a good coloring with respect to
//            φ-operations. [...] The advantage over merging the node of y with the nodes of the
//            xi in the interference graph is, that we do not modify the graph's structure (i.e.
//            possibly rendering it non-chordal)" (section 4)
// Never forget this: phis are __permutations__, just reading the code is confusing
// to anyone who reads this code without a proper mental model. One phi picks one source per
// incoming edge (source = the value on that edge ie: `phi [%12 from block %7] ...`), and all
// phis of a block run at once when the block is entered, so on each edge they move registers
// like one permutation:
// "all φ-operations in a basic block have to be executed simultaneously before all other
// instructions in that basic block" (4.2)
// "So generally φ-operations work like permutations on registers and not like a set of
// copies." (4.2)
// ```dm
// if(foo)
//      bar = 2
// else
//      bar = 3
// return bar
// ```
// `return_value` returns the phi, which has 2 incoming sources, one per edge:
// `phi [2 from the then block, 3 from the else block]`. When the phi shares a register with
// a source, that edge maps the register to itself and needs no copy:
// "coalescing is the task of finding a register allocation in which the permutations will have
// as many fixed points (registers that are mapped to themselves) as possible." (4.2)
//

use core::types::IrNodeId;
use std::collections::hash_map::Entry;

use fixedbitset::FixedBitSet;
use ir::{IrNode, Procedure};
use rustc_hash::FxHashMap;

use crate::{CodegenError, RegisterId, for_each_loaded_constant, produces_value};

#[derive(Default)]
pub(crate) struct Allocation {
    pub(crate) registers: FxHashMap<IrNodeId, RegisterId>,
    pub(crate) phi_temporaries: FxHashMap<IrNodeId, RegisterId>,
    /// per instruction, the constants loaded right before it and the registers they load into
    pub(crate) constant_loads: FxHashMap<IrNodeId, Vec<(IrNodeId, RegisterId)>>,
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
    order: Vec<BlockIndex>,
    assigned: Vec<RegisterId>,
    /// the phi a value is a source of, its register is the value's coalescing hint
    feeds: Vec<Option<ValueIndex>>,
    occupied: FixedBitSet,
    live: FixedBitSet,
    dead: FixedBitSet,
    dying: Vec<Vec<ValueIndex>>,
    loads: Vec<IrNodeId>,
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

    fn number(&mut self, module: &ir::Module, proc: &Procedure, blocks: &[IrNodeId]) -> Result<(), CodegenError> {
        for (index, block) in blocks.iter().enumerate() {
            self.block_index.insert(*block, BlockIndex(index as u32));
        }

        for parameter in &proc.parameters {
            self.add_value(*parameter);
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

    // Definition 3 (Interference): "We say, two variables interfere if there exists a label in
    // the program where they are both live."
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

    /// Definition 2 (Strict program): "A program is strict, if each usage of a variable v is
    /// dominated by Dv." (Dv is the label where v is defined)
    /// Only parameters exist before the first instruction, anything else live into the entry is
    /// read on some path before it is written
    fn is_strict(&self, module: &ir::Module, entry: BlockIndex) -> bool {
        self.live_in[entry.index()]
            .ones()
            .all(|index| matches!(module.node(self.values[index]), Some(IrNode::FunctionParameter(_))))
    }

    // Reverse postorder is the output of a postorder depth-first search (DFS) flipped
    // backwards. Postorder records a block only after everything below it is finished, so
    // flipped, a block comes after every block that leads into it (back edges aside), and after
    // every block that dominates it. That's the order the paper colors in:
    // "Theorem 2. A variable v can be added to a PEO of G if all variables whose definitions
    // are dominated by the definition of v have already been added to the PEO."
    // "Thus, a PEO of a SSA interference graph's nodes can be easily obtained by a post order
    // walk over the program's dominance tree."
    // "re-insert the nodes in reverse order and give each vi the first free color not used by
    // its already re-inserted neighbors." (section 3)
    fn reverse_postorder(&mut self, entry: BlockIndex, block_count: usize) {
        self.order.clear();

        let mut visited = FixedBitSet::with_capacity(block_count);
        visited.insert(entry.index());

        let mut stack = vec![(entry, 0)];
        while let Some(top) = stack.len().checked_sub(1) {
            let (block, next) = stack[top];
            if let Some(successor) = self.successors[block.index()].get(next).copied() {
                stack[top].1 += 1;
                if !visited.put(successor.index()) {
                    stack.push((successor, 0));
                }
            } else {
                // every successor is done, so this block is finished: record it (btw this is what all the yapping is about)
                self.order.push(block);
                stack.pop();
            }
        }

        self.order.reverse();
    }

    fn assign(
        &mut self, module: &ir::Module, proc: &Procedure, blocks: &[IrNodeId],
    ) -> Result<Option<Allocation>, CodegenError> {
        let value_count = self.values.len();

        self.assigned.clear();
        self.assigned.resize(value_count, RegisterId::INVALID);

        self.feeds.clear();
        self.feeds.resize(value_count, None);

        for block in blocks {
            for instruction in module.block(*block).ok_or(CodegenError::ExpectedBlock(*block))? {
                let Some(IrNode::Phi { operands }) = module.node(*instruction) else {
                    continue;
                };

                let phi = self.value_index[instruction];
                for operand in operands {
                    if let Some(value) = self.value_index.get(&operand.value) {
                        self.feeds[value.index()].get_or_insert(phi); // a value feeding several phis keeps the first one
                    }
                }
            }
        }

        // the VM binds arguments to r0..n and reads them back for `args`, so parameters never move or free
        let parameters = proc.parameters.len();
        if parameters >= usize::from(u16::MAX) {
            return Err(CodegenError::TooManyRegisters);
        }

        // the length is the register count so far, start it at this proc's parameters
        self.occupied.clone_from(&FixedBitSet::with_capacity(parameters));
        self.occupied.insert_range(..parameters);
        for (index, parameter) in proc.parameters.iter().enumerate() {
            self.assigned[self.value_index[parameter].index()] = RegisterId(index as u16);
        }

        let mut constant_loads = FxHashMap::default();
        let order = std::mem::take(&mut self.order);
        let mut phi_count = 0;
        for block in &order {
            let index = block.index();
            let instructions = module
                .block(blocks[index])
                .ok_or(CodegenError::ExpectedBlock(blocks[index]))?;

            // "Lemma 2. If v, w interfere and Dv ⪰ Dw, then v is live at Dw." Everything a value
            // here interferes with is live here, so live_in is all the block needs to know
            self.occupied.clear();
            self.occupied.insert_range(..parameters);
            for value in self.live_in[index].ones() {
                let register = self.assigned[value];
                if register.is_invalid() {
                    self.order = order;
                    return Ok(None);
                }

                self.occupied.insert(usize::from(register.0));
            }

            self.live.clone_from(&self.live_out[index]);
            self.dead.clear();
            self.dead.grow(value_count);
            if self.dying.len() < instructions.len() {
                self.dying.resize_with(instructions.len(), Vec::new);
            }

            for dying in &mut self.dying[..instructions.len()] {
                dying.clear();
            }

            for (position, instruction) in instructions.iter().enumerate().rev() {
                let node = module
                    .node(*instruction)
                    .ok_or(CodegenError::MissingNode(*instruction))?;
                if let Some(value) = self.value_index.get(instruction) {
                    if !self.live.contains(value.index()) {
                        self.dead.insert(value.index());
                    }

                    self.live.set(value.index(), false);
                }

                if !matches!(node, IrNode::Phi { .. }) {
                    node.for_each_operand(|operand| {
                        if let Some(value) = self.value_index.get(&operand)
                            && !self.live.put(value.index())
                        {
                            self.dying[position].push(*value);
                        }
                    });
                }
            }

            // phis are all written by the incoming edge at once, so a dead one frees only after the last
            let mut phis = 0;
            for instruction in instructions {
                let Some(IrNode::Phi { operands }) = module.node(*instruction) else {
                    break;
                };

                // "If we can assign as many of the xi the color of y, we save move operations on the
                // respective edges to the φ's block." (section 4)
                let hint = operands
                    .iter()
                    .filter_map(|operand| self.value_index.get(&operand.value))
                    .map(|source| self.assigned[source.index()])
                    .find(|register| register.is_valid() && !self.occupied.contains(usize::from(register.0)));
                let value = self.value_index[instruction].index();
                self.assigned[value] = take(&mut self.occupied, hint)?;
                phis += 1;
            }

            phi_count = phi_count.max(phis);
            for instruction in &instructions[..phis] {
                let value = self.value_index[instruction].index();
                if self.dead.contains(value) {
                    release(&mut self.occupied, self.assigned[value], parameters);
                }
            }

            for (position, instruction) in instructions.iter().enumerate().skip(phis) {
                let node = module
                    .node(*instruction)
                    .ok_or(CodegenError::MissingNode(*instruction))?;

                // a constant loads right before the instruction that reads it and dies there, so it
                // takes a register while the operands are still held and frees it with them
                self.loads.clear();
                for_each_loaded_constant(module, node, |constant| {
                    if !self.loads.contains(&constant) {
                        self.loads.push(constant);
                    }
                });

                let mut loads = Vec::with_capacity(self.loads.len());
                for constant in &self.loads {
                    loads.push((*constant, take(&mut self.occupied, None)?));
                }

                for value in &self.dying[position] {
                    release(&mut self.occupied, self.assigned[value.index()], parameters);
                }

                for (_, register) in &loads {
                    release(&mut self.occupied, *register, parameters);
                }

                if !loads.is_empty() {
                    constant_loads.insert(*instruction, loads);
                }

                let Some(value) = self.value_index.get(instruction).map(|value| value.index()) else {
                    continue;
                };

                // same idea from the other side: a back-edge source asks for its phi's color
                let hint = self.feeds[value].map(|phi| self.assigned[phi.index()]);
                self.assigned[value] = take(&mut self.occupied, hint)?;
                if self.dead.contains(value) {
                    release(&mut self.occupied, self.assigned[value], parameters);
                }
            }
        }

        self.order = order;

        let mut allocation = Allocation {
            constant_loads,
            ..Allocation::default()
        };
        for (index, register) in self.assigned.iter().enumerate() {
            if register.is_invalid() {
                return Ok(None);
            }

            allocation.registers.insert(self.values[index], *register);
        }

        // emit_phi_copies routes cyclic copies through these, above every value so they never clash
        // "each permutation of size n can be written as a sequence of transpositions (swaps) and
        // thus is implementable using n registers, using no extra register. [...] However, if one
        // register is spare at the φs, we can use it to implement the permutation with moves." (4.2)
        let base = self.occupied.len();
        for block in blocks {
            let phis = module
                .block(*block)
                .ok_or(CodegenError::ExpectedBlock(*block))?
                .iter()
                .take_while(|instruction| matches!(module.node(**instruction), Some(IrNode::Phi { .. })));
            for (position, phi) in phis.enumerate() {
                allocation.phi_temporaries.insert(*phi, register_at(base + position)?);
            }
        }

        allocation.register_count = u16::try_from(base + phi_count)
            .ok()
            .filter(|count| *count != u16::MAX)
            .ok_or(CodegenError::TooManyRegisters)?;

        Ok(Some(allocation))
    }
}

fn register_at(index: usize) -> Result<RegisterId, CodegenError> {
    u16::try_from(index)
        .ok()
        .map(RegisterId)
        .filter(|register| register.is_valid())
        .ok_or(CodegenError::TooManyRegisters)
}

fn take(occupied: &mut FixedBitSet, preferred: Option<RegisterId>) -> Result<RegisterId, CodegenError> {
    let index = match preferred {
        Some(register) if register.is_valid() && !occupied.contains(usize::from(register.0)) => usize::from(register.0),
        _ => occupied.zeroes().next().unwrap_or(occupied.len()),
    };

    let register = register_at(index)?;
    occupied.grow(index + 1);
    occupied.insert(index);

    Ok(register)
}

fn release(occupied: &mut FixedBitSet, register: RegisterId, parameters: usize) {
    if usize::from(register.0) >= parameters {
        occupied.set(usize::from(register.0), false);
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
    scratch: &mut Scratch, module: &ir::Module, proc: &Procedure, blocks: &[IrNodeId],
) -> Result<Allocation, CodegenError> {
    // try and spawn bodies run as nested execute_until calls that resume after the
    // instruction, so the IR control flow does not describe where values stay live
    if has_nested_execution(module, blocks)? {
        return one_per_value(module, proc, blocks);
    }

    match greedy(scratch, module, proc, blocks)? {
        Some(allocation) => Ok(allocation),
        None => one_per_value(module, proc, blocks),
    }
}

// Theorem 1, 2:
// Interference graph of an SSA program is chordal[1], and the dominance order is a perfect
// elimination order for it, so coloring in that order with the first free color is optimal.
// "Theorem 1. For each clique C = {c1, ..., cn} ⊆ V in the interference graph G = (V, E) of a
// SSA-form program P, there exists a label ℓ ∈ LabelsP where all c1, ..., cn are live."
// "Theorem 2 together with definition 4 state that the chromatic number of a SSA interference
// graph is determined by the largest clique in the graph." (section 4)
// So the registers we need = the most values live at one point, no backtracking.
//
// 1: every cycle of 4 or more nodes has a chord (an edge between two nodes of the cycle that
//    aren't next to each other on it):
// not chordal                   chordal
// a ── b                        a ── b
// │    │                        │ ╲  │
// │    │                        │  ╲ │
// d ── c                        d ── c
// a-b-c-d-a has no chord        a-c is a chord
fn greedy(
    scratch: &mut Scratch, module: &ir::Module, proc: &Procedure, blocks: &[IrNodeId],
) -> Result<Option<Allocation>, CodegenError> {
    scratch.clear(blocks.len());
    scratch.number(module, proc, blocks)?;
    scratch.collect_successors(module, blocks)?;
    scratch.liveness(module, blocks)?;

    let entry = scratch.block_index[&proc.body];
    if !scratch.is_strict(module, entry) {
        return Ok(None);
    }

    scratch.reverse_postorder(entry, blocks.len());
    scratch.assign(module, proc, blocks)
}

fn one_per_value(module: &ir::Module, proc: &Procedure, blocks: &[IrNodeId]) -> Result<Allocation, CodegenError> {
    let mut allocation = Allocation::default();
    for parameter in &proc.parameters {
        allocation.reserve(*parameter)?;
    }

    let mut constants = FxHashMap::default();
    for block in blocks {
        for instruction in module.block(*block).ok_or(CodegenError::ExpectedBlock(*block))? {
            let node = module
                .node(*instruction)
                .ok_or(CodegenError::MissingNode(*instruction))?;
            let mut loads = Vec::new();
            for_each_loaded_constant(module, node, |constant| {
                if !loads.iter().any(|(loaded, _)| *loaded == constant) {
                    loads.push((constant, RegisterId::INVALID));
                }
            });

            for (constant, register) in &mut loads {
                *register = match constants.entry(*constant) {
                    Entry::Occupied(entry) => *entry.get(),
                    Entry::Vacant(entry) => *entry.insert(allocation.fresh()?),
                };
            }

            if !loads.is_empty() {
                allocation.constant_loads.insert(*instruction, loads);
            }

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
    use crate::{reachable_blocks, tests::lower};

    fn analyzed(module: &ir::Module) -> (Scratch, Vec<IrNodeId>) {
        let proc = &module.procs[0];
        let blocks = reachable_blocks(module, proc.body).expect("blocks");
        let mut scratch = Scratch::default();
        scratch.clear(blocks.len());
        scratch.number(module, proc, &blocks).expect("number");
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

    // ; constants
    //  %5 = constant 3
    // %11 = constant 1
    //
    //  %0 = function /::choose vars=3
    //  %1 =   function_parameter #0
    //  %2 =   function_parameter #1
    //  %3 =   function_parameter #2
    //
    //  %4 =   label
    //  %9 =     selection_merge merge=%6
    // %10 =     conditional_branch %1 %7 %6
    //
    //  %7 =   label
    // %12 =     binary add %2 %11
    // %13 =     return %12
    //
    //  %6 =   label
    // %15 =     binary add %3 %11
    // %16 =     return %15
    #[test]
    fn parameters_come_first_and_every_value_is_numbered_once() {
        let module = lower(fixture!(
            "programs/parameters_come_first_and_every_value_is_numbered_once.dm"
        ));
        let (scratch, blocks) = analyzed(&module);
        let proc = &module.procs[0];

        assert_eq!(proc.parameters.len(), 3);
        assert_eq!(&scratch.values[..3], proc.parameters.as_slice());

        let mut expected = proc.parameters.clone();
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

        for constant in &module.constants {
            assert!(!scratch.value_index.contains_key(constant));
        }
    }

    // ; constants
    //  %3 = constant 0
    // %13 = constant 1
    //
    //  %0 = function /::count vars=2
    //  %1 =   function_parameter #0
    //
    //  %2 =   label
    //  %7 =     branch %4
    //
    //  %4 =   label
    //  %8 =     phi [%3 from block %2] [%14 from block %5]
    // %10 =     binary comp_less %8 %1
    // %11 =     loop_merge merge=%6 continue=%4
    // %12 =     conditional_branch %10 %5 %6
    //
    //  %5 =   label
    // %14 =     binary add %8 %13
    // %15 =     branch %4
    //
    //  %6 =   label
    // %16 =     return %8
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

    // ; constants
    //  %3 = constant 0
    // %13 = constant 1
    //
    //  %0 = function /::count vars=2
    //  %1 =   function_parameter #0
    //
    //  %2 =   label
    //  %7 =     branch %4
    //
    //  %4 =   label
    //  %8 =     phi [%3 from block %2] [%14 from block %5]
    // %10 =     binary comp_less %8 %1
    // %11 =     loop_merge merge=%6 continue=%4
    // %12 =     conditional_branch %10 %5 %6
    //
    //  %5 =   label
    // %14 =     binary add %8 %13
    // %15 =     branch %4
    //
    //  %6 =   label
    // %16 =     return %8
    #[test]
    fn loop_values_stay_live_around_the_back_edge() {
        let module = lower(fixture!("programs/counting_loop.dm"));
        let (scratch, blocks) = analyzed(&module);
        let n = module.procs[0].parameters[0];
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

        assert_eq!(live(&scratch, &scratch.live_in[entry]), [n]);
        assert_eq!(live(&scratch, &scratch.live_out[entry]), [n]);
        assert_eq!(live(&scratch, &scratch.live_in[header]), [n]);
        assert_eq!(live(&scratch, &scratch.live_out[header]), sorted([n, i]));
        assert_eq!(live(&scratch, &scratch.live_in[body]), sorted([n, i]));
        assert_eq!(live(&scratch, &scratch.live_out[body]), sorted([n, next]));
        assert_eq!(live(&scratch, &scratch.live_in[exit]), [i]);
        assert!(scratch.live_out[exit].is_clear());
    }

    // ; constants
    //  %5 = constant null
    // %11 = constant 1
    // %14 = constant 2
    //
    //  %0 = function /::select vars=4
    //  %1 =   function_parameter #0
    //  %2 =   function_parameter #1
    //  %3 =   function_parameter #2
    //
    //  %4 =   label
    //  %9 =     selection_merge merge=%6
    // %10 =     conditional_branch %1 %7 %8
    //
    //  %7 =   label
    // %12 =     binary add %2 %11
    // %13 =     branch %6
    //
    //  %6 =   label
    // %17 =     phi [%12 from block %7] [%15 from block %8]
    // %18 =     return %17
    //
    //  %8 =   label
    // %15 =     binary add %3 %14
    // %16 =     branch %6
    #[test]
    fn phi_sources_are_live_only_on_their_own_edge() {
        let module = lower(fixture!("programs/phi_sources_are_live_only_on_their_own_edge.dm"));
        let (scratch, blocks) = analyzed(&module);
        let [condition, a, b] = module.procs[0].parameters[..] else {
            panic!("three parameters");
        };
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

        assert_eq!(live(&scratch, &scratch.live_in[entry]), sorted([condition, a, b]));
        assert_eq!(live(&scratch, &scratch.live_out[entry]), sorted([a, b]));
        assert_eq!(live(&scratch, &scratch.live_in[then]), [a]);
        assert_eq!(live(&scratch, &scratch.live_out[then]), [left]);
        assert_eq!(live(&scratch, &scratch.live_in[otherwise]), [b]);
        assert_eq!(live(&scratch, &scratch.live_out[otherwise]), [right]);
        assert!(scratch.live_in[merge].is_clear());
        assert!(scratch.live_out[merge].is_clear());
    }
}
