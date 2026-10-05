use core::types::{IrNodeId, Value};
use std::collections::VecDeque;

use rustc_hash::FxHashMap;

use super::Adjacency;
use crate::{Argument, IrNode, Module, OutputTarget};

pub fn simplify_phis(module: &mut Module) {
    let phis = module
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(index, node)| matches!(node, IrNode::Phi { .. }).then_some(IrNodeId(index as u32)))
        .collect::<Vec<_>>();

    let users = Adjacency::new(module.nodes.len(), |edge| {
        for phi in &phis {
            if let Some(IrNode::Phi { operands }) = module.node(*phi) {
                for operand in operands {
                    edge(operand.value.0, *phi);
                }
            }
        }
    });
    let mut transferred = FxHashMap::<IrNodeId, Vec<IrNodeId>>::default();
    let mut replacements = Replacements::new(module.nodes.len());
    let mut queued = vec![false; module.nodes.len()];
    for phi in &phis {
        queued[phi.0 as usize] = true;
    }

    let mut pending = VecDeque::from(phis);

    'pending: while let Some(phi) = pending.pop_front() {
        queued[phi.0 as usize] = false;
        if replacements.contains(phi) {
            continue;
        }

        let Some(IrNode::Phi { operands }) = module.node(phi) else {
            continue;
        };
        let mut same = None;
        for operand in operands {
            let value = resolve(&replacements, operand.value);
            if value == phi || same == Some(value) {
                continue;
            }
            if same.is_some() {
                continue 'pending;
            }
            same = Some(value);
        }

        let replacement = match same {
            Some(value) => value,
            None => intern_null(module),
        };
        let replacement = resolve(&replacements, replacement);
        replacements.insert(phi, replacement);

        let moved = transferred.remove(&phi).unwrap_or_default();
        for user in users.of(phi.0).iter().chain(&moved).copied() {
            if user == phi || replacements.contains(user) {
                continue;
            }

            transferred.entry(replacement).or_default().push(user);
            if !queued[user.0 as usize] {
                queued[user.0 as usize] = true;
                pending.push_back(user);
            }
        }
    }

    if replacements.is_empty() {
        return;
    }

    replace_all_uses(module, &replacements);

    for node in &mut module.nodes {
        if let IrNode::Label(instructions) = node {
            instructions.retain(|instruction| !replacements.contains(*instruction));
        }
    }

    for phi in replacements.keys() {
        module.nodes[phi.0 as usize] = IrNode::Noop;
    }
}

pub(super) trait Replace {
    fn replacement(&self, id: IrNodeId) -> Option<IrNodeId>;
}

impl Replace for FxHashMap<IrNodeId, IrNodeId> {
    fn replacement(&self, id: IrNodeId) -> Option<IrNodeId> { self.get(&id).copied() }
}

pub(super) struct Replacements {
    targets: Vec<IrNodeId>,
    replaced: Vec<IrNodeId>,
}

impl Replacements {
    pub(super) fn new(node_count: usize) -> Self {
        Self {
            targets: vec![IrNodeId::INVALID; node_count],
            replaced: Vec::new(),
        }
    }

    pub(super) fn insert(&mut self, id: IrNodeId, replacement: IrNodeId) {
        let index = id.0 as usize;
        if self.targets[index].is_invalid() {
            self.replaced.push(id);
        }

        self.targets[index] = replacement;
    }

    pub(super) fn contains(&self, id: IrNodeId) -> bool { self.replacement(id).is_some() }

    pub(super) fn is_empty(&self) -> bool { self.replaced.is_empty() }

    pub(super) fn keys(&self) -> impl Iterator<Item = IrNodeId> + '_ { self.replaced.iter().copied() }
}

impl Replace for Replacements {
    fn replacement(&self, id: IrNodeId) -> Option<IrNodeId> {
        self.targets
            .get(id.0 as usize)
            .copied()
            .filter(|target| target.is_valid())
    }
}

pub(super) fn resolve(replacements: &impl Replace, mut value: IrNodeId) -> IrNodeId {
    while let Some(replacement) = replacements.replacement(value) {
        value = replacement;
    }

    value
}

fn intern_null(module: &mut Module) -> IrNodeId {
    if let Some(id) = module
        .constants
        .iter()
        .copied()
        .find(|id| matches!(module.node(*id), Some(IrNode::Constant(value)) if matches!(**value, Value::Null)))
    {
        return id;
    }

    let id = IrNodeId(module.nodes.len() as u32);
    module.nodes.push(IrNode::Constant(Box::new(Value::Null)));
    module.constants.push(id);

    id
}

fn replace_id(id: &mut IrNodeId, replacements: &impl Replace) { *id = resolve(replacements, *id); }

