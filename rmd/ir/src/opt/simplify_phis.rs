use core::types::{IrNodeId, Value};
use std::collections::{HashMap, HashSet, VecDeque};

use crate::{Argument, IrNode, Module, OutputTarget};

pub fn simplify_phis(module: &mut Module) {
    let phis = module
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(index, node)| matches!(node, IrNode::Phi { .. }).then_some(IrNodeId(index as u32)))
        .collect::<Vec<_>>();

    let mut users = HashMap::<IrNodeId, HashSet<IrNodeId>>::new();
    for phi in &phis {
        let Some(IrNode::Phi { operands }) = module.node(*phi) else {
            continue;
        };

        for operand in operands {
            users.entry(operand.value).or_default().insert(*phi);
        }
    }

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
        let values = operands.iter().map(|operand| operand.value).collect::<Vec<_>>();
        let mut same = None;
        for value in values {
            let value = resolve(&replacements, value);
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

        for user in users.remove(&phi).unwrap_or_default() {
            if user == phi || replacements.contains_key(&user) {
                continue;
            }

            users.entry(replacement).or_default().insert(user);
            if queued.insert(user) {
                pending.push_back(user);
            }
        }
    }

    if replacements.is_empty() {
        return;
    }

    for node in &mut module.nodes {
        replace_operands(node, &replacements);
    }

    for proc in &mut module.procs {
        for param in &mut proc.params {
            replace_optional(&mut param.default, &replacements);
            replace_optional(&mut param.in_list, &replacements);
            for dimension in &mut param.spec.dimensions {
                replace_optional(dimension, &replacements);
            }
        }

        for var in &mut proc.vars {
            for dimension in &mut var.dimensions {
                replace_optional(dimension, &replacements);
            }
        }
    }

    for node in &mut module.nodes {
        if let IrNode::Label(instructions) = node {
            instructions.retain(|instruction| !replacements.contains_key(instruction));
        }
    }

    for phi in replacements.keys() {
        module.nodes[phi.0 as usize] = IrNode::Noop;
    }
}

fn resolve(replacements: &HashMap<IrNodeId, IrNodeId>, mut value: IrNodeId) -> IrNodeId {
    while let Some(replacement) = replacements.get(&value) {
        value = *replacement;
    }

    value
}

fn intern_null(module: &mut Module) -> IrNodeId {
    if let Some(id) = module
        .constants
        .iter()
        .copied()
        .find(|id| matches!(module.node(*id), Some(IrNode::Constant(Value::Null))))
    {
        return id;
    }

    let id = IrNodeId(module.nodes.len() as u32);
    module.nodes.push(IrNode::Constant(Value::Null));
    module.constants.push(id);

    id
}

fn replace_id(id: &mut IrNodeId, replacements: &HashMap<IrNodeId, IrNodeId>) { *id = resolve(replacements, *id); }

fn replace_optional(id: &mut Option<IrNodeId>, replacements: &HashMap<IrNodeId, IrNodeId>) {
    if let Some(id) = id {
        replace_id(id, replacements);
    }
}

fn replace_arguments(args: &mut [Argument], replacements: &HashMap<IrNodeId, IrNodeId>) {
    for arg in args {
        replace_optional(&mut arg.key, replacements);
        replace_optional(&mut arg.value, replacements);
    }
}

fn replace_operands(node: &mut IrNode, replacements: &HashMap<IrNodeId, IrNodeId>) {
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
        IrNode::Interpolate { values, .. } => {
            for value in values {
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
