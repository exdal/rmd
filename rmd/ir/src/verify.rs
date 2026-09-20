use core::types::IrNodeId;
use std::collections::{HashMap, HashSet};

use crate::{IrNode, Module};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyError {
    message: String,
}

impl VerifyError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for VerifyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { formatter.write_str(&self.message) }
}

impl std::error::Error for VerifyError {}

pub fn verify(module: &Module) -> Result<(), VerifyError> {
    let blocks = module
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(index, node)| matches!(node, IrNode::Label(_)).then_some(IrNodeId(index as u32)))
        .collect::<HashSet<_>>();
    let mut instruction_blocks = HashMap::<IrNodeId, IrNodeId>::new();
    let mut predecessors = HashMap::<IrNodeId, Vec<IrNodeId>>::new();

    for (index, proc) in module.procs.iter().enumerate() {
        if !matches!(module.node(proc.function), Some(IrNode::Function(id)) if id.0 as usize == index) {
            return Err(VerifyError::new(format!(
                "procedure {index} has invalid function node {}",
                proc.function
            )));
        }
        require_block(&blocks, proc.body, &format!("procedure {index} body"))?;
        if let Some(previous) = proc.previous
            && module.proc(previous).is_none()
        {
            return Err(VerifyError::new(format!(
                "procedure {index} references missing previous procedure {previous}"
            )));
        }
        for parameter in &proc.parameters {
            if !matches!(module.node(*parameter), Some(IrNode::FunctionParameter(_))) {
                return Err(VerifyError::new(format!(
                    "procedure {index} has invalid parameter node {parameter}"
                )));
            }
        }
        for parameter in &proc.params {
            verify_optional(
                module,
                parameter.default,
                &format!("procedure {index} parameter default"),
            )?;
            verify_optional(
                module,
                parameter.in_list,
                &format!("procedure {index} parameter in-list"),
            )?;
            for dimension in &parameter.spec.dimensions {
                verify_optional(module, *dimension, &format!("procedure {index} parameter dimension"))?;
            }
        }
        for variable in &proc.vars {
            for dimension in &variable.dimensions {
                verify_optional(module, *dimension, &format!("procedure {index} variable dimension"))?;
            }
        }
    }

    for constant in &module.constants {
        if !matches!(module.node(*constant), Some(IrNode::Constant(_))) {
            return Err(VerifyError::new(format!(
                "constant table contains invalid node {constant}"
            )));
        }
    }
    for function in &module.external_functions {
        if !matches!(module.node(*function), Some(IrNode::ExternalFunction(_))) {
            return Err(VerifyError::new(format!(
                "external function table contains invalid node {function}"
            )));
        }
    }

    for block in &blocks {
        let instructions = module.block(*block).expect("block set was built from labels");
        if instructions.is_empty() {
            return Err(VerifyError::new(format!("block {block} is empty")));
        }

        let terminators = instructions
            .iter()
            .enumerate()
            .filter(|(_, instruction)| module.node(**instruction).is_some_and(IrNode::is_terminator))
            .collect::<Vec<_>>();
        if terminators.len() != 1 {
            return Err(VerifyError::new(format!(
                "block {block} has {} terminators",
                terminators.len()
            )));
        }
        if terminators[0].0 + 1 != instructions.len() {
            return Err(VerifyError::new(format!(
                "block {block} has instructions after its terminator"
            )));
        }

        let mut saw_non_phi = false;
        for (position, instruction) in instructions.iter().copied().enumerate() {
            let node = module
                .node(instruction)
                .ok_or_else(|| VerifyError::new(format!("block {block} contains missing node {instruction}")))?;
            if matches!(node, IrNode::Noop) {
                return Err(VerifyError::new(format!(
                    "block {block} contains removed node {instruction}"
                )));
            }
            if let Some(owner) = instruction_blocks.insert(instruction, *block) {
                return Err(VerifyError::new(format!(
                    "instruction {instruction} belongs to both block {owner} and block {block}"
                )));
            }

            if matches!(node, IrNode::Phi { .. }) {
                if saw_non_phi {
                    return Err(VerifyError::new(format!(
                        "block {block} has phi {instruction} after another instruction"
                    )));
                }
            } else {
                saw_non_phi = true;
            }

            if node.is_merge() {
                if position + 2 != instructions.len() {
                    return Err(VerifyError::new(format!(
                        "block {block} has misplaced merge {instruction}"
                    )));
                }
                let terminator = module.node(*instructions.last().expect("nonempty block"));
                if !matches!(terminator, Some(IrNode::Branch(_) | IrNode::ConditionalBranch { .. })) {
                    return Err(VerifyError::new(format!(
                        "block {block} merge {instruction} is not followed by a branch"
                    )));
                }
            }

            for operand in node.operands() {
                verify_reference(module, operand, &format!("instruction {instruction}"))?;
            }
            verify_targets(node, &blocks, instruction)?;
        }

        let terminator = instructions.last().and_then(|instruction| module.node(*instruction));
        match terminator {
            Some(IrNode::Branch(target)) => predecessors.entry(*target).or_default().push(*block),
            Some(IrNode::ConditionalBranch {
                true_block,
                false_block,
                ..
            }) => {
                predecessors.entry(*true_block).or_default().push(*block);
                predecessors.entry(*false_block).or_default().push(*block);
            },
            _ => {},
        }
    }

    for block in &blocks {
        let mut expected = predecessors.get(block).cloned().unwrap_or_default();
        expected.sort_by_key(|id| id.0);
        for instruction in module.block(*block).unwrap_or_default() {
            let Some(IrNode::Phi { operands }) = module.node(*instruction) else {
                break;
            };
            let mut actual = operands.iter().map(|operand| operand.block).collect::<Vec<_>>();
            actual.sort_by_key(|id| id.0);
            if actual != expected {
                return Err(VerifyError::new(format!(
                    "phi {instruction} in block {block} has predecessors {actual:?}, expected {expected:?}"
                )));
            }
        }
    }

    Ok(())
}