fn replace_optional(id: &mut Option<IrNodeId>, replacements: &impl Replace) {
    if let Some(id) = id {
        replace_id(id, replacements);
    }
}

pub(super) fn replace_all_uses(module: &mut Module, replacements: &impl Replace) {
    for node in &mut module.nodes {
        replace_operands(node, replacements);
    }

    replace_metadata_uses(module, replacements);
}

pub(super) fn replace_metadata_uses(module: &mut Module, replacements: &impl Replace) {
    for proc in &mut module.procs {
        for param in &mut proc.params {
            replace_optional(&mut param.default, replacements);
            replace_optional(&mut param.in_list, replacements);
            for dimension in &mut param.spec.dimensions {
                replace_optional(dimension, replacements);
            }
        }

        for var in &mut proc.vars {
            for dimension in &mut var.dimensions {
                replace_optional(dimension, replacements);
            }
        }
    }
}

fn replace_arguments(args: &mut [Argument], replacements: &impl Replace) {
    for arg in args {
        replace_optional(&mut arg.key, replacements);
        replace_optional(&mut arg.value, replacements);
    }
}

pub(super) fn replace_operands(node: &mut IrNode, replacements: &impl Replace) {
    match node {
        IrNode::Phi { operands } => {
            for operand in operands {
                replace_id(&mut operand.value, replacements);
            }
        },
        IrNode::Unary { operand, .. } => replace_id(operand, replacements),
        IrNode::Binary { lhs, rhs, .. } | IrNode::CompoundBinary { lhs, rhs, .. } => {
            replace_id(lhs, replacements);
            replace_id(rhs, replacements);
        },
        IrNode::Load { pointer } => replace_id(pointer, replacements),
        IrNode::AccessField { object, .. } => replace_id(object, replacements),
        IrNode::Initial { object, .. } => replace_optional(object, replacements),
        IrNode::Index { object, index, .. } => {
            replace_id(object, replacements);
            replace_id(index, replacements);
        },
        IrNode::Call { callee, args } => {
            replace_id(callee, replacements);
            replace_arguments(args, replacements);
        },
        IrNode::FunctionCall { function, args } => {
            replace_id(function, replacements);
            replace_arguments(args, replacements);
        },
        IrNode::Super { args, .. } | IrNode::List(args) => replace_arguments(args, replacements),
        IrNode::New { ty, args } => {
            replace_optional(ty, replacements);
            replace_arguments(args, replacements);
        },
        IrNode::ModifiedType { overrides, .. } => {
            for (_, value) in overrides {
                replace_id(value, replacements);
            }
        },
        IrNode::Pick(choices) => {
            for (weight, value) in choices {
                replace_optional(weight, replacements);
                replace_id(value, replacements);
            }
        },
        IrNode::Interpolate(interpolation) => {
            for value in &mut interpolation.values {
                replace_id(value, replacements);
            }
        },
        IrNode::InRange {
            value,
            start,
            end,
            step,
        } => {
            replace_id(value, replacements);
            replace_id(start, replacements);
            replace_id(end, replacements);
            replace_optional(step, replacements);
        },
        IrNode::Range { start, end, step } => {
            replace_id(start, replacements);
            replace_id(end, replacements);
            replace_optional(step, replacements);
        },
        IrNode::RangeTest { current, end, step } => {
            replace_id(current, replacements);
            replace_id(end, replacements);
            replace_id(step, replacements);
        },
        IrNode::IterInit { list, .. } => replace_id(list, replacements),
        IrNode::IterNext(iter) | IrNode::IterValue(iter) | IrNode::IterKey(iter) => {
            replace_id(iter, replacements);
        },
        IrNode::SetField { object, value, .. } => {
            replace_id(object, replacements);
            replace_id(value, replacements);
        },
        IrNode::SetIndex {
            object, index, value, ..
        } => {
            replace_id(object, replacements);
            replace_id(index, replacements);
            replace_id(value, replacements);
        },
        IrNode::Store { pointer, value } | IrNode::Initialize { pointer, value } => {
            replace_id(pointer, replacements);
            replace_id(value, replacements);
        },
        IrNode::StoreBuiltin { value, .. } => replace_id(value, replacements),
        IrNode::ConditionalBranch { condition, .. } => replace_id(condition, replacements),
        IrNode::Return(value) => replace_optional(value, replacements),
        IrNode::Output { target, value } => {
            match target {
                OutputTarget::Value(target) => replace_id(target, replacements),
                OutputTarget::Field { object, .. } => replace_id(object, replacements),
                OutputTarget::Index { object, index, .. } => {
                    replace_id(object, replacements);
                    replace_id(index, replacements);
                },
            }
            replace_id(value, replacements);
        },
        IrNode::Del(value) | IrNode::Throw(value) => replace_id(value, replacements),
        _ => {},
    }
}
