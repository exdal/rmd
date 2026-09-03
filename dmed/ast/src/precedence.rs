use lexer::token::Token;

use crate::{AssignmentKind, BinaryOp, UnaryOp};

/// Loosest to tightest
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Precedence {
    Lowest,
    In,
    Assignment,
    Ternary,
    LogicalOr,
    LogicalAnd,
    BitOr,
    BitXor,
    BitAnd,
    Equality,
    Shift,
    Comparison,
    Additive,
    Multiplicative,
    Power,
    Unary,
    Postfix,
}

impl Precedence {
    pub fn is_right_associative(self) -> bool { self == Precedence::Assignment }

    pub fn next_tighter(self) -> Self {
        match self {
            Self::Lowest => Self::In,
            Self::In => Self::Assignment,
            Self::Assignment => Self::Ternary,
            Self::Ternary => Self::LogicalOr,
            Self::LogicalOr => Self::LogicalAnd,
            Self::LogicalAnd => Self::BitOr,
            Self::BitOr => Self::BitXor,
            Self::BitXor => Self::BitAnd,
            Self::BitAnd => Self::Equality,
            Self::Equality => Self::Shift,
            Self::Shift => Self::Comparison,
            Self::Comparison => Self::Additive,
            Self::Additive => Self::Multiplicative,
            Self::Multiplicative => Self::Power,
            Self::Power => Self::Unary,
            Self::Unary => Self::Postfix,
            Self::Postfix => Self::Postfix,
        }
    }
}

pub fn token_to_precedence(token: &Token<'_>) -> Precedence {
    match token {
        Token::Equal
        | Token::AddEqual
        | Token::SubEqual
        | Token::MulEqual
        | Token::DivEqual
        | Token::ModEqual
        | Token::FloatModuloEqual
        | Token::PowEqual
        | Token::AndEqual
        | Token::OrEqual
        | Token::XorEqual
        | Token::ShiftLeftEqual
        | Token::ShiftRightEqual
        | Token::LogicalAndEqual
        | Token::LogicalOrEqual
        | Token::AssignInto => Precedence::Assignment,

        Token::In => Precedence::In,

        Token::Question => Precedence::Ternary,
        Token::LogicalOr => Precedence::LogicalOr,
        Token::LogicalAnd => Precedence::LogicalAnd,
        Token::BitOr => Precedence::BitOr,
        Token::BitXor => Precedence::BitXor,
        Token::BitAnd => Precedence::BitAnd,

        Token::CompareEqual
        | Token::CompareNotEqual
        | Token::CompareNotEqualAlt
        | Token::Equivalent
        | Token::NotEquivalent => Precedence::Equality,

        Token::AngleLeft | Token::AngleRight | Token::LessEqual | Token::GreaterEqual | Token::CompareThreeWay => {
            Precedence::Comparison
        },

        Token::ShiftLeft | Token::ShiftRight => Precedence::Shift,
        Token::Add | Token::Sub => Precedence::Additive,
        Token::Mul | Token::Slash | Token::Modulo | Token::FloatModulo => Precedence::Multiplicative,
        Token::Pow => Precedence::Power,

        Token::Dot | Token::Colon | Token::SafeDot | Token::SafeColon | Token::Scope => Precedence::Postfix,
        Token::SquareLeft | Token::SafeSquare | Token::ParenLeft => Precedence::Postfix,

        _ => Precedence::Lowest,
    }
}

pub fn token_to_binary_op(token: &Token<'_>) -> Option<BinaryOp> {
    Some(match token {
        Token::Add => BinaryOp::Add,
        Token::Sub => BinaryOp::Sub,
        Token::Mul => BinaryOp::Mul,
        Token::Slash => BinaryOp::Div,
        Token::Modulo => BinaryOp::Mod,
        Token::FloatModulo => BinaryOp::FloatMod,
        Token::Pow => BinaryOp::Pow,
        Token::BitAnd => BinaryOp::BitAnd,
        Token::BitXor => BinaryOp::BitXor,
        Token::BitOr => BinaryOp::BitOr,
        Token::AngleRight => BinaryOp::CompGreater,
        Token::AngleLeft => BinaryOp::CompLess,
        Token::CompareEqual => BinaryOp::CompEq,
        Token::CompareNotEqual | Token::CompareNotEqualAlt => BinaryOp::CompNotEq,
        Token::GreaterEqual => BinaryOp::CompGreaterEq,
        Token::LessEqual => BinaryOp::CompLessEq,
        Token::Equivalent => BinaryOp::CompEquiv,
        Token::NotEquivalent => BinaryOp::CompNotEquiv,
        Token::CompareThreeWay => BinaryOp::CompThreeWay,
        Token::LogicalAnd => BinaryOp::LogicalAnd,
        Token::LogicalOr => BinaryOp::LogicalOr,
        Token::ShiftLeft => BinaryOp::ShiftLeft,
        Token::ShiftRight => BinaryOp::ShiftRight,
        Token::In => BinaryOp::In,
        _ => return None,
    })
}

pub fn token_to_assignment_kind(token: &Token<'_>) -> Option<AssignmentKind> {
    Some(match token {
        Token::Equal => AssignmentKind::Assign,
        Token::AddEqual => AssignmentKind::CompoundAdd,
        Token::SubEqual => AssignmentKind::CompoundSub,
        Token::MulEqual => AssignmentKind::CompoundMul,
        Token::DivEqual => AssignmentKind::CompoundDiv,
        Token::ModEqual => AssignmentKind::CompoundMod,
        Token::FloatModuloEqual => AssignmentKind::CompoundFloatMod,
        Token::PowEqual => AssignmentKind::CompoundPow,
        Token::AndEqual => AssignmentKind::CompoundAnd,
        Token::OrEqual => AssignmentKind::CompoundOr,
        Token::XorEqual => AssignmentKind::CompoundXor,
        Token::ShiftLeftEqual => AssignmentKind::CompoundShl,
        Token::ShiftRightEqual => AssignmentKind::CompoundShr,
        Token::LogicalAndEqual => AssignmentKind::LogicalAndSet,
        Token::LogicalOrEqual => AssignmentKind::LogicalOrSet,
        Token::AssignInto => AssignmentKind::AssignInto,
        _ => return None,
    })
}

pub fn token_to_unary_op(token: &Token<'_>) -> Option<UnaryOp> {
    Some(match token {
        Token::Sub => UnaryOp::Neg,
        Token::Exclaim => UnaryOp::Not,
        Token::BitNot => UnaryOp::BitNot,
        Token::Increment => UnaryOp::PreIncrement,
        Token::Decrement => UnaryOp::PreDecrement,
        Token::BitAnd => UnaryOp::Reference,
        Token::Mul => UnaryOp::Dereference,
        _ => return None,
    })
}
