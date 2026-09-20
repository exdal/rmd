// https://dl.acm.org/doi/10.1145/103135.103136
// https://www.cs.cornell.edu/courses/cs6120/2019fa/blog/sccp

use core::{
    path::{PathFlags, TreePath},
    types::{IrNodeId, Value},
};
use std::collections::{HashMap, HashSet, VecDeque};

use super::simplify_phis::{replace_metadata_uses, replace_operands};
use crate::{BinaryOp, IrNode, Module, PhiOperand, UnaryOp};

pub fn sparse_constant_propagation(module: &mut Module) {
    let analysis = Solver::new(module).run();

    let removed_edges = rewrite_constant_branches(module, &analysis);

    let mut constant_ids = scalar_constant_ids(module);
    let mut replacements = HashMap::new();
    for id in analysis.uses.candidates() {
        let Some(Lattice::Constant(value)) = analysis.values.get(id.0 as usize) else {
            continue;
        };

        let constant = match value {
            ConstantRef::Module(id) => *id,
            ConstantRef::Folded(index) => {
                intern_constant(module, &mut constant_ids, analysis.folded_values[*index].clone())
            },
        };
        replacements.insert(*id, constant);
    }

    if !replacements.is_empty() {
        for replaced in replacements.keys() {
            for user in analysis.uses.get(*replaced) {
                if let Some(node) = module.nodes.get_mut(user.0 as usize) {
                    replace_operands(node, &replacements);
                }
            }
        }
        replace_metadata_uses(module, &replacements);

        let affected_blocks = replacements
            .keys()
            .filter_map(|instruction| analysis.instruction_blocks.get(instruction.0 as usize))
            .copied()
            .filter(|block| block.is_valid())
            .collect::<HashSet<_>>();
        for block in affected_blocks {
            if let Some(IrNode::Label(instructions)) = module.nodes.get_mut(block.0 as usize) {
                instructions.retain(|instruction| !replacements.contains_key(instruction));
            }
        }
        for instruction in replacements.keys() {
            module.nodes[instruction.0 as usize] = IrNode::Noop;
        }
    }

    if !removed_edges.is_empty() {
        let mut affected_phis = Vec::new();
        for (predecessor, target) in removed_edges {
            remove_phi_edge(module, predecessor, target, &mut affected_phis);
        }

        simplify_affected_phis(module, affected_phis, &analysis, &mut constant_ids);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConstantRef {
    Module(IrNodeId),
    Folded(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lattice {
    Unknown,
    Constant(ConstantRef),
    Overdefined,
}

struct Analysis {
    values: Vec<Lattice>,
    folded_values: Vec<Value>,
    executable_blocks: Vec<IrNodeId>,
    uses: CandidateUses,
    instruction_blocks: Vec<IrNodeId>,
}

struct Solver<'a> {
    module: &'a Module,
    values: Vec<Lattice>,
    folded_values: Vec<Value>,
    uses: CandidateUses,
    instruction_blocks: Vec<IrNodeId>,
    executable_blocks: Vec<bool>,
    executable_block_ids: Vec<IrNodeId>,
    executable_edges: HashSet<u64>,
    pending_blocks: VecDeque<IrNodeId>,
    pending_instructions: VecDeque<IrNodeId>,
    queued_instructions: Vec<bool>,
}

impl<'a> Solver<'a> {
    fn new(module: &'a Module) -> Self {
        let values = module
            .nodes
            .iter()
            .enumerate()
            .map(|(index, node)| match node {
                IrNode::Constant(value) if scalar(value).is_some() => {
                    Lattice::Constant(ConstantRef::Module(IrNodeId(index as u32)))
                },
                IrNode::Phi { .. } | IrNode::Unary { .. } | IrNode::Binary { .. } => Lattice::Unknown,
                _ => Lattice::Overdefined,
            })
            .collect::<Vec<_>>();
        let mut uses = CandidateUses::new(module);
        let mut instruction_blocks = vec![IrNodeId::INVALID; module.nodes.len()];
        let mut block_count = 0;

        for (index, node) in module.nodes.iter().enumerate() {
            let user = IrNodeId(index as u32);
            node.for_each_operand(|operand| uses.add(operand, user));
            if let IrNode::Label(instructions) = node {
                block_count += 1;
                for instruction in instructions {
                    if let Some(block) = instruction_blocks.get_mut(instruction.0 as usize) {
                        *block = user;
                    }
                }
            }
        }

        let node_count = module.nodes.len();
        let candidate_count = uses.candidates().len();

        Self {
            module,
            values,
            folded_values: Vec::with_capacity(candidate_count / 4),
            uses,
            instruction_blocks,
            executable_blocks: vec![false; node_count],
            executable_block_ids: Vec::with_capacity(block_count),
            executable_edges: HashSet::with_capacity(block_count),
            pending_blocks: VecDeque::with_capacity(module.procs.len()),
            pending_instructions: VecDeque::with_capacity(candidate_count),
            queued_instructions: vec![false; node_count],
        }
    }

    fn run(mut self) -> Analysis {
        for entry in self.module.procs.iter().map(|proc| proc.body) {
            self.mark_block(entry);
        }

        while !self.pending_blocks.is_empty() || !self.pending_instructions.is_empty() {
            if let Some(block) = self.pending_blocks.pop_front() {
                for instruction in self.module.block(block).unwrap_or_default() {
                    if self.module.node(*instruction).is_some_and(is_solver_instruction) {
                        self.queue_instruction(*instruction);
                    }
                }
                continue;
            }

            if let Some(instruction) = self.pending_instructions.pop_front() {
                self.queued_instructions[instruction.0 as usize] = false;
                self.visit_instruction(instruction);
            }
        }

        Analysis {
            values: self.values,
            folded_values: self.folded_values,
            executable_blocks: self.executable_block_ids,
            uses: self.uses,
            instruction_blocks: self.instruction_blocks,
        }
    }

    fn visit_instruction(&mut self, instruction: IrNodeId) {
        let Some(block) = self.instruction_blocks.get(instruction.0 as usize).copied() else {
            return;
        };
        if block.is_invalid() {
            return;
        }
        if !self.is_executable(block) {
            return;
        }
        let Some(node) = self.module.node(instruction).map(SolverInstruction::from) else {
            return;
        };

        match &node {
            SolverInstruction::Branch(target) => self.mark_edge(block, *target),
            SolverInstruction::ConditionalBranch {
                condition,
                true_block,
                false_block,
            } => match self.value(*condition) {
                Lattice::Unknown => {},
                Lattice::Constant(value) => {
                    let truthy = self.constant(value).is_some_and(Value::is_truthy);
                    self.mark_edge(block, if truthy { *true_block } else { *false_block });
                },
                Lattice::Overdefined => {
                    self.mark_edge(block, *true_block);
                    self.mark_edge(block, *false_block);
                },
            },
            // these blocks are entered by the VM's structured exception machinery rather than a
            // normal branch, both must remain conservatively executable
            SolverInstruction::TryCatch { body, catch } => {
                self.mark_block(*body);
                self.mark_block(*catch);
            },
            _ => {},
        }

        let next = match node {
            SolverInstruction::Phi(operands) => {
                let mut result = Lattice::Unknown;
                for operand in operands {
                    if self.executable_edges.contains(&edge_key(operand.block, block)) {
                        result = self.join(result, self.value(operand.value));
                    }
                }
                Some(result)
            },
            SolverInstruction::Unary { op, operand } => Some(self.evaluate_unary(op, operand)),
            SolverInstruction::Binary { op, lhs, rhs } => Some(self.evaluate_binary(op, lhs, rhs)),
            _ => None,
        };

        if let Some(next) = next {
            self.update(instruction, next);
        }
    }

    fn evaluate_unary(&mut self, op: UnaryOp, operand: IrNodeId) -> Lattice {
        match self.value(operand) {
            Lattice::Unknown => Lattice::Unknown,
            Lattice::Overdefined => Lattice::Overdefined,
            Lattice::Constant(value) => {
                let folded = self.constant(value).and_then(|value| fold_unary(op, value));
                self.folded(folded)
            },
        }
    }

    fn evaluate_binary(&mut self, op: BinaryOp, lhs: IrNodeId, rhs: IrNodeId) -> Lattice {
        match (self.value(lhs), self.value(rhs)) {
            (Lattice::Overdefined, _) | (_, Lattice::Overdefined) => Lattice::Overdefined,
            (Lattice::Unknown, _) | (_, Lattice::Unknown) => Lattice::Unknown,
            (Lattice::Constant(lhs), Lattice::Constant(rhs)) => {
                let folded = match (self.constant(lhs), self.constant(rhs)) {
                    (Some(lhs), Some(rhs)) => fold_binary(op, lhs, rhs),
                    _ => None,
                };
                self.folded(folded)
            },
        }
    }

    fn value(&self, id: IrNodeId) -> Lattice { self.values.get(id.0 as usize).copied().unwrap_or(Lattice::Overdefined) }

    fn constant(&self, value: ConstantRef) -> Option<&Value> {
        match value {
            ConstantRef::Module(id) => match self.module.node(id) {
                Some(IrNode::Constant(value)) => Some(value),
                _ => None,
            },
            ConstantRef::Folded(index) => self.folded_values.get(index),
        }
    }

    fn folded(&mut self, value: Option<Value>) -> Lattice {
        let Some(value) = value else {
            return Lattice::Overdefined;
        };
        let index = self.folded_values.len();
        self.folded_values.push(value);
        Lattice::Constant(ConstantRef::Folded(index))
    }

    fn join(&self, left: Lattice, right: Lattice) -> Lattice {
        match (left, right) {
            (Lattice::Unknown, other) | (other, Lattice::Unknown) => other,
            (Lattice::Constant(left), Lattice::Constant(right)) if self.constant(left) == self.constant(right) => {
                Lattice::Constant(left)
            },
            _ => Lattice::Overdefined,
        }
    }

    fn update(&mut self, instruction: IrNodeId, next: Lattice) {
        let Some(current) = self.values.get(instruction.0 as usize).copied() else {
            return;
        };
        let merged = self.join(current, next);
        if merged == current {
            return;
        }
        self.values[instruction.0 as usize] = merged;

        for user in self.uses.get(instruction) {
            if !self.module.node(*user).is_some_and(is_solver_instruction) {
                continue;
            }
            let index = user.0 as usize;
            if self.queued_instructions.get(index) == Some(&false) {
                self.queued_instructions[index] = true;
                self.pending_instructions.push_back(*user);
            }
        }
    }

    fn mark_block(&mut self, block: IrNodeId) {
        let index = block.0 as usize;
        if self.executable_blocks.get(index) == Some(&false) {
            self.executable_blocks[index] = true;
            self.executable_block_ids.push(block);
            self.pending_blocks.push_back(block);
        }
    }

    fn mark_edge(&mut self, from: IrNodeId, to: IrNodeId) {
        if !self.executable_edges.insert(edge_key(from, to)) {
            return;
        }
        let was_executable = self.is_executable(to);
        self.mark_block(to);
        if was_executable {
            let phis = self
                .module
                .block(to)
                .unwrap_or_default()
                .iter()
                .take_while(|instruction| matches!(self.module.node(**instruction), Some(IrNode::Phi { .. })))
                .copied()
                .collect::<Vec<_>>();
            for instruction in phis {
                self.queue_instruction(instruction);
            }
        }
    }

    fn is_executable(&self, block: IrNodeId) -> bool {
        self.executable_blocks.get(block.0 as usize).copied().unwrap_or(false)
    }

    fn queue_instruction(&mut self, instruction: IrNodeId) {
        let index = instruction.0 as usize;
        if self.queued_instructions.get(index) == Some(&false) {
            self.queued_instructions[index] = true;
            self.pending_instructions.push_back(instruction);
        }
    }
}

fn edge_key(from: IrNodeId, to: IrNodeId) -> u64 { (u64::from(from.0) << 32) | u64::from(to.0) }

struct CandidateUses {
    slots: Vec<u32>,
    candidates: Vec<IrNodeId>,
    users: Vec<Vec<IrNodeId>>,
}

impl CandidateUses {
    fn new(module: &Module) -> Self {
        let mut slots = vec![u32::MAX; module.nodes.len()];
        let mut candidates = Vec::new();
        let mut users = Vec::new();
        for (index, node) in module.nodes.iter().enumerate() {
            if matches!(node, IrNode::Phi { .. } | IrNode::Unary { .. } | IrNode::Binary { .. }) {
                slots[index] = users.len() as u32;
                candidates.push(IrNodeId(index as u32));
                users.push(Vec::new());
            }
        }
        Self {
            slots,
            candidates,
            users,
        }
    }

    fn add(&mut self, value: IrNodeId, user: IrNodeId) {
        let Some(slot) = self.slots.get(value.0 as usize).copied() else {
            return;
        };
        if slot != u32::MAX {
            self.users[slot as usize].push(user);
        }
    }

    fn get(&self, value: IrNodeId) -> &[IrNodeId] {
        let Some(slot) = self.slots.get(value.0 as usize).copied() else {
            return &[];
        };
        if slot == u32::MAX {
            &[]
        } else {
            &self.users[slot as usize]
        }
    }

    fn candidates(&self) -> &[IrNodeId] { &self.candidates }
}

enum SolverInstruction {
    Branch(IrNodeId),
    ConditionalBranch {
        condition: IrNodeId,
        true_block: IrNodeId,
        false_block: IrNodeId,
    },
    TryCatch {
        body: IrNodeId,
        catch: IrNodeId,
    },
    Phi(Vec<PhiOperand>),
    Unary {
        op: UnaryOp,
        operand: IrNodeId,
    },
    Binary {
        op: BinaryOp,
        lhs: IrNodeId,
        rhs: IrNodeId,
    },
    Other,
}

impl From<&IrNode> for SolverInstruction {
    fn from(node: &IrNode) -> Self {
        match node {
            IrNode::Branch(target) => Self::Branch(*target),
            IrNode::ConditionalBranch {
                condition,
                true_block,
                false_block,
            } => Self::ConditionalBranch {
                condition: *condition,
                true_block: *true_block,
                false_block: *false_block,
            },
            IrNode::TryCatch { body, catch, .. } => Self::TryCatch {
                body: *body,
                catch: *catch,
            },
            IrNode::Phi { operands } => Self::Phi(operands.clone()),
            IrNode::Unary { op, operand } => Self::Unary {
                op: *op,
                operand: *operand,
            },
            IrNode::Binary { op, lhs, rhs } => Self::Binary {
                op: *op,
                lhs: *lhs,
                rhs: *rhs,
            },
            _ => Self::Other,
        }
    }
}

fn is_solver_instruction(node: &IrNode) -> bool {
    matches!(
        node,
        IrNode::Branch(_)
            | IrNode::ConditionalBranch { .. }
            | IrNode::TryCatch { .. }
            | IrNode::Phi { .. }
            | IrNode::Unary { .. }
            | IrNode::Binary { .. }
    )
}

impl Analysis {
    fn constant<'a>(&'a self, module: &'a Module, value: ConstantRef) -> Option<&'a Value> {
        match value {
            ConstantRef::Module(id) => match module.node(id) {
                Some(IrNode::Constant(value)) => Some(value),
                _ => None,
            },
            ConstantRef::Folded(index) => self.folded_values.get(index),
        }
    }
}

fn rewrite_constant_branches(module: &mut Module, analysis: &Analysis) -> Vec<(IrNodeId, IrNodeId)> {
    let branches = analysis
        .executable_blocks
        .iter()
        .filter_map(|block| {
            module
                .block(*block)
                .and_then(<[IrNodeId]>::last)
                .copied()
                .map(|branch| (*block, branch))
        })
        .collect::<Vec<_>>();
    let mut removed_edges = Vec::new();

    for (predecessor, branch) in branches {
        let Some(IrNode::ConditionalBranch {
            condition,
            true_block,
            false_block,
        }) = module.node(branch).cloned()
        else {
            continue;
        };
        let Some(Lattice::Constant(value)) = analysis.values.get(condition.0 as usize) else {
            continue;
        };
        let Some(value) = analysis.constant(module, *value) else {
            continue;
        };
        let (selected, removed) = if value.is_truthy() {
            (true_block, false_block)
        } else {
            (false_block, true_block)
        };
        module.nodes[branch.0 as usize] = IrNode::Branch(selected);
        removed_edges.push((predecessor, removed));
    }

    removed_edges
}

fn scalar(value: &Value) -> Option<&Value> {
    matches!(
        value,
        Value::Null | Value::Num(_) | Value::Text(_) | Value::Resource(_) | Value::Path(_)
    )
    .then_some(value)
}

#[derive(Clone, PartialEq, Eq, Hash)]
enum ScalarKey {
    Null,
    Num(u32),
    Text(String),
    Resource(String),
    Path(TreePath),
}

impl ScalarKey {
    fn new(value: &Value) -> Option<Self> {
        Some(match value {
            Value::Null => Self::Null,
            Value::Num(value) => Self::Num(value.to_bits()),
            Value::Text(value) => Self::Text(value.clone()),
            Value::Resource(value) => Self::Resource(value.clone()),
            Value::Path(value) => Self::Path(value.clone()),
            Value::List(_) | Value::Unevaluated => return None,
        })
    }
}

fn scalar_constant_ids(module: &Module) -> HashMap<ScalarKey, IrNodeId> {
    module
        .constants
        .iter()
        .filter_map(|id| match module.node(*id) {
            Some(IrNode::Constant(value)) => ScalarKey::new(value).map(|key| (key, *id)),
            _ => None,
        })
        .collect::<HashMap<_, _>>()
}

fn remove_phi_edge(module: &mut Module, predecessor: IrNodeId, target: IrNodeId, affected: &mut Vec<IrNodeId>) {
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
        if let Some(position) = operands.iter().position(|operand| operand.block == predecessor) {
            operands.remove(position);
            affected.push(phi);
        }
    }
}

fn simplify_affected_phis(
    module: &mut Module, phis: Vec<IrNodeId>, analysis: &Analysis, constant_ids: &mut HashMap<ScalarKey, IrNodeId>,
) {
    let mut replacements = HashMap::<IrNodeId, IrNodeId>::new();
    let mut pending = VecDeque::from(phis);
    let mut queued = pending.iter().copied().collect::<HashSet<_>>();

    'pending: while let Some(phi) = pending.pop_front() {
        queued.remove(&phi);
        if replacements.contains_key(&phi) {
            continue;
        }
        let Some(IrNode::Phi { operands }) = module.node(phi) else {
            continue;
        };

        let mut same = None;
        for operand in operands {
            let value = resolve_replacement(&replacements, operand.value);
            if value == phi || same == Some(value) {
                continue;
            }
            if same.is_some() {
                continue 'pending;
            }
            same = Some(value);
        }

        let replacement = match same {
            Some(value) => resolve_replacement(&replacements, value),
            None => intern_constant(module, constant_ids, Value::Null),
        };
        replacements.insert(phi, replacement);

        for user in analysis.uses.get(phi) {
            if matches!(module.node(*user), Some(IrNode::Phi { .. })) && queued.insert(*user) {
                pending.push_back(*user);
            }
        }
    }

    if replacements.is_empty() {
        return;
    }

    for replaced in replacements.keys() {
        for user in analysis.uses.get(*replaced) {
            if let Some(node) = module.nodes.get_mut(user.0 as usize) {
                replace_operands(node, &replacements);
            }
        }
    }
    replace_metadata_uses(module, &replacements);

    let affected_blocks = replacements
        .keys()
        .filter_map(|instruction| analysis.instruction_blocks.get(instruction.0 as usize))
        .copied()
        .filter(|block| block.is_valid())
        .collect::<HashSet<_>>();
    for block in affected_blocks {
        if let Some(IrNode::Label(instructions)) = module.nodes.get_mut(block.0 as usize) {
            instructions.retain(|instruction| !replacements.contains_key(instruction));
        }
    }
    for phi in replacements.keys() {
        module.nodes[phi.0 as usize] = IrNode::Noop;
    }
}

fn resolve_replacement(replacements: &HashMap<IrNodeId, IrNodeId>, mut value: IrNodeId) -> IrNodeId {
    while let Some(replacement) = replacements.get(&value) {
        value = *replacement;
    }
    value
}

fn number(value: &Value) -> Option<f32> {
    match value {
        Value::Null => Some(0.0),
        Value::Num(value) => Some(*value),
        _ => None,
    }
}

fn boolean(value: bool) -> Value { Value::Num(if value { 1.0 } else { 0.0 }) }

fn fold_unary(op: UnaryOp, operand: &Value) -> Option<Value> {
    match op {
        UnaryOp::Neg => Some(Value::Num(-number(operand)?)),
        UnaryOp::Not => Some(boolean(!operand.is_truthy())),
        UnaryOp::BitNot => Some(Value::Num(((!(number(operand)? as u32)) & 0x00ff_ffff) as f32)),
        UnaryOp::PreIncrement
        | UnaryOp::PreDecrement
        | UnaryOp::PostIncrement
        | UnaryOp::PostDecrement
        | UnaryOp::Reference
        | UnaryOp::Dereference => None,
    }
}

fn fold_binary(op: BinaryOp, lhs: &Value, rhs: &Value) -> Option<Value> {
    use BinaryOp::*;

    match op {
        CompEq | CompEquiv => return Some(boolean(equal(lhs, rhs))),
        CompNotEq | CompNotEquiv => return Some(boolean(!equal(lhs, rhs))),
        LogicalAnd => return Some(if lhs.is_truthy() { rhs.clone() } else { lhs.clone() }),
        LogicalOr => return Some(if lhs.is_truthy() { lhs.clone() } else { rhs.clone() }),
        In => return None,
        // Runtime text concatenation observes the evaluator's text allocation limit.
        Add if matches!((lhs, rhs), (Value::Text(_), Value::Text(_))) => return None,
        _ => {},
    }

    if matches!(lhs, Value::Null) && op == Add && number(rhs).is_none() {
        return Some(rhs.clone());
    }

    if let (Some(lhs), Some(rhs)) = (lhs.as_text(), rhs.as_text()) {
        return Some(boolean(match op {
            CompLess => lhs < rhs,
            CompLessEq => lhs <= rhs,
            CompGreater => lhs > rhs,
            CompGreaterEq => lhs >= rhs,
            _ => return None,
        }));
    }

    let lhs = number(lhs)?;
    let rhs = number(rhs)?;
    if matches!(op, Div | Mod | FloatMod) && rhs == 0.0 {
        return None;
    }

    Some(match op {
        Add => Value::Num(lhs + rhs),
        Sub => Value::Num(lhs - rhs),
        Mul => Value::Num(lhs * rhs),
        Div => Value::Num(lhs / rhs),
        Mod => {
            if rhs.trunc() == 0.0 {
                return None;
            }
            Value::Num(lhs.trunc() % rhs.trunc())
        },
        FloatMod => Value::Num(lhs % rhs),
        Pow => Value::Num(lhs.powf(rhs)),
        BitAnd => Value::Num(((lhs as u32) & (rhs as u32)) as f32),
        BitOr => Value::Num(((lhs as u32) | (rhs as u32)) as f32),
        BitXor => Value::Num(((lhs as u32) ^ (rhs as u32)) as f32),
        ShiftLeft => Value::Num(((lhs as u32).checked_shl(rhs as u32).unwrap_or(0) & 0x00ff_ffff) as f32),
        ShiftRight => Value::Num((lhs as u32).checked_shr(rhs as u32).unwrap_or(0) as f32),
        CompLess => boolean(lhs < rhs),
        CompLessEq => boolean(lhs <= rhs),
        CompGreater => boolean(lhs > rhs),
        CompGreaterEq => boolean(lhs >= rhs),
        CompThreeWay => Value::Num(if lhs < rhs {
            -1.0
        } else if lhs > rhs {
            1.0
        } else {
            0.0
        }),
        CompEq | CompNotEq | CompEquiv | CompNotEquiv | LogicalAnd | LogicalOr | In => unreachable!(),
    })
}

fn equal(lhs: &Value, rhs: &Value) -> bool {
    match (lhs, rhs) {
        (Value::Null, Value::Null) => true,
        (Value::Num(lhs), Value::Num(rhs)) => lhs == rhs,
        (Value::Text(lhs), Value::Text(rhs)) | (Value::Resource(lhs), Value::Resource(rhs)) => lhs == rhs,
        (Value::Path(lhs), Value::Path(rhs)) => {
            lhs.segments == rhs.segments
                && lhs.flags.intersects(PathFlags::IS_PROC | PathFlags::IS_VERB)
                    == rhs.flags.intersects(PathFlags::IS_PROC | PathFlags::IS_VERB)
        },
        _ => false,
    }
}

fn intern_constant(module: &mut Module, constant_ids: &mut HashMap<ScalarKey, IrNodeId>, value: Value) -> IrNodeId {
    let key = ScalarKey::new(&value).expect("SCCP only folds scalar values");
    if let Some(id) = constant_ids.get(&key) {
        return *id;
    }

    let id = IrNodeId(module.nodes.len() as u32);
    module.nodes.push(IrNode::Constant(value));
    module.constants.push(id);
    constant_ids.insert(key, id);
    id
}

#[cfg(test)]
mod tests {
    use core::{location::Location, path::TreePath, types::Value};

    use super::{fold_binary, fold_unary};
    use crate::{BinaryOp, IrModuleBuilder, IrNode, Module, UnaryOp};

    fn lower(source: &str) -> Module {
        let (tokens, _) = lexer::tokenize(source);
        let ast = ast::parse(&tokens).expect("fixture should parse");
        let mut builder = IrModuleBuilder::new(&ast);
        for declaration in &ast.declarations {
            let ast::Declaration::Proc { path, params, body, .. } = declaration else {
                continue;
            };
            builder.lower_proc(
                TreePath::default(),
                path.name().cloned().unwrap_or_else(|| "anonymous".into()),
                params,
                false,
                body.as_deref().unwrap_or_default(),
                Location::default(),
            );
        }
        builder.finish()
    }

    fn returned(module: &Module) -> Option<&Value> {
        module.nodes.iter().find_map(|node| {
            let IrNode::Return(Some(value)) = node else {
                return None;
            };
            let Some(IrNode::Constant(value)) = module.node(*value) else {
                return None;
            };
            Some(value)
        })
    }

    #[test]
    fn folds_scalar_expression_chains() {
        let module = lower("/proc/t()\n\treturn -(1 + 2) * 4\n");

        assert_eq!(returned(&module), Some(&Value::Num(-12.0)));
        assert!(
            !module
                .nodes
                .iter()
                .any(|node| matches!(node, IrNode::Unary { .. } | IrNode::Binary { .. }))
        );
        crate::verify(&module).expect("folded module should verify");
    }

    #[test]
    fn follows_only_the_executable_phi_edge() {
        let module = lower("/proc/t()\n\tvar/x\n\tif(1)\n\t\tx = 2 + 3\n\telse\n\t\tx = 9\n\treturn x\n");

        assert_eq!(returned(&module), Some(&Value::Num(5.0)));
        assert!(
            !module
                .nodes
                .iter()
                .any(|node| matches!(node, IrNode::ConditionalBranch { .. }))
        );
        crate::verify(&module).expect("constant branch module should verify");
    }

    #[test]
    fn removes_a_faulting_rhs_when_short_circuiting_skips_it() {
        let module = lower("/proc/t()\n\treturn 0 && (1 / 0)\n");

        assert_eq!(returned(&module), Some(&Value::Num(0.0)));
        assert!(
            module
                .nodes
                .iter()
                .all(|node| !matches!(node, IrNode::Binary { op: BinaryOp::Div, .. }))
        );
        crate::verify(&module).expect("short-circuit module should verify");
    }

    #[test]
    fn preserves_a_faulting_rhs_when_its_path_executes() {
        let module = lower("/proc/t()\n\treturn 1 && (1 / 0)\n");

        assert!(
            module
                .nodes
                .iter()
                .any(|node| matches!(node, IrNode::Binary { op: BinaryOp::Div, .. }))
        );
        crate::verify(&module).expect("executable faulting expression should verify");
    }

    #[test]
    fn leaves_faulting_and_allocating_operations_for_the_vm() {
        assert_eq!(fold_binary(BinaryOp::Div, &Value::Num(1.0), &Value::Num(0.0)), None);
        assert_eq!(
            fold_binary(BinaryOp::Add, &Value::Text("a".into()), &Value::Text("b".into())),
            None
        );
        assert_eq!(fold_unary(UnaryOp::Reference, &Value::Num(1.0)), None);
    }

    #[test]
    fn uses_dm_null_and_bitwise_rules() {
        assert_eq!(fold_unary(UnaryOp::Neg, &Value::Null), Some(Value::Num(-0.0)));
        assert_eq!(
            fold_unary(UnaryOp::BitNot, &Value::Num(0.0)),
            Some(Value::Num(16_777_215.0))
        );
        assert_eq!(
            fold_binary(BinaryOp::Add, &Value::Null, &Value::Text("value".into())),
            Some(Value::Text("value".into()))
        );
        assert_eq!(
            fold_binary(BinaryOp::CompEq, &Value::Null, &Value::Num(0.0)),
            Some(Value::Num(0.0))
        );
    }
}
