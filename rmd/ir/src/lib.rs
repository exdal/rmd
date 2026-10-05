pub use core::types::{IrNodeId, ProcId, ProcKind, ProcParam};
use core::{
    interner::SymbolMap,
    location::Location,
    path::TreePath,
    types::{Identifier, Value, VarSpec},
};

pub use ast::{AccessKind, BinaryOp, Builtin, UnaryOp};

pub mod ast_lowering;
pub mod disasm;
mod lower;
pub mod opt;
pub mod verify;
pub use ast_lowering::IrModuleBuilder;
pub use lower::lower;
use objtree::{ObjectTree, TypeId};
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

#[derive(Debug, Clone)]
pub struct Interpolation {
    pub chunks: Vec<String>,
    pub values: Vec<IrNodeId>,
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
    Constant(Box<Value>),
    FunctionParameter(u32),
    Phi {
        operands: Vec<PhiOperand>,
    },
    Variable(Identifier),
    Load {
        pointer: IrNodeId,
    },
    Builtin(Builtin),
    Interpolate(Box<Interpolation>),
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
        args: Box<[Argument]>,
    },
    ModifiedType {
        path: Box<TreePath>,
        overrides: Box<[(Identifier, IrNodeId)]>,
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
        ty: Option<Box<TreePath>>,
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

#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<IrNode>() == 32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SideEffect {
    /// Evaluation has no observable behavior and cannot fault.
    Pure,
    /// Evaluation has no known mutation, but can fault for some operands.
    MayFault,
    /// Evaluation reads or changes runtime state, allocates an identity, or
    /// invokes code whose behavior is not represented by IR operands.
    Observable,
    /// Evaluation changes control flow.
    ControlFlow,
    /// The node describes IR structure rather than a movable instruction.
    Structural,
}

impl IrNode {
    pub fn effect(&self) -> SideEffect {
        match self {
            Self::Builtin(
                Builtin::Src | Builtin::Usr | Builtin::World | Builtin::Global | Builtin::Callee | Builtin::ThisProc,
            )
            | Self::Unary { op: UnaryOp::Not, .. }
            | Self::Binary {
                op:
                    BinaryOp::CompEq
                    | BinaryOp::CompNotEq
                    | BinaryOp::CompEquiv
                    | BinaryOp::CompNotEquiv
                    | BinaryOp::LogicalAnd
                    | BinaryOp::LogicalOr,
                ..
            }
            | Self::ModifiedType { .. } => SideEffect::Pure,

            Self::Interpolate(_)
            | Self::Unary { .. }
            | Self::Binary { .. }
            | Self::CompoundBinary { .. }
            | Self::InRange { .. }
            | Self::Range { .. }
            | Self::RangeTest { .. } => SideEffect::MayFault,

            Self::Load { .. }
            | Self::Builtin(_)
            | Self::AccessField { .. }
            | Self::Initial { .. }
            | Self::Index { .. }
            | Self::Call { .. }
            | Self::FunctionCall { .. }
            | Self::Super { .. }
            | Self::New { .. }
            | Self::List(_)
            | Self::Pick(_)
            | Self::IterInit { .. }
            | Self::IterNext(_)
            | Self::IterValue(_)
            | Self::IterKey(_)
            | Self::SetField { .. }
            | Self::SetIndex { .. }
            | Self::Store { .. }
            | Self::Initialize { .. }
            | Self::StoreBuiltin { .. }
            | Self::CatchValue
            | Self::Del(_)
            | Self::Output { .. } => SideEffect::Observable,

            Self::Branch(_)
            | Self::ConditionalBranch { .. }
            | Self::Return(_)
            | Self::Throw(_)
            | Self::Blocked(_)
            | Self::Trap { .. } => SideEffect::ControlFlow,

            Self::Function(_)
            | Self::ExternalFunction(_)
            | Self::Constant(_)
            | Self::FunctionParameter(_)
            | Self::Phi { .. }
            | Self::Variable(_)
            | Self::Label(_)
            | Self::SelectionMerge { .. }
            | Self::LoopMerge { .. }
            | Self::TryCatch { .. }
            | Self::Noop => SideEffect::Structural,
        }
    }

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
            Self::Interpolate(interpolation) => interpolation.values.iter().copied().for_each(&mut visit),
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
    pub index: ProcIndex,
}

#[derive(Debug, Clone, Default)]
pub struct ProcIndex {
    bodies: Vec<SymbolMap<ProcId>>,
    initializers: Vec<SymbolMap<ProcId>>,
}

impl ProcIndex {
    pub fn body(&self, ty: TypeId, name: &Identifier) -> Option<ProcId> { lookup(&self.bodies, ty, name) }

    pub fn initializer(&self, ty: TypeId, name: &Identifier) -> Option<ProcId> { lookup(&self.initializers, ty, name) }

    pub fn inherited_body(&self, tree: &ObjectTree, ty: TypeId, name: &Identifier) -> Option<ProcId> {
        let declaration = tree
            .ancestors(ty)
            .find(|declaration| declaration.procs.contains_key(name))?;

        self.body(declaration.id, name)
    }

    pub fn inherited_initializer(&self, tree: &ObjectTree, ty: TypeId, name: &Identifier) -> Option<ProcId> {
        let (declaration, _) = tree.var_declaration(ty, name)?;

        self.initializer(declaration.id, name)
    }

    fn set_body(&mut self, ty: TypeId, name: Identifier, body: Option<ProcId>) {
        set(&mut self.bodies, ty, name, body);
    }

    fn set_initializer(&mut self, ty: TypeId, name: Identifier, initializer: Option<ProcId>) {
        set(&mut self.initializers, ty, name, initializer);
    }
}

fn lookup(maps: &[SymbolMap<ProcId>], ty: TypeId, name: &Identifier) -> Option<ProcId> {
    maps.get(ty.0 as usize)?.get(name).copied()
}

fn set(maps: &mut Vec<SymbolMap<ProcId>>, ty: TypeId, name: Identifier, proc: Option<ProcId>) {
    let index = ty.0 as usize;
    if maps.len() <= index {
        maps.resize_with(index + 1, SymbolMap::default);
    }

    match proc {
        Some(proc) => {
            maps[index].insert(name, proc);
        },
        None => {
            maps[index].remove(&name);
        },
    }
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
