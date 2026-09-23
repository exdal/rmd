use core::types::{IrNodeId, Value};
use std::collections::VecDeque;

use crate::{BinaryOp, IrNode, Module, UnaryOp};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ValueKinds(u8);

impl ValueKinds {
    const ANY: Self = Self(Self::BOOLEAN.0 | Self::U24.0 | Self::NUMBER.0 | Self::NON_NUMBER.0);
    const ANY_NUMBER: Self = Self(Self::BOOLEAN.0 | Self::U24.0 | Self::NUMBER.0);
    const ANY_U24: Self = Self(Self::BOOLEAN.0 | Self::U24.0);
    const BOOLEAN: Self = Self(1 << 0);
    const NON_NUMBER: Self = Self(1 << 3);
    const NUMBER: Self = Self(1 << 2);
    const U24: Self = Self(1 << 1);

    fn union(self, other: Self) -> Self { Self(self.0 | other.0) }

    fn is_empty(self) -> bool { self.0 == 0 }

    fn is_subset_of(self, other: Self) -> bool { !self.is_empty() && self.0 & !other.0 == 0 }
}

pub(super) struct ValueFacts {
    kinds: Vec<ValueKinds>,
}

impl ValueFacts {
    pub(super) fn analyze(module: &Module) -> Self {
        let mut kinds = vec![ValueKinds::default(); module.nodes.len()];
        let mut users = vec![Vec::new(); module.nodes.len()];

        for (index, node) in module.nodes.iter().enumerate() {
            let user = IrNodeId(index as u32);
            node.for_each_operand(|operand| {
                if let Some(users) = users.get_mut(operand.0 as usize) {
                    users.push(user);
                }
            });
        }

        let mut pending = (0..module.nodes.len())
            .map(|index| IrNodeId(index as u32))
            .collect::<VecDeque<_>>();
        let mut queued = vec![true; module.nodes.len()];

        while let Some(id) = pending.pop_front() {
            let index = id.0 as usize;
            queued[index] = false;
            let next = module.node(id).map(|node| transfer(node, &kinds)).unwrap_or_default();
            let joined = kinds[index].union(next);
            if joined == kinds[index] {
                continue;
            }

            kinds[index] = joined;
            for user in &users[index] {
                let user = user.0 as usize;
                if !queued[user] {
                    queued[user] = true;
                    pending.push_back(IrNodeId(user as u32));
                }
            }
        }

        Self { kinds }
    }

    pub(super) fn is_number(&self, id: IrNodeId) -> bool { self.kind(id).is_subset_of(ValueKinds::ANY_NUMBER) }

    pub(super) fn is_u24(&self, id: IrNodeId) -> bool { self.kind(id).is_subset_of(ValueKinds::ANY_U24) }

    pub(super) fn is_boolean(&self, id: IrNodeId) -> bool { self.kind(id).is_subset_of(ValueKinds::BOOLEAN) }

    fn kind(&self, id: IrNodeId) -> ValueKinds { self.kinds.get(id.0 as usize).copied().unwrap_or(ValueKinds::ANY) }
}

fn transfer(node: &IrNode, kinds: &[ValueKinds]) -> ValueKinds {
    match node {
        IrNode::Constant(value) => constant_kind(value),
        IrNode::Phi { operands } => operands
            .iter()
            .map(|operand| kind(kinds, operand.value))
            .fold(ValueKinds::default(), ValueKinds::union),
        IrNode::Unary { op, .. } => match op {
            UnaryOp::Not => ValueKinds::BOOLEAN,
            UnaryOp::BitNot => ValueKinds::ANY_U24,
            UnaryOp::Neg
            | UnaryOp::PreIncrement
            | UnaryOp::PreDecrement
            | UnaryOp::PostIncrement
            | UnaryOp::PostDecrement => ValueKinds::ANY_NUMBER,
            UnaryOp::Reference | UnaryOp::Dereference => ValueKinds::ANY,
        },
        IrNode::Binary { op, lhs, rhs } | IrNode::CompoundBinary { op, lhs, rhs } => {
            binary_kind(*op, kind(kinds, *lhs), kind(kinds, *rhs))
        },
        IrNode::InRange { .. } | IrNode::IterNext(_) | IrNode::RangeTest { .. } => ValueKinds::BOOLEAN,
        IrNode::Noop => ValueKinds::default(),
        _ => ValueKinds::ANY,
    }
}

