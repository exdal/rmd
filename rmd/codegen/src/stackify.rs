use core::types::IrNodeId;
use std::collections::{HashMap, HashSet};

use ir::{IrNode, Procedure, SideEffect};

use crate::{CodegenError, produces_value};

pub(super) struct Stackify {
    schedules: HashMap<IrNodeId, Vec<IrNodeId>>,
    stacked: HashSet<IrNodeId>,
    uses: HashMap<IrNodeId, u32>,
    users: HashMap<IrNodeId, IrNodeId>,
}

impl Stackify {
    pub(super) fn analyze(module: &ir::Module, proc: &Procedure, blocks: &[IrNodeId]) -> Result<Self, CodegenError> {
        let mut uses = HashMap::<IrNodeId, u32>::new();
        let mut users = HashMap::new();

        for block in blocks {
            for instruction in module.block(*block).ok_or(CodegenError::ExpectedBlock(*block))? {
                let node = module
                    .node(*instruction)
                    .ok_or(CodegenError::MissingNode(*instruction))?;
                node.for_each_operand(|operand| {
                    let count = uses.entry(operand).or_default();
                    *count = count.saturating_add(1);
                    users.entry(operand).or_insert(*instruction);
                });
            }
        }
        for parameter in &proc.params {
            if let Some(default) = parameter.default {
                let count = uses.entry(default).or_default();
                *count = count.saturating_add(1);
            }
        }

        let mut schedules = HashMap::new();
        for block in blocks {
            let instructions = module.block(*block).ok_or(CodegenError::ExpectedBlock(*block))?;

            schedules.insert(*block, reorder_pure_runs(module, instructions, &uses, &users));
        }

        let mut stacked = HashSet::new();
        for schedule in schedules.values() {
            for index in (0..schedule.len()).rev() {
                let instruction = schedule[index];
                if stacked.contains(&instruction) {
                    continue;
                }

                let mut cursor = index;
                match_operands(module, schedule, instruction, &mut cursor, &uses, &mut stacked);
            }
        }

        Ok(Self {
            schedules,
            stacked,
            uses,
            users,
        })
    }

    pub(super) fn schedule(&self, block: IrNodeId) -> Option<&[IrNodeId]> {
        self.schedules.get(&block).map(Vec::as_slice)
    }

    pub(super) fn is_stacked(&self, value: IrNodeId) -> bool { self.stacked.contains(&value) }

    pub(super) fn uses(&self, value: IrNodeId) -> u32 { self.uses.get(&value).copied().unwrap_or_default() }

    pub(super) fn needs_local(&self, value: IrNodeId) -> bool { self.uses(value) != 0 && !self.is_stacked(value) }

    pub(super) fn sole_user(&self, value: IrNodeId) -> Option<IrNodeId> {
        if self.uses(value) != 1 {
            return None;
        }
        self.users.get(&value).copied()
    }

    pub(super) fn can_coalesce_phi_source(
        &self, module: &ir::Module, predecessor: IrNodeId, target: IrNodeId, phi: IrNodeId, source: IrNodeId,
    ) -> bool {
        if self.sole_user(source) != Some(phi) || !self.needs_local(source) {
            return false;
        }

        match module.node(source) {
            // parameter slots already contain the incoming value and the sole use
            // check guarantees that overwriting one on other edges is harmless
            Some(IrNode::FunctionParameter(_)) => return true,
            Some(IrNode::Constant(_) | IrNode::Phi { .. }) | None => return false,
            Some(node) if !produces_value(node) => return false,
            Some(_) => {},
        }

        let Some(schedule) = self.schedule(predecessor) else {
            return false;
        };
        let Some(source_position) = schedule.iter().position(|instruction| *instruction == source) else {
            return false;
        };
        let Some(terminator) = schedule.last() else {
            return false;
        };
        if !matches!(module.node(*terminator), Some(IrNode::Branch(actual)) if *actual == target) {
            return false;
        }

        // storing the source directly into the phi local overwrites the phi's
        // current value before the edge, only permit code after the source when
        // it cannot observe that value or transfer control before the edge
        for instruction in &schedule[source_position + 1..schedule.len() - 1] {
            let Some(node) = module.node(*instruction) else {
                return false;
            };
            if node.operands().contains(&phi) || (emits_code(node) && node.effect() != SideEffect::Pure) {
                return false;
            }
        }

        // parallel copies may still need the old phi value as another phi's
        // source, such an edge must keep separate locals until all loads finish
        let Some(target_instructions) = module.block(target) else {
            return false;
        };
        for instruction in target_instructions {
            let Some(IrNode::Phi { operands }) = module.node(*instruction) else {
                continue;
            };

            if operands
                .iter()
                .any(|operand| operand.block == predecessor && operand.value == phi)
            {
                return false;
            }
        }

        true
    }
}