fn verify_optional(module: &Module, node: Option<IrNodeId>, context: &str) -> Result<(), VerifyError> {
    if let Some(node) = node {
        verify_reference(module, node, context)?;
    }

    Ok(())
}

fn verify_reference(module: &Module, node: IrNodeId, context: &str) -> Result<(), VerifyError> {
    match module.node(node) {
        Some(IrNode::Noop) => Err(VerifyError::new(format!("{context} references removed node {node}"))),
        Some(_) => Ok(()),
        None => Err(VerifyError::new(format!("{context} references missing node {node}"))),
    }
}

fn require_block(blocks: &HashSet<IrNodeId>, block: IrNodeId, context: &str) -> Result<(), VerifyError> {
    if blocks.contains(&block) {
        Ok(())
    } else {
        Err(VerifyError::new(format!("{context} references non-block node {block}")))
    }
}

fn verify_targets(node: &IrNode, blocks: &HashSet<IrNodeId>, instruction: IrNodeId) -> Result<(), VerifyError> {
    let targets = match node {
        IrNode::SelectionMerge { merge_block } => vec![*merge_block],
        IrNode::LoopMerge {
            merge_block,
            continue_block,
        } => vec![*merge_block, *continue_block],
        IrNode::Branch(target) => vec![*target],
        IrNode::ConditionalBranch {
            true_block,
            false_block,
            ..
        } => vec![*true_block, *false_block],
        IrNode::TryCatch { body, catch, merge } => vec![*body, *catch, *merge],
        IrNode::Phi { operands } => operands.iter().map(|operand| operand.block).collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    for target in targets {
        require_block(blocks, target, &format!("instruction {instruction}"))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use core::types::IrNodeId;

    use super::verify;
    use crate::{IrNode, Module, PhiOperand};

    #[test]
    fn rejects_instructions_after_a_terminator() {
        let module = Module {
            nodes: vec![
                IrNode::Label(vec![IrNodeId(1), IrNodeId(2)]),
                IrNode::Return(None),
                IrNode::Return(None),
            ],
            ..Module::default()
        };

        let error = verify(&module).expect_err("two terminators should be invalid");

        assert!(error.to_string().contains("2 terminators"));
    }

    #[test]
    fn rejects_a_phi_without_every_predecessor() {
        let module = Module {
            nodes: vec![
                IrNode::Label(vec![IrNodeId(1)]),
                IrNode::Branch(IrNodeId(2)),
                IrNode::Label(vec![IrNodeId(3), IrNodeId(4)]),
                IrNode::Phi {
                    operands: Vec::<PhiOperand>::new(),
                },
                IrNode::Return(Some(IrNodeId(3))),
            ],
            ..Module::default()
        };

        let error = verify(&module).expect_err("phi should name its predecessor");

        assert!(error.to_string().contains("expected [%0]"));
    }
}