fn constant_kind(value: &Value) -> ValueKinds {
    let Value::Num(value) = value else {
        return ValueKinds::NON_NUMBER;
    };

    if matches!(value.to_bits(), 0x0000_0000 | 0x3f80_0000) {
        ValueKinds::BOOLEAN
    } else if !value.is_sign_negative()
        && value.is_finite()
        && value.fract() == 0.0
        && (0.0..=16_777_215.0).contains(value)
    {
        ValueKinds::U24
    } else {
        ValueKinds::NUMBER
    }
}

fn binary_kind(op: BinaryOp, lhs: ValueKinds, rhs: ValueKinds) -> ValueKinds {
    use BinaryOp::*;

    match op {
        CompEq | CompNotEq | CompEquiv | CompNotEquiv | CompGreater | CompLess | CompGreaterEq | CompLessEq | In => {
            ValueKinds::BOOLEAN
        },
        CompThreeWay | Mul | Div | Mod | FloatMod | Pow | ShiftRight => ValueKinds::ANY_NUMBER,
        ShiftLeft => ValueKinds::ANY_U24,
        LogicalAnd | LogicalOr => lhs.union(rhs),
        Add | Sub | BitAnd | BitXor | BitOr => {
            if lhs.is_empty() {
                ValueKinds::default()
            } else if lhs.is_subset_of(ValueKinds::ANY_NUMBER) {
                ValueKinds::ANY_NUMBER
            } else {
                ValueKinds::ANY
            }
        },
    }
}

fn kind(kinds: &[ValueKinds], id: IrNodeId) -> ValueKinds {
    kinds.get(id.0 as usize).copied().unwrap_or(ValueKinds::ANY)
}

#[cfg(test)]
mod tests {
    use core::types::{IrNodeId, Value};

    use super::ValueFacts;
    use crate::{BinaryOp, IrNode, Module, PhiOperand, UnaryOp};

    #[test]
    fn propagates_refinements_through_phi_cycles() {
        let module = Module {
            nodes: vec![
                IrNode::Constant(Value::Num(2.0)),
                IrNode::Phi {
                    operands: vec![
                        PhiOperand {
                            block: IrNodeId(5),
                            value: IrNodeId(0),
                        },
                        PhiOperand {
                            block: IrNodeId(6),
                            value: IrNodeId(2),
                        },
                    ],
                },
                IrNode::Binary {
                    op: BinaryOp::Add,
                    lhs: IrNodeId(1),
                    rhs: IrNodeId(0),
                },
                IrNode::Unary {
                    op: UnaryOp::Not,
                    operand: IrNodeId(1),
                },
                IrNode::Unary {
                    op: UnaryOp::BitNot,
                    operand: IrNodeId(1),
                },
            ],
            ..Module::default()
        };

        let facts = ValueFacts::analyze(&module);

        assert!(facts.is_number(IrNodeId(1)));
        assert!(facts.is_number(IrNodeId(2)));
        assert!(facts.is_boolean(IrNodeId(3)));
        assert!(facts.is_u24(IrNodeId(4)));
    }

    #[test]
    fn does_not_assume_dynamic_values_are_numbers() {
        let module = Module {
            nodes: vec![IrNode::FunctionParameter(0)],
            ..Module::default()
        };

        let facts = ValueFacts::analyze(&module);

        assert!(!facts.is_number(IrNodeId(0)));
        assert!(!facts.is_boolean(IrNodeId(0)));
        assert!(!facts.is_u24(IrNodeId(0)));
    }

    #[test]
    fn preserves_negative_zero_as_a_general_number() {
        let module = Module {
            nodes: vec![IrNode::Constant(Value::Num(-0.0))],
            ..Module::default()
        };

        let facts = ValueFacts::analyze(&module);

        assert!(facts.is_number(IrNodeId(0)));
        assert!(!facts.is_boolean(IrNodeId(0)));
        assert!(!facts.is_u24(IrNodeId(0)));
    }
}