fn reorder_pure_runs(
    module: &ir::Module, instructions: &[IrNodeId], uses: &HashMap<IrNodeId, u32>, users: &HashMap<IrNodeId, IrNodeId>,
) -> Vec<IrNodeId> {
    let mut schedule = Vec::with_capacity(instructions.len());
    let mut start = 0;

    while start < instructions.len() {
        if !module
            .node(instructions[start])
            .is_some_and(|node| node.effect() == SideEffect::Pure)
        {
            schedule.push(instructions[start]);
            start += 1;
            continue;
        }

        let mut end = start + 1;
        while end < instructions.len()
            && module
                .node(instructions[end])
                .is_some_and(|node| node.effect() == SideEffect::Pure)
        {
            end += 1;
        }

        let run = &instructions[start..end];
        let run_set = run.iter().copied().collect::<HashSet<_>>();
        let mut emitted = HashSet::new();
        for instruction in run {
            let is_child = uses.get(instruction).copied() == Some(1)
                && users.get(instruction).is_some_and(|user| run_set.contains(user));
            if !is_child {
                emit_tree(module, *instruction, &run_set, uses, users, &mut emitted, &mut schedule);
            }
        }
        for instruction in run {
            emit_tree(module, *instruction, &run_set, uses, users, &mut emitted, &mut schedule);
        }

        start = end;
    }

    schedule
}

fn emit_tree(
    module: &ir::Module, instruction: IrNodeId, run: &HashSet<IrNodeId>, uses: &HashMap<IrNodeId, u32>,
    users: &HashMap<IrNodeId, IrNodeId>, emitted: &mut HashSet<IrNodeId>, schedule: &mut Vec<IrNodeId>,
) {
    if !emitted.insert(instruction) {
        return;
    }

    if let Some(node) = module.node(instruction) {
        node.for_each_operand(|operand| {
            let is_child = run.contains(&operand)
                && uses.get(&operand).copied() == Some(1)
                && users.get(&operand).is_some_and(|user| *user == instruction);
            if is_child {
                emit_tree(module, operand, run, uses, users, emitted, schedule);
            }
        });
    }

    schedule.push(instruction);
}

fn match_operands(
    module: &ir::Module, schedule: &[IrNodeId], instruction: IrNodeId, cursor: &mut usize,
    uses: &HashMap<IrNodeId, u32>, stacked: &mut HashSet<IrNodeId>,
) {
    let Some(node) = module.node(instruction) else {
        return;
    };

    for operand in node.operands().into_iter().rev() {
        let Some(previous) = previous_emitted(module, schedule, *cursor) else {
            continue;
        };
        if schedule[previous] != operand
            || uses.get(&operand).copied() != Some(1)
            || !module.node(operand).is_some_and(produces_value)
        {
            continue;
        }

        stacked.insert(operand);
        *cursor = previous;
        match_operands(module, schedule, operand, cursor, uses, stacked);
    }
}

fn previous_emitted(module: &ir::Module, schedule: &[IrNodeId], cursor: usize) -> Option<usize> {
    (0..cursor)
        .rev()
        .find(|index| module.node(schedule[*index]).is_some_and(emits_code))
}

fn emits_code(node: &IrNode) -> bool {
    !matches!(
        node,
        IrNode::Constant(_)
            | IrNode::Function(_)
            | IrNode::ExternalFunction(_)
            | IrNode::FunctionParameter(_)
            | IrNode::Label(_)
            | IrNode::Phi { .. }
            | IrNode::Variable(_)
            | IrNode::SelectionMerge { .. }
            | IrNode::LoopMerge { .. }
            | IrNode::Noop
    )
}

#[cfg(test)]
mod tests {
    use core::types::IrNodeId;
    use std::collections::HashMap;

    use ir::{BinaryOp, IrNode, Module, UnaryOp};

    use super::reorder_pure_runs;

    #[test]
    fn reorders_pure_definitions_next_to_their_users() {
        let module = Module {
            nodes: vec![
                IrNode::FunctionParameter(0),
                IrNode::FunctionParameter(1),
                IrNode::Unary {
                    op: UnaryOp::Not,
                    operand: IrNodeId(0),
                },
                IrNode::Unary {
                    op: UnaryOp::Not,
                    operand: IrNodeId(1),
                },
                IrNode::Binary {
                    op: BinaryOp::LogicalAnd,
                    lhs: IrNodeId(2),
                    rhs: IrNodeId(1),
                },
                IrNode::Binary {
                    op: BinaryOp::LogicalOr,
                    lhs: IrNodeId(3),
                    rhs: IrNodeId(0),
                },
            ],
            ..Module::default()
        };
        let uses = HashMap::from([(IrNodeId(0), 1), (IrNodeId(1), 1), (IrNodeId(2), 1), (IrNodeId(3), 1)]);
        let users = HashMap::from([
            (IrNodeId(0), IrNodeId(2)),
            (IrNodeId(1), IrNodeId(3)),
            (IrNodeId(2), IrNodeId(4)),
            (IrNodeId(3), IrNodeId(5)),
        ]);

        let schedule = reorder_pure_runs(
            &module,
            &[IrNodeId(2), IrNodeId(3), IrNodeId(4), IrNodeId(5)],
            &uses,
            &users,
        );

        assert_eq!(schedule, [IrNodeId(2), IrNodeId(4), IrNodeId(3), IrNodeId(5)]);
    }
}
