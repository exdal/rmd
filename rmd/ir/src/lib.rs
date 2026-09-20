pub use core::types::{IrNodeId, ProcId, ProcKind, ProcParam};
use core::{
    location::Location,
    path::TreePath,
    types::{Identifier, Value, VarSpec},
};

pub use ast::{AccessKind, BinaryOp, Builtin, UnaryOp};

pub mod ast_lowering;
pub mod disasm;
pub mod opt;
pub mod verify;
pub use ast_lowering::{IrModuleBuilder, UnresolvedNew};
pub use prelude::Intrinsic;
pub use verify::{VerifyError, verify};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct BindingId(pub u32);

impl std::fmt::Display for BindingId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "var{}", self.0) }
}

impl std::fmt::Debug for BindingId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { std::fmt::Display::fmt(self, f) }
}

#[derive(Debug, Clone)]
pub struct Argument {
    pub key: Option<IrNodeId>,
    pub value: Option<IrNodeId>,
}

#[derive(Debug, Clone)]
pub enum OutputTarget {
    Value(IrNodeId),
    Field {
        object: IrNodeId,
        name: Identifier,
        access: AccessKind,
    },
    Index {
        object: IrNodeId,
        index: IrNodeId,
        conditional: bool,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct PhiOperand {
    pub block: IrNodeId,
    pub value: IrNodeId,
}

#[derive(Debug, Clone)]
pub enum IrNode {
    Function(ProcId),
    ExternalFunction(Identifier),
    Constant(Value),
    FunctionParameter(u32),
    Phi {
        operands: Vec<PhiOperand>,
    },
    Variable(Identifier),
    Load {
        pointer: IrNodeId,
    },
    Builtin(Builtin),
    Interpolate {
        chunks: Vec<String>,
        values: Vec<IrNodeId>,
    },
    Unary {
        op: UnaryOp,
        operand: IrNodeId,
    },
    Binary {
        op: BinaryOp,
        lhs: IrNodeId,
        rhs: IrNodeId,
    },
    CompoundBinary {
        op: BinaryOp,
        lhs: IrNodeId,
        rhs: IrNodeId,
    },
    AccessField {
        object: IrNodeId,
        name: Identifier,
        access: AccessKind,
    },
    Initial {
        object: Option<IrNodeId>,
        name: Identifier,
    },
    Index {
        object: IrNodeId,
        index: IrNodeId,
        conditional: bool,
    },
    Call {
        callee: IrNodeId,
        args: Vec<Argument>,
    },
    FunctionCall {
        function: IrNodeId,
        args: Vec<Argument>,
    },
    Super {
        args: Vec<Argument>,
        forwards_extra_args: bool,
    },
    New {
        ty: Option<IrNodeId>,
        args: Vec<Argument>,
    },
    ModifiedType {
        path: TreePath,
        overrides: Vec<(Identifier, IrNodeId)>,
    },
    List(Vec<Argument>),
    Pick(Vec<(Option<IrNodeId>, IrNodeId)>),
    InRange {
        value: IrNodeId,
        start: IrNodeId,
        end: IrNodeId,
        step: Option<IrNodeId>,
    },
    Range {
        start: IrNodeId,
        end: IrNodeId,
        step: Option<IrNodeId>,
    },
    /// `for(var/mob/M in world)`, with `ty` the filter
    IterInit {
        list: IrNodeId,
        ty: Option<TreePath>,
        value_is_associated: bool,
    },
    /// advances and reports whether a value is now available
    IterNext(IrNodeId),
    IterValue(IrNodeId),
    IterKey(IrNodeId),
    /// `current <= end` counting up, `current >= end` counting down, decided by `step`'s sign
    RangeTest {
        current: IrNodeId,
        end: IrNodeId,
        step: IrNodeId,
    },

    SetField {
        object: IrNodeId,
        name: Identifier,
        access: AccessKind,
        value: IrNodeId,
    },
    SetIndex {
        object: IrNodeId,
        index: IrNodeId,
        value: IrNodeId,
        conditional: bool,
    },
    Store {
        pointer: IrNodeId,
        value: IrNodeId,
    },
    Initialize {
        pointer: IrNodeId,
        value: IrNodeId,
    },
    StoreBuiltin {
        builtin: Builtin,
        value: IrNodeId,
    },
    CatchValue,

    Label(Vec<IrNodeId>),
    SelectionMerge {
        merge_block: IrNodeId,
    },
    LoopMerge {
        merge_block: IrNodeId,
        continue_block: IrNodeId,
    },
    Branch(IrNodeId),
    ConditionalBranch {
        condition: IrNodeId,
        true_block: IrNodeId,
        false_block: IrNodeId,
    },
    Return(Option<IrNodeId>),

    Del(IrNodeId),
    Throw(IrNodeId),
    Output {
        target: OutputTarget,
        value: IrNodeId,
    },
    // DM unwinding has no branch form, so this stays structured. `body` and `catch` are blocks
    TryCatch {
        body: IrNodeId,
        catch: IrNodeId,
        merge: IrNodeId,
    },
    Noop,
    Blocked(&'static str),
    Trap {
        reason: String,
    },
}

impl IrNode {
    pub fn is_terminator(&self) -> bool {
        matches!(
            self,
            Self::Branch(_)
                | Self::ConditionalBranch { .. }
                | Self::Return(_)
                | Self::Throw(_)
                | Self::Blocked(_)
                | Self::Trap { .. }
        )
    }

    pub fn is_merge(&self) -> bool { matches!(self, Self::SelectionMerge { .. } | Self::LoopMerge { .. }) }

    pub fn operands(&self) -> Vec<IrNodeId> {
        let mut operands = Vec::new();
        self.for_each_operand(|operand| operands.push(operand));
        operands
    }

    pub fn for_each_operand(&self, mut visit: impl FnMut(IrNodeId)) {
        let args = |args: &[Argument], visit: &mut dyn FnMut(IrNodeId)| {
            for arg in args {
                if let Some(key) = arg.key {
                    visit(key);
                }
                if let Some(value) = arg.value {
                    visit(value);
                }
            }
        };

        match self {
            Self::Phi { operands, .. } => operands.iter().for_each(|operand| visit(operand.value)),
            Self::Unary { operand, .. } => visit(*operand),
            Self::Binary { lhs, rhs, .. } | Self::CompoundBinary { lhs, rhs, .. } => {
                visit(*lhs);
                visit(*rhs);
            },
            Self::Load { pointer } => visit(*pointer),
            Self::AccessField { object, .. } => visit(*object),
            Self::Initial { object, .. } => object.iter().copied().for_each(&mut visit),
            Self::Index { object, index, .. } => {
                visit(*object);
                visit(*index);
            },
            Self::Call { callee, args: a } => {
                visit(*callee);
                args(a, &mut visit);
            },
            Self::FunctionCall { function, args: a } => {
                visit(*function);
                args(a, &mut visit);
            },
            Self::Super { args: a, .. } | Self::List(a) => args(a, &mut visit),
            Self::New { ty, args: a } => {
                ty.iter().copied().for_each(&mut visit);
                args(a, &mut visit);
            },
            Self::ModifiedType { overrides, .. } => overrides.iter().for_each(|(_, id)| visit(*id)),
            Self::Pick(choices) => choices.iter().for_each(|(weight, value)| {
                weight.iter().copied().for_each(&mut visit);
                visit(*value);
            }),
            Self::Interpolate { values, .. } => values.iter().copied().for_each(&mut visit),
            Self::InRange {
                value,
                start,
                end,
                step,
            } => {
                visit(*value);
                visit(*start);
                visit(*end);
                step.iter().copied().for_each(&mut visit);
            },
            Self::Range { start, end, step } => {
                visit(*start);
                visit(*end);
                step.iter().copied().for_each(&mut visit);
            },
            Self::RangeTest { current, end, step } => {
                visit(*current);
                visit(*end);
                visit(*step);
            },
            Self::IterInit { list, .. } => visit(*list),
            Self::IterNext(id) | Self::IterValue(id) | Self::IterKey(id) => visit(*id),
            Self::SetField { object, value, .. } => {
                visit(*object);
                visit(*value);
            },
            Self::SetIndex {
                object, index, value, ..
            } => {
                visit(*object);
                visit(*index);
                visit(*value);
            },
            Self::Store { pointer, value } | Self::Initialize { pointer, value } => {
                visit(*pointer);
                visit(*value);
            },
            Self::StoreBuiltin { value, .. } => visit(*value),
            Self::ConditionalBranch { condition, .. } => visit(*condition),
            Self::Return(value) => value.iter().copied().for_each(&mut visit),
            Self::Output { target, value } => {
                match target {
                    OutputTarget::Value(target) => visit(*target),
                    OutputTarget::Field { object, .. } => visit(*object),
                    OutputTarget::Index { object, index, .. } => {
                        visit(*object);
                        visit(*index);
                    },
                }
                visit(*value);
            },
            Self::Del(id) | Self::Throw(id) => visit(*id),
            _ => {},
        }
    }
}

#[derive(Debug, Clone)]
pub struct Procedure {
    pub function: IrNodeId,
    pub parameters: Vec<IrNodeId>,
    pub previous: Option<ProcId>,
    pub owner: TreePath,
    pub name: Identifier,
    pub params: Vec<ProcParam<IrNodeId>>,
    pub variadic: bool,
    pub vars: Vec<VarSpec<IrNodeId>>,
    pub body: IrNodeId,
    pub intrinsic: Option<Intrinsic>,
    pub location: Location,
}

#[derive(Debug, Default)]
pub struct Module {
    pub nodes: Vec<IrNode>,
    /// Module-scope constants, interned by value and shared by every procedure.
    pub constants: Vec<IrNodeId>,
    /// Module-scope declarations for bare proc names defined outside this module.
    pub external_functions: Vec<IrNodeId>,
    pub procs: Vec<Procedure>,
}

impl Module {
    pub fn node(&self, id: IrNodeId) -> Option<&IrNode> { self.nodes.get(id.0 as usize) }

    pub fn proc(&self, id: ProcId) -> Option<&Procedure> { self.procs.get(id.0 as usize) }

    pub fn block(&self, id: IrNodeId) -> Option<&[IrNodeId]> {
        match self.node(id)? {
            IrNode::Label(instructions) => Some(instructions),
            _ => None,
        }
    }
}
