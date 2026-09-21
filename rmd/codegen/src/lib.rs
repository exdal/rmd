pub mod disasm;
pub mod error;
pub mod opcode;
mod reachability;
mod stackify;

use core::{
    location::Location,
    path::TreePath,
    types::{Identifier, IrNodeId, ProcId, Value},
};
use std::collections::{HashMap, HashSet, VecDeque};

pub use error::CodegenError;
use ir::{Argument, IrNode, OutputTarget, Procedure};
use opcode::{ARGUMENT_KEY, ARGUMENT_VALUE, Access, Binary, Builtin, Op, OutputTargetKind, Unary};
use prelude::Intrinsic;

macro_rules! id_type {
    ($name:ident, $prefix:literal) => {
        #[derive(Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $name(pub u32);

        impl $name {
            pub const INVALID: Self = Self(u32::MAX);

            pub const fn is_valid(self) -> bool { self.0 != u32::MAX }

            pub const fn is_invalid(self) -> bool { self.0 == u32::MAX }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                match self.is_valid() {
                    true => write!(formatter, concat!($prefix, "{}"), self.0),
                    false => formatter.write_str(concat!($prefix, "-")),
                }
            }
        }

        impl std::fmt::Debug for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                std::fmt::Display::fmt(self, formatter)
            }
        }
    };
}

id_type!(FunctionId, "fn");
id_type!(LocalId, "local");
id_type!(ConstantId, "const");
id_type!(StringId, "str");
id_type!(PathId, "path");

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct CodeOffset(pub u32);

impl CodeOffset {
    pub const INVALID: Self = Self(u32::MAX);

    pub const fn is_valid(self) -> bool { self.0 != u32::MAX }

    pub const fn is_invalid(self) -> bool { self.0 == u32::MAX }
}

impl std::fmt::Display for CodeOffset {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.is_valid() {
            true => write!(formatter, "0x{:08x}", self.0),
            false => formatter.write_str("0x--------"),
        }
    }
}

impl std::fmt::Debug for CodeOffset {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, formatter)
    }
}

#[derive(Debug, Clone)]
pub struct Module {
    pub magic: [u8; 4],
    pub version: u16,
    pub constants: Vec<Value>,
    pub strings: Vec<String>,
    pub symbols: Vec<Identifier>,
    pub paths: Vec<TreePath>,
    pub functions: Vec<CompiledFunction>,
    pub code: Vec<u8>,
    proc_functions: Vec<FunctionId>,
}

impl Module {
    pub const MAGIC: [u8; 4] = *b"DMIR";
    pub const VERSION: u16 = 2;

    pub fn function(&self, id: FunctionId) -> Option<&CompiledFunction> { self.functions.get(id.0 as usize) }

    pub fn function_for_proc(&self, proc: ProcId) -> Option<&CompiledFunction> {
        let id = self.proc_functions.get(proc.0 as usize).copied()?;

        self.function(id).filter(|function| function.proc == Some(proc))
    }

    fn index_procs(functions: &[CompiledFunction]) -> Vec<FunctionId> {
        let slots = functions
            .iter()
            .filter_map(|function| function.proc)
            .map(|proc| proc.0 as usize + 1)
            .max()
            .unwrap_or_default();

        let mut index = vec![FunctionId::INVALID; slots];
        for function in functions {
            if let Some(proc) = function.proc
                && let Some(slot) = index.get_mut(proc.0 as usize)
                && slot.is_invalid()
            {
                *slot = function.id;
            }
        }

        index
    }
}

#[derive(Debug, Clone)]
pub struct CompiledFunction {
    pub id: FunctionId,
    pub proc: Option<ProcId>,
    pub name: StringId,
    pub proc_name: StringId,
    pub owner: TreePath,
    pub previous: Option<ProcId>,
    pub parameter_names: Vec<StringId>,
    pub address: CodeOffset,
    pub length: u32,
    pub parameter_count: u32,
    pub local_count: u32,
    pub external: bool,
    pub intrinsic: Option<Intrinsic>,
    pub location: Location,
}

#[derive(Debug, Clone)]
pub enum ProcedureReachability {
    All,
    Selected(HashSet<ProcId>),
}

pub fn generate(module: &ir::Module) -> Result<Module, CodegenError> { Generator::new().generate(module, None) }

pub fn reachable_procedures(
    module: &ir::Module, tree: &objtree::ObjectTree, roots: &[ProcId],
) -> Result<ProcedureReachability, CodegenError> {
    reachability::reachable_procedures(module, tree, roots)
}

pub fn generate_reachable(
    module: &ir::Module, tree: &objtree::ObjectTree, roots: &[ProcId],
) -> Result<Module, CodegenError> {
    match reachable_procedures(module, tree, roots)? {
        ProcedureReachability::All => Generator::new().generate(module, None),
        ProcedureReachability::Selected(procedures) => Generator::new().generate(module, Some(&procedures)),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Label {
    Block(IrNodeId),
    Synthetic(usize),
}

#[derive(Debug)]
struct AddressPatch {
    operand: usize,
    target: Label,
}

#[derive(Debug)]
struct JumpPatch {
    instruction: usize,
    address: usize,
    op: Op,
}

#[derive(Debug)]
struct EdgeStub {
    label: Label,
    predecessor: IrNodeId,
    target: IrNodeId,
}

struct FunctionState {
    blocks: Vec<IrNodeId>,
    locals: HashMap<IrNodeId, LocalId>,
    parameter_defaults: HashMap<IrNodeId, LocalId>,
    next_local: u32,
    stackify: stackify::Stackify,
}

impl FunctionState {
    fn reserve(&mut self, node: IrNodeId) -> LocalId {
        if let Some(local) = self.locals.get(&node) {
            return *local;
        }

        let local = LocalId(self.next_local);
        self.next_local += 1;
        self.locals.insert(node, local);

        local
    }

    fn local(&self, node: IrNodeId) -> Result<LocalId, CodegenError> {
        self.locals.get(&node).copied().ok_or(CodegenError::MissingLocal(node))
    }

    fn assigned_local(&self, node: IrNodeId) -> Option<LocalId> { self.locals.get(&node).copied() }

    fn assign_local(&mut self, node: IrNodeId, local: LocalId) {
        let previous = self.locals.insert(node, local);
        if let Some(previous) = previous {
            debug_assert_eq!(previous, local);
        }
    }
}

struct Generator {
    constants: Vec<Value>,
    constant_ids: HashMap<IrNodeId, ConstantId>,
    strings: Vec<String>,
    string_ids: HashMap<String, StringId>,
    paths: Vec<TreePath>,
    path_ids: HashMap<TreePath, PathId>,
    functions: Vec<CompiledFunction>,
    function_ids: HashMap<IrNodeId, FunctionId>,
    labels: HashMap<Label, usize>,
    addresses: Vec<AddressPatch>,
    jumps: Vec<JumpPatch>,
    next_label: usize,
    code: Vec<u8>,
    /// `a.b()` looks `b` up as a proc first, and plain `a.b` as a var first
    callees: HashSet<IrNodeId>,
}

impl Generator {
    fn new() -> Self {
        Self {
            constants: Vec::new(),
            constant_ids: HashMap::new(),
            strings: Vec::new(),
            string_ids: HashMap::new(),
            paths: Vec::new(),
            path_ids: HashMap::new(),
            functions: Vec::new(),
            function_ids: HashMap::new(),
            labels: HashMap::new(),
            addresses: Vec::new(),
            jumps: Vec::new(),
            next_label: 0,
            code: Vec::new(),
            callees: HashSet::new(),
        }
    }

    fn generate(mut self, module: &ir::Module, procedures: Option<&HashSet<ProcId>>) -> Result<Module, CodegenError> {
        self.callees = module
            .nodes
            .iter()
            .filter_map(|node| match node {
                IrNode::Call { callee, .. } => Some(*callee),
                _ => None,
            })
            .collect();
        let referenced = procedures
            .map(|procedures| referenced_nodes(module, procedures))
            .transpose()?;
        self.register_constants(module, referenced.as_ref())?;
        self.register_functions(module, procedures, referenced.as_ref())?;

        for (index, proc) in module.procs.iter().enumerate() {
            let id = ProcId(index as u32);
            if procedures.is_some_and(|procedures| !procedures.contains(&id)) {
                continue;
            }
            self.generate_function(module, proc)?;
        }

        Ok(Module {
            magic: Module::MAGIC,
            version: Module::VERSION,
            constants: self.constants,
            symbols: self
                .strings
                .iter()
                .map(|value| Identifier::from(value.as_str()))
                .collect(),
            strings: self.strings,
            paths: self.paths,
            proc_functions: Module::index_procs(&self.functions),
            functions: self.functions,
            code: self.code,
        })
    }

    fn register_constants(
        &mut self, module: &ir::Module, referenced: Option<&HashSet<IrNodeId>>,
    ) -> Result<(), CodegenError> {
        for node in &module.constants {
            if referenced.is_some_and(|referenced| !referenced.contains(node)) {
                continue;
            }
            let Some(IrNode::Constant(value)) = module.node(*node) else {
                return Err(CodegenError::MissingNode(*node));
            };
            let id =
                ConstantId(u32::try_from(self.constants.len()).map_err(|_| CodegenError::PoolTooLarge("constant"))?);
            self.constant_ids.insert(*node, id);
            self.constants.push(value.clone());
        }

        Ok(())
    }

    fn register_functions(
        &mut self, module: &ir::Module, procedures: Option<&HashSet<ProcId>>, referenced: Option<&HashSet<IrNodeId>>,
    ) -> Result<(), CodegenError> {
        for (index, proc) in module.procs.iter().enumerate() {
            let proc_id = ProcId(index as u32);
            if procedures.is_some_and(|procedures| !procedures.contains(&proc_id)) {
                continue;
            }
            let id =
                FunctionId(u32::try_from(self.functions.len()).map_err(|_| CodegenError::PoolTooLarge("function"))?);
            let name = self.intern_string(format!("{}::{}", proc.owner, proc.name))?;
            let proc_name = self.intern_identifier(&proc.name)?;
            let parameter_names = proc
                .params
                .iter()
                .map(|parameter| self.intern_identifier(&parameter.spec.name))
                .collect::<Result<Vec<_>, _>>()?;
            self.function_ids.insert(proc.function, id);
            self.functions.push(CompiledFunction {
                id,
                proc: Some(proc_id),
                name,
                proc_name,
                owner: proc.owner.clone(),
                previous: proc.previous,
                parameter_names,
                address: CodeOffset::INVALID,
                length: 0,
                parameter_count: proc.parameters.len() as u32,
                local_count: 0,
                external: false,
                intrinsic: proc.intrinsic,
                location: proc.location,
            });
        }

        for node in &module.external_functions {
            if referenced.is_some_and(|referenced| !referenced.contains(node)) {
                continue;
            }
            let Some(IrNode::ExternalFunction(name)) = module.node(*node) else {
                return Err(CodegenError::MissingNode(*node));
            };
            let id =
                FunctionId(u32::try_from(self.functions.len()).map_err(|_| CodegenError::PoolTooLarge("function"))?);
            let name = self.intern_string(name.as_str().to_owned())?;
            self.function_ids.insert(*node, id);
            self.functions.push(CompiledFunction {
                id,
                proc: None,
                name,
                proc_name: name,
                owner: TreePath::default(),
                previous: None,
                parameter_names: Vec::new(),
                address: CodeOffset::INVALID,
                length: 0,
                parameter_count: 0,
                local_count: 0,
                external: true,
                intrinsic: None,
                location: Location::default(),
            });
        }

        Ok(())
    }

    fn generate_function(&mut self, module: &ir::Module, proc: &Procedure) -> Result<(), CodegenError> {
        let blocks = reachable_blocks(module, proc.body)?;
        let layout = block_layout(module, proc.body, &blocks)?;
        let stackify = stackify::Stackify::analyze(module, proc, &blocks)?;
        let mut state = FunctionState {
            blocks,
            locals: HashMap::new(),
            parameter_defaults: HashMap::new(),
            next_local: 0,
            stackify,
        };

        for parameter in &proc.parameters {
            state.reserve(*parameter);
        }

        let mut constant_defaults = Vec::new();
        for (index, parameter) in proc.params.iter().enumerate() {
            let Some(default) = parameter.default else {
                continue;
            };
            let destination = state.local(proc.parameters[index])?;
            if matches!(module.node(default), Some(IrNode::Constant(_))) {
                constant_defaults.push((default, destination));
            } else {
                state.parameter_defaults.insert(default, destination);
            }
        }

        assign_phi_locals(module, &mut state)?;

        for block in state.blocks.clone() {
            for instruction in module.block(block).ok_or(CodegenError::ExpectedBlock(block))? {
                if module.node(*instruction).is_some_and(produces_value) && state.stackify.needs_local(*instruction) {
                    state.reserve(*instruction);
                }
            }
        }

        let address = self.code_offset()?;
        for (default, destination) in constant_defaults {
            self.emit_value(module, &state, default)?;
            self.emit_op(Op::DefaultParameter);
            self.emit_u32(destination.0);
        }

        let mut edge_stubs = Vec::new();

        for (index, block) in layout.iter().copied().enumerate() {
            let next = layout.get(index + 1).copied();
            self.generate_block(module, block, next, &state, &mut edge_stubs)?;
        }

        for stub in edge_stubs {
            self.mark_label(stub.label);
            self.emit_phi_copies(module, &state, stub.predecessor, stub.target)?;
            self.emit_jump(Op::Jump, Label::Block(stub.target));
        }

        let length = self.finish_function(address.0 as usize)?;
        let id = *self
            .function_ids
            .get(&proc.function)
            .ok_or(CodegenError::MissingFunction(proc.function))?;
        let function = self
            .functions
            .get_mut(id.0 as usize)
            .ok_or(CodegenError::MissingFunction(proc.function))?;
        function.address = address;
        function.length = length;
        function.local_count = state.next_local;

        Ok(())
    }

    fn generate_block(
        &mut self, module: &ir::Module, block: IrNodeId, next: Option<IrNodeId>, state: &FunctionState,
        edge_stubs: &mut Vec<EdgeStub>,
    ) -> Result<(), CodegenError> {
        self.mark_label(Label::Block(block));
        let instructions = state
            .stackify
            .schedule(block)
            .ok_or(CodegenError::ExpectedBlock(block))?;

        for instruction in instructions {
            if state.stackify.is_stacked(*instruction) {
                continue;
            }

            match module
                .node(*instruction)
                .ok_or(CodegenError::MissingNode(*instruction))?
            {
                IrNode::Phi { .. }
                | IrNode::Variable(_)
                | IrNode::SelectionMerge { .. }
                | IrNode::LoopMerge { .. }
                | IrNode::Noop => {},
                IrNode::Branch(target) => {
                    self.emit_phi_copies(module, state, block, *target)?;
                    self.emit_jump(Op::Jump, Label::Block(*target));
                },
                IrNode::ConditionalBranch {
                    condition,
                    true_block,
                    false_block,
                } => {
                    self.emit_value(module, state, *condition)?;
                    let false_target = if has_phi_copies(module, state, block, *false_block)? {
                        // Phi copies belong to the incoming edge so keep the false
                        // copies behind the conditional while the true copies can
                        // lead directly into the preferred fallthrough block
                        let label = self.fresh_label();
                        edge_stubs.push(EdgeStub {
                            label,
                            predecessor: block,
                            target: *false_block,
                        });

                        label
                    } else {
                        Label::Block(*false_block)
                    };
                    self.emit_jump(Op::JumpIfFalse, false_target);

                    self.emit_phi_copies(module, state, block, *true_block)?;
                    if next != Some(*true_block) {
                        self.emit_jump(Op::Jump, Label::Block(*true_block));
                    }
                },
                node => self.generate_instruction(module, state, *instruction, node)?,
            }
        }

        Ok(())
    }

    fn generate_instruction(
        &mut self, module: &ir::Module, state: &FunctionState, id: IrNodeId, node: &IrNode,
    ) -> Result<(), CodegenError> {
        match node {
            IrNode::Constant(_)
            | IrNode::Function(_)
            | IrNode::ExternalFunction(_)
            | IrNode::FunctionParameter(_)
            | IrNode::Label(_)
            | IrNode::Phi { .. }
            | IrNode::Variable(_)
            | IrNode::SelectionMerge { .. }
            | IrNode::LoopMerge { .. }
            | IrNode::Branch(_)
            | IrNode::ConditionalBranch { .. }
            | IrNode::Noop => {},
            IrNode::Load { pointer } => {
                let name = variable_name(module, *pointer)?;
                let name = self.intern_identifier(name)?;
                self.emit_op(Op::LoadVariable);
                self.emit_u32(name.0);
                self.store_result(state, id)?;
            },
            IrNode::Builtin(builtin) => {
                self.emit_op(Op::PushBuiltin);
                self.emit_u8(builtin_code(*builtin) as u8);
                self.store_result(state, id)?;
            },
            IrNode::Interpolate { chunks, values } => {
                for value in values {
                    self.emit_value(module, state, *value)?;
                }

                let chunks = chunks
                    .iter()
                    .map(|chunk| self.intern_string(chunk.clone()))
                    .collect::<Result<Vec<_>, _>>()?;
                self.emit_op(Op::Interpolate);
                self.emit_count(values.len(), "interpolation value")?;
                self.emit_count(chunks.len(), "interpolation chunk")?;
                for chunk in chunks {
                    self.emit_u32(chunk.0);
                }
                self.store_result(state, id)?;
            },
            IrNode::Unary { op, operand } => {
                self.emit_value(module, state, *operand)?;
                self.emit_op(Op::Unary);
                self.emit_u8(unary_code(*op) as u8);
                self.store_result(state, id)?;
            },
            IrNode::Binary { op, lhs, rhs } => {
                self.emit_value(module, state, *lhs)?;
                self.emit_value(module, state, *rhs)?;
                self.emit_op(Op::Binary);
                self.emit_u8(binary_code(*op) as u8);
                self.store_result(state, id)?;
            },
            IrNode::CompoundBinary { op, lhs, rhs } => {
                self.emit_value(module, state, *lhs)?;
                self.emit_value(module, state, *rhs)?;
                self.emit_op(Op::CompoundBinary);
                self.emit_u8(binary_code(*op) as u8);
                self.store_result(state, id)?;
            },
            IrNode::AccessField { object, name, access } => {
                self.emit_value(module, state, *object)?;
                let name = self.intern_identifier(name)?;
                self.emit_op(Op::AccessField);
                self.emit_u32(name.0);
                self.emit_u8(access_code(*access) as u8);
                self.emit_bool(self.callees.contains(&id));
                self.store_result(state, id)?;
            },
            IrNode::Initial { object, name } => {
                if let Some(object) = object {
                    self.emit_value(module, state, *object)?;
                }
                let name = self.intern_identifier(name)?;
                self.emit_op(Op::Initial);
                self.emit_bool(object.is_some());
                self.emit_u32(name.0);
                self.store_result(state, id)?;
            },
            IrNode::Index {
                object,
                index,
                conditional,
            } => {
                self.emit_value(module, state, *object)?;
                self.emit_value(module, state, *index)?;
                self.emit_op(Op::Index);
                self.emit_bool(*conditional);
                self.store_result(state, id)?;
            },
            IrNode::Call { callee, args } => {
                self.emit_value(module, state, *callee)?;
                let shapes = self.emit_arguments(module, state, args)?;
                let conditional = matches!(
                    module.node(*callee),
                    Some(IrNode::AccessField {
                        access: ir::AccessKind::SafeDot | ir::AccessKind::SafeColon,
                        ..
                    })
                );
                self.emit_op(Op::Call);
                self.emit_argument_layout(&shapes)?;
                self.emit_bool(conditional);
                self.store_result(state, id)?;
            },
            IrNode::FunctionCall { function, args } => {
                let shapes = self.emit_arguments(module, state, args)?;
                let function = self
                    .function_ids
                    .get(function)
                    .copied()
                    .ok_or(CodegenError::MissingFunction(*function))?;
                self.emit_op(Op::FunctionCall);
                self.emit_u32(function.0);
                self.emit_argument_layout(&shapes)?;
                self.store_result(state, id)?;
            },
            IrNode::Super {
                args,
                forwards_extra_args,
            } => {
                let shapes = self.emit_arguments(module, state, args)?;
                self.emit_op(Op::SuperCall);
                self.emit_argument_layout(&shapes)?;
                self.emit_bool(*forwards_extra_args);
                self.store_result(state, id)?;
            },
            IrNode::New { ty, args } => {
                if let Some(ty) = ty {
                    self.emit_value(module, state, *ty)?;
                }
                let shapes = self.emit_arguments(module, state, args)?;
                self.emit_op(Op::New);
                self.emit_bool(ty.is_some());
                self.emit_argument_layout(&shapes)?;
                self.store_result(state, id)?;
            },
            IrNode::ModifiedType { path, overrides } => {
                for (_, value) in overrides {
                    self.emit_value(module, state, *value)?;
                }
                let path = self.intern_path(path.clone())?;
                let names = overrides
                    .iter()
                    .map(|(name, _)| self.intern_identifier(name))
                    .collect::<Result<Vec<_>, _>>()?;
                self.emit_op(Op::ModifiedType);
                self.emit_u32(path.0);
                self.emit_count(names.len(), "modified type override")?;
                for name in names {
                    self.emit_u32(name.0);
                }
                self.store_result(state, id)?;
            },
            IrNode::List(args) => {
                let shapes = self.emit_arguments(module, state, args)?;
                self.emit_op(Op::MakeList);
                self.emit_argument_layout(&shapes)?;
                self.store_result(state, id)?;
            },
            IrNode::Pick(choices) => {
                let mut weighted = Vec::with_capacity(choices.len());
                for (weight, value) in choices {
                    if let Some(weight) = weight {
                        self.emit_value(module, state, *weight)?;
                    }
                    self.emit_value(module, state, *value)?;
                    weighted.push(weight.is_some());
                }
                self.emit_op(Op::Pick);
                self.emit_count(weighted.len(), "pick choice")?;
                for weighted in weighted {
                    self.emit_bool(weighted);
                }
                self.store_result(state, id)?;
            },
            IrNode::InRange {
                value,
                start,
                end,
                step,
            } => {
                self.emit_value(module, state, *value)?;
                self.emit_value(module, state, *start)?;
                self.emit_value(module, state, *end)?;
                if let Some(step) = step {
                    self.emit_value(module, state, *step)?;
                }
                self.emit_op(Op::InRange);
                self.emit_bool(step.is_some());
                self.store_result(state, id)?;
            },
            IrNode::Range { start, end, step } => {
                self.emit_value(module, state, *start)?;
                self.emit_value(module, state, *end)?;
                if let Some(step) = step {
                    self.emit_value(module, state, *step)?;
                }
                self.emit_op(Op::Range);
                self.emit_bool(step.is_some());
                self.store_result(state, id)?;
            },
            IrNode::IterInit {
                list,
                ty,
                value_is_associated,
            } => {
                self.emit_value(module, state, *list)?;
                let ty = ty.clone().map(|ty| self.intern_path(ty)).transpose()?;
                self.emit_op(Op::IterInit);
                self.emit_optional_path(ty);
                self.emit_bool(*value_is_associated);
                self.store_result(state, id)?;
            },
            IrNode::IterNext(iter) => {
                self.emit_value(module, state, *iter)?;
                self.emit_op(Op::IterNext);
                self.store_result(state, id)?;
            },
            IrNode::IterValue(iter) => {
                self.emit_value(module, state, *iter)?;
                self.emit_op(Op::IterValue);
                self.store_result(state, id)?;
            },
            IrNode::IterKey(iter) => {
                self.emit_value(module, state, *iter)?;
                self.emit_op(Op::IterKey);
                self.store_result(state, id)?;
            },
            IrNode::RangeTest { current, end, step } => {
                self.emit_value(module, state, *current)?;
                self.emit_value(module, state, *end)?;
                self.emit_value(module, state, *step)?;
                self.emit_op(Op::RangeTest);
                self.store_result(state, id)?;
            },
            IrNode::SetField {
                object,
                name,
                access,
                value,
            } => {
                self.emit_value(module, state, *object)?;
                self.emit_value(module, state, *value)?;
                let name = self.intern_identifier(name)?;
                self.emit_op(Op::SetField);
                self.emit_u32(name.0);
                self.emit_u8(access_code(*access) as u8);
            },
            IrNode::SetIndex {
                object,
                index,
                value,
                conditional,
            } => {
                self.emit_value(module, state, *object)?;
                self.emit_value(module, state, *index)?;
                self.emit_value(module, state, *value)?;
                self.emit_op(Op::SetIndex);
                self.emit_bool(*conditional);
            },
            IrNode::Store { pointer, value } => {
                self.emit_value(module, state, *value)?;
                let name = variable_name(module, *pointer)?;
                let name = self.intern_identifier(name)?;
                self.emit_op(Op::StoreVariable);
                self.emit_u32(name.0);
            },
            IrNode::Initialize { pointer, value } => {
                self.emit_value(module, state, *value)?;
                let name = variable_name(module, *pointer)?;
                let name = self.intern_identifier(name)?;
                self.emit_op(Op::InitializeVariable);
                self.emit_u32(name.0);
            },
            IrNode::StoreBuiltin { builtin, value } => {
                self.emit_value(module, state, *value)?;
                self.emit_op(Op::StoreBuiltin);
                self.emit_u8(builtin_code(*builtin) as u8);
            },
            IrNode::CatchValue => {
                self.emit_op(Op::CatchValue);
                self.store_result(state, id)?;
            },
            IrNode::Return(value) => match value {
                Some(value) => {
                    self.emit_value(module, state, *value)?;
                    self.emit_op(Op::ReturnValue);
                },
                None => self.emit_op(Op::Return),
            },
            IrNode::Del(value) => {
                self.emit_value(module, state, *value)?;
                self.emit_op(Op::Del);
            },
            IrNode::Throw(value) => {
                self.emit_value(module, state, *value)?;
                self.emit_op(Op::Throw);
            },
            IrNode::Output { target, value } => {
                match target {
                    OutputTarget::Value(target) => self.emit_value(module, state, *target)?,
                    OutputTarget::Field { object, .. } => self.emit_value(module, state, *object)?,
                    OutputTarget::Index { object, index, .. } => {
                        self.emit_value(module, state, *object)?;
                        self.emit_value(module, state, *index)?;
                    },
                }
                self.emit_value(module, state, *value)?;
                self.emit_op(Op::Output);
                match target {
                    OutputTarget::Value(_) => self.emit_u8(OutputTargetKind::Value as u8),
                    OutputTarget::Field { name, access, .. } => {
                        let name = self.intern_identifier(name)?;
                        self.emit_u8(OutputTargetKind::Field as u8);
                        self.emit_u32(name.0);
                        self.emit_u8(access_code(*access) as u8);
                    },
                    OutputTarget::Index { conditional, .. } => {
                        self.emit_u8(OutputTargetKind::Index as u8);
                        self.emit_bool(*conditional);
                    },
                }
            },
            IrNode::TryCatch { body, catch, merge } => {
                self.emit_op(Op::TryCatch);
                self.emit_address(Label::Block(*body));
                self.emit_address(Label::Block(*catch));
                self.emit_address(Label::Block(*merge));
            },
            IrNode::Blocked(reason) => {
                let reason = self.intern_string((*reason).to_owned())?;
                self.emit_op(Op::Blocked);
                self.emit_u32(reason.0);
                return Ok(());
            },
            IrNode::Trap { reason } => {
                let reason = self.intern_string(reason.clone())?;
                self.emit_op(Op::Trap);
                self.emit_u32(reason.0);
                return Ok(());
            },
        }

        if let Some(parameter) = state.parameter_defaults.get(&id) {
            self.emit_value(module, state, id)?;
            self.emit_op(Op::DefaultParameter);
            self.emit_u32(parameter.0);
        }

        Ok(())
    }

    fn emit_phi_copies(
        &mut self, module: &ir::Module, state: &FunctionState, predecessor: IrNodeId, target: IrNodeId,
    ) -> Result<(), CodegenError> {
        let copies = phi_copies(module, state, predecessor, target)?;

        for (source, _) in &copies {
            self.emit_value(module, state, *source)?;
        }

        for (_, destination) in copies.iter().rev() {
            self.emit_op(Op::StoreLocal);
            self.emit_u32(destination.0);
        }

        Ok(())
    }

    fn emit_value(&mut self, module: &ir::Module, state: &FunctionState, value: IrNodeId) -> Result<(), CodegenError> {
        if state.stackify.is_stacked(value) {
            let node = module.node(value).ok_or(CodegenError::MissingNode(value))?;
            return self.generate_instruction(module, state, value, node);
        }

        match module.node(value).ok_or(CodegenError::MissingNode(value))? {
            IrNode::Constant(_) => {
                let constant = self
                    .constant_ids
                    .get(&value)
                    .copied()
                    .ok_or(CodegenError::MissingConstant(value))?;
                self.emit_op(Op::PushConstant);
                self.emit_u32(constant.0);
            },
            _ => {
                let local = state.local(value)?;
                self.emit_op(Op::LoadLocal);
                self.emit_u32(local.0);
            },
        }

        Ok(())
    }

    fn store_result(&mut self, state: &FunctionState, value: IrNodeId) -> Result<(), CodegenError> {
        if state.stackify.is_stacked(value) {
            return Ok(());
        }

        if state.stackify.uses(value) == 0 {
            self.emit_op(Op::Drop);
            return Ok(());
        }

        let local = state.local(value)?;
        self.emit_op(Op::StoreLocal);
        self.emit_u32(local.0);

        Ok(())
    }

    fn emit_arguments(
        &mut self, module: &ir::Module, state: &FunctionState, args: &[Argument],
    ) -> Result<Vec<u8>, CodegenError> {
        let mut shapes = Vec::with_capacity(args.len());
        for argument in args {
            let mut shape = 0;
            if let Some(key) = argument.key {
                self.emit_value(module, state, key)?;
                shape |= ARGUMENT_KEY;
            }
            if let Some(value) = argument.value {
                self.emit_value(module, state, value)?;
                shape |= ARGUMENT_VALUE;
            }
            shapes.push(shape);
        }

        Ok(shapes)
    }

    fn emit_argument_layout(&mut self, shapes: &[u8]) -> Result<(), CodegenError> {
        self.emit_count(shapes.len(), "argument")?;
        self.code.extend_from_slice(shapes);

        Ok(())
    }

    fn emit_optional_path(&mut self, path: Option<PathId>) {
        self.emit_bool(path.is_some());

        if let Some(path) = path {
            self.emit_u32(path.0);
        }
    }

    fn intern_identifier(&mut self, identifier: &Identifier) -> Result<StringId, CodegenError> {
        self.intern_string(identifier.as_str().to_owned())
    }

    fn intern_string(&mut self, value: String) -> Result<StringId, CodegenError> {
        if let Some(id) = self.string_ids.get(&value) {
            return Ok(*id);
        }

        let id = StringId(u32::try_from(self.strings.len()).map_err(|_| CodegenError::PoolTooLarge("string"))?);
        self.string_ids.insert(value.clone(), id);
        self.strings.push(value);

        Ok(id)
    }

    fn intern_path(&mut self, value: TreePath) -> Result<PathId, CodegenError> {
        if let Some(id) = self.path_ids.get(&value) {
            return Ok(*id);
        }

        let id = PathId(u32::try_from(self.paths.len()).map_err(|_| CodegenError::PoolTooLarge("path"))?);
        self.path_ids.insert(value.clone(), id);
        self.paths.push(value);

        Ok(id)
    }

    fn emit_op(&mut self, op: Op) { self.emit_u8(op as u8); }

    fn emit_bool(&mut self, value: bool) { self.emit_u8(u8::from(value)); }

    fn emit_u8(&mut self, value: u8) { self.code.push(value); }

    fn emit_u32(&mut self, value: u32) { self.code.extend_from_slice(&value.to_le_bytes()); }

    fn emit_count(&mut self, count: usize, what: &'static str) -> Result<(), CodegenError> {
        let count = u32::try_from(count).map_err(|_| CodegenError::PoolTooLarge(what))?;
        self.emit_u32(count);

        Ok(())
    }

    fn code_offset(&self) -> Result<CodeOffset, CodegenError> {
        Ok(CodeOffset(
            u32::try_from(self.code.len()).map_err(|_| CodegenError::CodeTooLarge)?,
        ))
    }

    fn reserve_address(&mut self) -> usize {
        let operand = self.code.len();
        self.emit_u32(0);

        operand
    }

    fn emit_jump(&mut self, op: Op, target: Label) {
        let instruction = self.code.len();
        self.emit_op(op);
        let address = self.addresses.len();
        self.emit_address(target);
        self.jumps.push(JumpPatch {
            instruction,
            address,
            op,
        });
    }

    fn emit_address(&mut self, target: Label) {
        let operand = self.reserve_address();
        self.addresses.push(AddressPatch { operand, target });
    }

    fn fresh_label(&mut self) -> Label {
        let label = Label::Synthetic(self.next_label);
        self.next_label += 1;
        label
    }

    fn mark_label(&mut self, label: Label) { self.labels.insert(label, self.code.len()); }

    fn write_address(&mut self, operand: usize, target: usize) -> Result<(), CodegenError> {
        let end = operand.checked_add(4).ok_or(CodegenError::CodeTooLarge)?;
        let bytes = self.code.get_mut(operand..end).ok_or(CodegenError::CodeTooLarge)?;
        let target = u32::try_from(target).map_err(|_| CodegenError::CodeTooLarge)?;
        bytes.copy_from_slice(&target.to_le_bytes());

        Ok(())
    }

    fn finish_function(&mut self, start: usize) -> Result<u32, CodegenError> {
        let end = self.code.len();
        debug_assert!(self.labels.values().all(|offset| (start..=end).contains(offset)));

        // Targets stay symbolic until the function has its final layout. This
        // lets compaction move every absolute address, including TryCatch
        // operands, without decoding the variable-width bytecode stream.
        self.redirect_jump_chains();

        let mut removed = vec![false; self.jumps.len()];
        loop {
            let mut changed = false;
            for (index, jump) in self.jumps.iter().enumerate() {
                if removed[index] || jump.op != Op::Jump {
                    continue;
                }
                let target = self.addresses[jump.address].target;
                let target = label_offset(&self.labels, target)?;
                if compact_offset(jump.instruction + 5, &self.jumps, &removed)
                    == compact_offset(target, &self.jumps, &removed)
                {
                    removed[index] = true;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }

        let old = self.code[start..end].to_vec();
        self.code.truncate(start);
        let mut cursor = start;
        for (index, jump) in self.jumps.iter().enumerate() {
            if !removed[index] {
                continue;
            }
            self.code
                .extend_from_slice(&old[cursor - start..jump.instruction - start]);
            cursor = jump.instruction + 5;
        }
        self.code.extend_from_slice(&old[cursor - start..]);

        let labels = std::mem::take(&mut self.labels)
            .into_iter()
            .map(|(label, offset)| (label, compact_offset(offset, &self.jumps, &removed)))
            .collect::<HashMap<_, _>>();
        let removed_addresses = self
            .jumps
            .iter()
            .enumerate()
            .filter_map(|(index, jump)| removed[index].then_some(jump.address))
            .collect::<HashSet<_>>();
        let addresses = std::mem::take(&mut self.addresses);
        for (index, address) in addresses.into_iter().enumerate() {
            if removed_addresses.contains(&index) {
                continue;
            }
            let operand = compact_offset(address.operand, &self.jumps, &removed);
            let target = label_offset(&labels, address.target)?;
            self.write_address(operand, target)?;
        }
        self.jumps.clear();

        u32::try_from(self.code.len() - start).map_err(|_| CodegenError::CodeTooLarge)
    }

    fn redirect_jump_chains(&mut self) {
        let jumps_at = self
            .jumps
            .iter()
            .filter(|jump| jump.op == Op::Jump)
            .map(|jump| (jump.instruction, jump.address))
            .collect::<HashMap<_, _>>();

        for jump in &self.jumps {
            let original = self.addresses[jump.address].target;
            let mut target = original;
            let mut seen = HashSet::new();
            let resolved = loop {
                if !seen.insert(target) {
                    break original;
                }
                let Some(offset) = self.labels.get(&target) else {
                    break target;
                };
                let Some(address) = jumps_at.get(offset) else {
                    break target;
                };
                target = self.addresses[*address].target;
            };
            self.addresses[jump.address].target = resolved;
        }
    }
}

fn compact_offset(offset: usize, jumps: &[JumpPatch], removed: &[bool]) -> usize {
    let removed_bytes = jumps
        .iter()
        .zip(removed)
        .filter(|(jump, removed)| **removed && jump.instruction < offset)
        .count()
        * 5;
    offset - removed_bytes
}

fn label_offset(labels: &HashMap<Label, usize>, label: Label) -> Result<usize, CodegenError> {
    match labels.get(&label).copied() {
        Some(offset) => Ok(offset),
        None => match label {
            Label::Block(block) => Err(CodegenError::ExpectedBlock(block)),
            Label::Synthetic(_) => unreachable!("generated synthetic target should have a label"),
        },
    }
}

fn assign_phi_locals(module: &ir::Module, state: &mut FunctionState) -> Result<(), CodegenError> {
    let phis = state
        .blocks
        .iter()
        .copied()
        .map(|block| {
            module
                .block(block)
                .ok_or(CodegenError::ExpectedBlock(block))
                .map(|instructions| {
                    instructions
                        .iter()
                        .copied()
                        .filter(|instruction| matches!(module.node(*instruction), Some(IrNode::Phi { .. })))
                        .map(|phi| (block, phi))
                        .collect::<Vec<_>>()
                })
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();

    for (target, phi) in phis {
        let Some(IrNode::Phi { operands }) = module.node(phi) else {
            continue;
        };

        let candidates = operands
            .iter()
            .filter(|operand| {
                state
                    .stackify
                    .can_coalesce_phi_source(module, operand.block, target, phi, operand.value)
            })
            .map(|operand| operand.value)
            .collect::<Vec<_>>();

        // parameters have fixed ABI slots, prefer the first eligible fixed slot,
        // otherwise allocate the phi before assigning its edge-dead producers
        let existing = candidates.iter().find_map(|source| state.assigned_local(*source));
        let local = match existing {
            Some(local) => {
                state.assign_local(phi, local);
                local
            },
            None => state.reserve(phi),
        };

        for source in candidates {
            if state.assigned_local(source).is_none() {
                state.assign_local(source, local);
            }
        }
    }

    Ok(())
}

fn phi_copies(
    module: &ir::Module, state: &FunctionState, predecessor: IrNodeId, target: IrNodeId,
) -> Result<Vec<(IrNodeId, LocalId)>, CodegenError> {
    let mut copies = Vec::new();
    for instruction in module.block(target).ok_or(CodegenError::ExpectedBlock(target))? {
        let Some(IrNode::Phi { operands }) = module.node(*instruction) else {
            continue;
        };
        let source = operands
            .iter()
            .find(|operand| operand.block == predecessor)
            .map(|operand| operand.value)
            .ok_or(CodegenError::MissingPhiOperand {
                phi: *instruction,
                predecessor,
            })?;
        let destination = state.local(*instruction)?;
        if state.assigned_local(source) != Some(destination) {
            copies.push((source, destination));
        }
    }

    Ok(copies)
}

fn has_phi_copies(
    module: &ir::Module, state: &FunctionState, predecessor: IrNodeId, target: IrNodeId,
) -> Result<bool, CodegenError> {
    Ok(!phi_copies(module, state, predecessor, target)?.is_empty())
}

fn block_layout(module: &ir::Module, entry: IrNodeId, reachable: &[IrNodeId]) -> Result<Vec<IrNodeId>, CodegenError> {
    let reachable_set = reachable.iter().copied().collect::<HashSet<_>>();
    let mut placed = HashSet::new();
    let mut layout = Vec::with_capacity(reachable.len());

    for seed in std::iter::once(entry).chain(reachable.iter().copied()) {
        let mut block = seed;
        while reachable_set.contains(&block) && placed.insert(block) {
            layout.push(block);
            let instructions = module.block(block).ok_or(CodegenError::ExpectedBlock(block))?;
            let Some(terminator) = instructions.last().and_then(|instruction| module.node(*instruction)) else {
                break;
            };
            block = match terminator {
                IrNode::Branch(target) => *target,
                IrNode::ConditionalBranch { true_block, .. } => *true_block,
                _ => break,
            };
        }
    }

    Ok(layout)
}

fn referenced_nodes(module: &ir::Module, procedures: &HashSet<ProcId>) -> Result<HashSet<IrNodeId>, CodegenError> {
    let mut referenced = HashSet::new();

    for proc_id in procedures {
        let Some(proc) = module.proc(*proc_id) else {
            continue;
        };

        for parameter in &proc.params {
            referenced.extend(parameter.default);
        }

        for block in reachable_blocks(module, proc.body)? {
            for instruction in module.block(block).ok_or(CodegenError::ExpectedBlock(block))? {
                let node = module
                    .node(*instruction)
                    .ok_or(CodegenError::MissingNode(*instruction))?;
                referenced.insert(*instruction);
                node.for_each_operand(|operand| {
                    referenced.insert(operand);
                });
            }
        }
    }

    Ok(referenced)
}

pub(crate) fn reachable_blocks(module: &ir::Module, entry: IrNodeId) -> Result<Vec<IrNodeId>, CodegenError> {
    let mut blocks = Vec::new();
    let mut seen = HashSet::new();
    let mut pending = VecDeque::from([entry]);

    while let Some(block) = pending.pop_front() {
        if !seen.insert(block) {
            continue;
        }

        let instructions = module.block(block).ok_or(CodegenError::ExpectedBlock(block))?;
        blocks.push(block);

        for instruction in instructions {
            match module
                .node(*instruction)
                .ok_or(CodegenError::MissingNode(*instruction))?
            {
                IrNode::Branch(target) => pending.push_back(*target),
                IrNode::ConditionalBranch {
                    true_block,
                    false_block,
                    ..
                } => {
                    pending.push_back(*true_block);
                    pending.push_back(*false_block);
                },
                IrNode::TryCatch { body, catch, .. } => {
                    pending.push_back(*body);
                    pending.push_back(*catch);
                },
                _ => {},
            }
        }
    }

    Ok(blocks)
}

fn produces_value(node: &IrNode) -> bool {
    matches!(
        node,
        IrNode::FunctionParameter(_)
            | IrNode::Phi { .. }
            | IrNode::Load { .. }
            | IrNode::Builtin(_)
            | IrNode::Interpolate { .. }
            | IrNode::Unary { .. }
            | IrNode::Binary { .. }
            | IrNode::CompoundBinary { .. }
            | IrNode::AccessField { .. }
            | IrNode::Initial { .. }
            | IrNode::Index { .. }
            | IrNode::Call { .. }
            | IrNode::FunctionCall { .. }
            | IrNode::Super { .. }
            | IrNode::New { .. }
            | IrNode::ModifiedType { .. }
            | IrNode::List(_)
            | IrNode::Pick(_)
            | IrNode::InRange { .. }
            | IrNode::Range { .. }
            | IrNode::IterInit { .. }
            | IrNode::IterNext(_)
            | IrNode::IterValue(_)
            | IrNode::IterKey(_)
            | IrNode::RangeTest { .. }
            | IrNode::CatchValue
    )
}

fn variable_name(module: &ir::Module, pointer: IrNodeId) -> Result<&Identifier, CodegenError> {
    match module.node(pointer).ok_or(CodegenError::MissingNode(pointer))? {
        IrNode::Variable(name) => Ok(name),
        _ => Err(CodegenError::ExpectedVariable(pointer)),
    }
}

fn unary_code(op: ir::UnaryOp) -> Unary {
    match op {
        ir::UnaryOp::Neg => Unary::Neg,
        ir::UnaryOp::Not => Unary::Not,
        ir::UnaryOp::BitNot => Unary::BitNot,
        ir::UnaryOp::PreIncrement => Unary::PreIncrement,
        ir::UnaryOp::PreDecrement => Unary::PreDecrement,
        ir::UnaryOp::PostIncrement => Unary::PostIncrement,
        ir::UnaryOp::PostDecrement => Unary::PostDecrement,
        ir::UnaryOp::Reference => Unary::Reference,
        ir::UnaryOp::Dereference => Unary::Dereference,
    }
}

fn binary_code(op: ir::BinaryOp) -> Binary {
    match op {
        ir::BinaryOp::Add => Binary::Add,
        ir::BinaryOp::Sub => Binary::Sub,
        ir::BinaryOp::Mul => Binary::Mul,
        ir::BinaryOp::Div => Binary::Div,
        ir::BinaryOp::Mod => Binary::Mod,
        ir::BinaryOp::FloatMod => Binary::FloatMod,
        ir::BinaryOp::Pow => Binary::Pow,
        ir::BinaryOp::BitAnd => Binary::BitAnd,
        ir::BinaryOp::BitXor => Binary::BitXor,
        ir::BinaryOp::BitOr => Binary::BitOr,
        ir::BinaryOp::CompGreater => Binary::CompGreater,
        ir::BinaryOp::CompLess => Binary::CompLess,
        ir::BinaryOp::CompEq => Binary::CompEq,
        ir::BinaryOp::CompNotEq => Binary::CompNotEq,
        ir::BinaryOp::CompGreaterEq => Binary::CompGreaterEq,
        ir::BinaryOp::CompLessEq => Binary::CompLessEq,
        ir::BinaryOp::CompEquiv => Binary::CompEquiv,
        ir::BinaryOp::CompNotEquiv => Binary::CompNotEquiv,
        ir::BinaryOp::CompThreeWay => Binary::CompThreeWay,
        ir::BinaryOp::LogicalAnd => Binary::LogicalAnd,
        ir::BinaryOp::LogicalOr => Binary::LogicalOr,
        ir::BinaryOp::ShiftLeft => Binary::ShiftLeft,
        ir::BinaryOp::ShiftRight => Binary::ShiftRight,
        ir::BinaryOp::In => Binary::In,
    }
}

fn builtin_code(builtin: ir::Builtin) -> Builtin {
    match builtin {
        ir::Builtin::Src => Builtin::Src,
        ir::Builtin::Usr => Builtin::Usr,
        ir::Builtin::World => Builtin::World,
        ir::Builtin::Global => Builtin::Global,
        ir::Builtin::Args => Builtin::Args,
        ir::Builtin::Callee => Builtin::Callee,
        ir::Builtin::Caller => Builtin::Caller,
        ir::Builtin::ThisProc => Builtin::ThisProc,
        ir::Builtin::SuperProc => Builtin::SuperProc,
    }
}

fn access_code(access: ir::AccessKind) -> Access {
    match access {
        ir::AccessKind::Dot => Access::Dot,
        ir::AccessKind::Colon => Access::Colon,
        ir::AccessKind::SafeDot => Access::SafeDot,
        ir::AccessKind::SafeColon => Access::SafeColon,
        ir::AccessKind::Scope => Access::Scope,
    }
}

#[cfg(test)]
mod tests {
    use core::{location::Location, path::TreePath, types::ProcId};

    use super::*;

    fn lower(source: &str) -> ir::Module {
        let (tokens, errors) = lexer::tokenize(source);
        assert!(errors.is_empty());
        let ast = ast::parse(&tokens).expect("fixture should parse");
        let mut builder = ir::IrModuleBuilder::new(&ast);

        for declaration in &ast.declarations {
            let ast::Declaration::Proc {
                path,
                params,
                body,
                variadic,
                location,
                ..
            } = declaration
            else {
                continue;
            };
            builder.lower_proc(
                TreePath::default(),
                path.name().cloned().expect("fixture procedure name"),
                params,
                *variadic,
                body.as_deref().unwrap_or_default(),
                *location,
            );
        }

        builder.finish()
    }

    fn analyze(source: &str) -> (objtree::ObjectTree, ir::Module) {
        let (tokens, errors) = lexer::tokenize(source);
        assert!(errors.is_empty(), "{errors:?}");
        let ast = ast::parse(&tokens).expect("fixture should parse");
        let (tree, module, errors) = sema::analyze(&ast);
        assert!(errors.is_empty(), "{errors:?}");

        (tree, module)
    }

    fn proc_id(tree: &objtree::ObjectTree, owner: &str, name: &str) -> ProcId {
        tree.id_of(&TreePath::parse(owner))
            .and_then(|owner| tree.proc_inherited(owner, &name.into()))
            .and_then(|procedure| procedure.body)
            .expect("fixture procedure should exist")
    }

    fn bytecode_instructions(output: &str) -> Vec<&str> {
        output
            .lines()
            .filter(|line| line.trim_start().starts_with("0x"))
            .map(str::trim)
            .collect::<Vec<_>>()
    }

    #[test]
    fn arithmetic_ssa_values_are_stackified_without_a_temporary_local() {
        let module = generate(&lower("/proc/add(a, b)\n\treturn a + b\n")).expect("generate");
        let output = disasm::dump(&module).expect("disassemble");

        assert_eq!(module.magic, Module::MAGIC);
        assert_eq!(module.version, Module::VERSION);
        assert_eq!(module.functions.len(), 1);
        assert_eq!(module.functions[0].parameter_count, 2);
        assert_eq!(module.functions[0].local_count, 2);
        assert!(output.contains("load_local local0"), "{output}");
        assert!(output.contains("load_local local1"), "{output}");
        assert!(output.contains("binary add"), "{output}");
        assert!(output.contains("return_value"), "{output}");
        assert!(!output.contains("store_local"), "{output}");
    }

    #[test]
    fn named_calls_use_function_table_ids() {
        let module = generate(&lower(
            "/proc/invoke(value)\n\treturn target(value)\n/proc/target(value)\n\treturn value\n",
        ))
        .expect("generate");
        let output = disasm::dump(&module).expect("disassemble");

        assert_eq!(module.functions.len(), 2);
        assert!(output.contains("function_call fn1"), "{output}");
    }

    #[test]
    fn unused_call_results_are_dropped_without_a_local() {
        let module = generate(&lower(
            "/proc/effect()\n\treturn 1\n/proc/test()\n\teffect()\n\treturn 2\n",
        ))
        .expect("generate");
        let output = disasm::dump(&module).expect("disassemble");

        assert_eq!(module.functions[1].local_count, 0);
        assert!(output.contains("function_call fn0 args=[]\n"), "{output}");
        assert!(output.contains(" drop\n"), "{output}");
    }

    #[test]
    fn stackification_preserves_dynamic_evaluation_order() {
        let module = generate(&lower("/proc/test(object, a, b)\n\treturn object.run(a + b)\n")).expect("generate");
        let output = disasm::dump(&module).expect("disassemble");

        let object = output.find("load_local local0").expect("object load");
        let field = output.find("access_field .run method").expect("method lookup");
        let lhs = output.find("load_local local1").expect("left argument");
        let rhs = output.find("load_local local2").expect("right argument");
        let add = output.find("binary add").expect("argument expression");
        let call = output.find("call args=[value]").expect("dynamic call");

        assert!(
            object < field && field < lhs && lhs < rhs && rhs < add && add < call,
            "{output}"
        );
        assert!(!output.contains("store_local"), "{output}");
    }

    #[test]
    fn coalesces_edge_dead_loop_updates_into_the_phi_local() {
        let module = generate(&lower(
            "/proc/count(n)\n\tvar/i = 0\n\twhile(i < n)\n\t\ti++\n\treturn i\n",
        ))
        .expect("generate");
        let output = disasm::dump(&module).expect("disassemble");

        assert_eq!(module.functions[0].local_count, 2, "{output}");
        assert!(!output.contains("local2"), "{output}");
        assert_eq!(output.matches("store_local local1").count(), 2, "{output}");
    }

    #[test]
    fn coalesces_distinct_edge_dead_sources_into_one_phi_local() {
        let module = generate(&lower(
            r#"
/proc/select_and_increment(condition, a, b)
    var/value
    if(condition)
        value = a + 1
    else
        value = b + 1
    return value
"#,
        ))
        .expect("generate");
        let output = disasm::dump(&module).expect("disassemble");

        assert_eq!(module.functions[0].local_count, 4, "{output}");
        assert!(!output.contains("local4"), "{output}");
        assert_eq!(output.matches("store_local local3").count(), 2, "{output}");
    }

    #[test]
    fn keeps_a_phi_copy_when_the_old_value_is_used_after_the_source() {
        let module = generate(&lower(
            r#"
/proc/test(n)
    var/i = 0
    while(i < n)
        var/next = i + 1
        world.log << i
        i = next
    return i
"#,
        ))
        .expect("generate");
        let output = disasm::dump(&module).expect("disassemble");

        assert_eq!(module.functions[0].local_count, 3, "{output}");
        assert!(output.contains("store_local local2"), "{output}");
        assert!(output.contains("load_local local2"), "{output}");
    }

    #[test]
    fn keeps_a_phi_copy_when_the_source_has_another_user() {
        let module = generate(&lower(
            r#"
/proc/test(n)
    var/i = 0
    while(i < n)
        var/next = i + 1
        world.log << next
        i = next
    return i
"#,
        ))
        .expect("generate");
        let output = disasm::dump(&module).expect("disassemble");

        assert_eq!(module.functions[0].local_count, 3, "{output}");
        assert!(output.contains("store_local local2"), "{output}");
        assert!(output.contains("load_local local2"), "{output}");
    }

    #[test]
    fn a_coalesced_false_edge_needs_no_copy_stub() {
        let module = generate(&lower(
            r#"
/proc/test(condition, input)
    var/value = input
    if(condition)
        value = 2
    return value
"#,
        ))
        .expect("generate");
        let output = disasm::dump(&module).expect("disassemble");
        let instructions = bytecode_instructions(&output);

        assert_eq!(module.functions[0].local_count, 2, "{output}");
        assert_eq!(
            instructions
                .iter()
                .filter(|instruction| instruction.contains(" jump "))
                .count(),
            0,
            "{output}"
        );
        assert_eq!(output.matches("store_local local1").count(), 1, "{output}");
    }

    #[test]
    fn conditional_true_successor_falls_through() {
        let module = generate(&lower(
            "/proc/test(condition)\n\tif(condition)\n\t\treturn 1\n\treturn 2\n",
        ))
        .expect("generate");
        let output = disasm::dump(&module).expect("disassemble");
        let instructions = bytecode_instructions(&output);

        assert_eq!(
            instructions
                .iter()
                .filter(|instruction| instruction.contains(" jump "))
                .count(),
            0,
            "{output}"
        );
        assert_eq!(
            instructions
                .iter()
                .filter(|instruction| instruction.contains(" jump_if_false "))
                .count(),
            1,
            "{output}"
        );
    }

    #[test]
    fn false_edge_phi_copies_stay_on_the_false_path() {
        let ir = lower("/proc/test(condition)\n\tvar/value = 2\n\tif(condition)\n\t\tvalue = 1\n\treturn value\n");
        let false_block = ir
            .nodes
            .iter()
            .find_map(|node| match node {
                IrNode::ConditionalBranch { false_block, .. } => Some(*false_block),
                _ => None,
            })
            .expect("conditional branch");
        assert!(
            ir.block(false_block)
                .expect("false block")
                .iter()
                .any(|instruction| matches!(ir.node(*instruction), Some(IrNode::Phi { .. })))
        );

        let module = generate(&ir).expect("generate");
        let output = disasm::dump(&module).expect("disassemble");
        let instructions = bytecode_instructions(&output);
        let branch = instructions
            .iter()
            .position(|instruction| instruction.contains(" jump_if_false "))
            .expect("conditional jump");
        let target = instructions[branch]
            .split_whitespace()
            .last()
            .expect("conditional target");
        let stub = instructions
            .iter()
            .position(|instruction| instruction.starts_with(target))
            .expect("false-edge stub");
        let return_value = instructions
            .iter()
            .position(|instruction| instruction.ends_with("return_value"))
            .expect("return");

        assert!(stub > return_value, "{output}");
        assert!(
            instructions[stub..]
                .iter()
                .any(|instruction| instruction.contains(" jump ")),
            "{output}"
        );
    }

    #[test]
    fn bytecode_cleanup_threads_chains_and_removes_fallthrough_jumps() {
        let mut generator = Generator::new();
        let entry = Label::Block(IrNodeId(0));
        let forwarding = Label::Block(IrNodeId(1));
        let destination = Label::Block(IrNodeId(2));

        generator.mark_label(entry);
        generator.emit_jump(Op::Jump, forwarding);
        generator.mark_label(forwarding);
        generator.emit_jump(Op::Jump, destination);
        generator.mark_label(destination);
        generator.emit_op(Op::Return);

        let length = generator.finish_function(0).expect("finish function");

        assert_eq!(length, 1);
        assert_eq!(generator.code, [Op::Return as u8]);
    }

    #[test]
    fn bytecode_cleanup_preserves_jump_cycles() {
        let mut generator = Generator::new();
        let first = Label::Block(IrNodeId(0));
        let second = Label::Block(IrNodeId(1));

        generator.mark_label(first);
        generator.emit_jump(Op::Jump, second);
        generator.mark_label(second);
        generator.emit_jump(Op::Jump, first);

        let length = generator.finish_function(0).expect("finish function");

        assert_eq!(length, 5);
        assert_eq!(generator.code[0], Op::Jump as u8);
        assert_eq!(u32::from_le_bytes(generator.code[1..5].try_into().unwrap()), 0);
    }

    #[test]
    fn compacted_functions_keep_contiguous_ranges() {
        let module = generate(&lower(
            r#"
/proc/first(condition)
    if(condition)
        return 1
    return 2
/proc/second(condition)
    if(condition)
        return 3
    return 4
"#,
        ))
        .expect("generate");

        assert_eq!(module.functions.len(), 2);
        assert_eq!(
            module.functions[0].address.0 + module.functions[0].length,
            module.functions[1].address.0
        );
        assert_eq!(
            module.functions[1].address.0 + module.functions[1].length,
            module.code.len() as u32
        );
        disasm::dump(&module).expect("compacted functions should disassemble");
    }

    #[test]
    fn unsupported_source_constructs_become_traps() {
        let module = generate(&lower("/proc/invalid()\n\tbreak\n")).expect("generate");
        let output = disasm::dump(&module).expect("disassemble");

        assert!(output.contains("trap break outside loop"), "{output}");
    }

    #[test]
    fn output_preserves_field_reference_shape() {
        let module = generate(&lower("/proc/test()\n\tworld.log << \"hello\"\n")).expect("generate");
        let output = disasm::dump(&module).expect("disassemble");

        assert!(output.contains("output field .log"), "{output}");
        assert!(!output.contains("access_field .log"), "{output}");
    }

    #[test]
    fn try_inside_a_loop_does_not_leave_orphaned_ssa_loads() {
        let module = lower(
            r#"
/proc/test(values)
    var/total = 0
    for(var/value in values)
        try
            total += value
        catch
            total = -1
    return total
"#,
        );

        generate(&module).expect("generate");
    }

    #[test]
    fn phi_cycles_load_every_source_before_storing_any_destination() {
        let module = ir::Module {
            nodes: vec![
                IrNode::Function(ProcId(0)),
                IrNode::FunctionParameter(0),
                IrNode::FunctionParameter(1),
                IrNode::Label(vec![IrNodeId(6)]),
                IrNode::Label(vec![IrNodeId(7), IrNodeId(8), IrNodeId(9)]),
                IrNode::Noop,
                IrNode::Branch(IrNodeId(4)),
                IrNode::Phi {
                    operands: vec![
                        ir::PhiOperand {
                            block: IrNodeId(3),
                            value: IrNodeId(1),
                        },
                        ir::PhiOperand {
                            block: IrNodeId(4),
                            value: IrNodeId(8),
                        },
                    ],
                },
                IrNode::Phi {
                    operands: vec![
                        ir::PhiOperand {
                            block: IrNodeId(3),
                            value: IrNodeId(2),
                        },
                        ir::PhiOperand {
                            block: IrNodeId(4),
                            value: IrNodeId(7),
                        },
                    ],
                },
                IrNode::Branch(IrNodeId(4)),
            ],
            constants: Vec::new(),
            external_functions: Vec::new(),
            procs: vec![ir::Procedure {
                function: IrNodeId(0),
                parameters: vec![IrNodeId(1), IrNodeId(2)],
                previous: None,
                owner: TreePath::default(),
                name: "swap".into(),
                params: Vec::new(),
                variadic: false,
                vars: Vec::new(),
                body: IrNodeId(3),
                intrinsic: None,
                location: Location::default(),
            }],
        };
        let module = generate(&module).expect("generate");
        let output = disasm::dump(&module).expect("disassemble");
        let instructions = bytecode_instructions(&output)
            .into_iter()
            .map(|line| line.split_whitespace().skip(1).collect::<Vec<_>>().join(" "))
            .collect::<Vec<_>>();

        assert_eq!(
            &instructions[..5],
            [
                "load_local local1",
                "load_local local0",
                "store_local local1",
                "store_local local0",
                "jump 0x00000000",
            ]
        );
    }

    #[test]
    fn reachable_codegen_keeps_named_dispatch_candidates_and_stable_proc_ids() {
        let (tree, ir_module) = analyze(
            r#"
/datum/base
    proc/live()
        return 1
    proc/dead()
        return 2
/datum/base/child/live()
    return 3
/proc/entry(datum/base/value)
    return value.live()
/proc/unused()
    return 4
"#,
        );
        let entry = proc_id(&tree, "/", "entry");
        let base_live = proc_id(&tree, "/datum/base", "live");
        let child_live = proc_id(&tree, "/datum/base/child", "live");
        let dead = proc_id(&tree, "/datum/base", "dead");
        let unused = proc_id(&tree, "/", "unused");

        let module = generate_reachable(&ir_module, &tree, &[entry]).expect("generate selected procedures");

        for retained in [entry, base_live, child_live] {
            assert_eq!(
                module.function_for_proc(retained).and_then(|function| function.proc),
                Some(retained)
            );
        }
        assert!(module.function_for_proc(dead).is_none());
        assert!(module.function_for_proc(unused).is_none());
        assert_eq!(module.functions.len(), 3);
        assert!(!module.constants.contains(&Value::Num(2.0)));
        assert!(!module.constants.contains(&Value::Num(4.0)));
    }

    #[test]
    fn reachable_codegen_does_not_treat_field_reads_as_method_calls() {
        let (tree, ir_module) = analyze(
            r#"
/datum/base/proc/status()
    return 1
/proc/entry(datum/base/value)
    return value.status
"#,
        );
        let entry = proc_id(&tree, "/", "entry");
        let status = proc_id(&tree, "/datum/base", "status");

        let module = generate_reachable(&ir_module, &tree, &[entry]).expect("generate selected procedures");

        assert!(module.function_for_proc(entry).is_some());
        assert!(module.function_for_proc(status).is_none());
    }

    #[test]
    fn reachable_codegen_limits_typed_receiver_dispatch_to_its_type_family() {
        let (tree, ir_module) = analyze(
            r#"
/datum/wanted/proc/Initialize()
    return 1
/datum/wanted/child/Initialize()
    return 2
/datum/unrelated/proc/Initialize()
    return 3
/proc/entry(values)
    for(var/datum/wanted/value in values)
        value.Initialize()
"#,
        );
        let entry = proc_id(&tree, "/", "entry");
        let wanted = proc_id(&tree, "/datum/wanted", "Initialize");
        let child = proc_id(&tree, "/datum/wanted/child", "Initialize");
        let unrelated = proc_id(&tree, "/datum/unrelated", "Initialize");

        let module = generate_reachable(&ir_module, &tree, &[entry]).expect("generate selected procedures");

        for retained in [entry, wanted, child] {
            assert!(
                module.function_for_proc(retained).is_some(),
                "missing retained procedure {retained}"
            );
        }
        assert!(module.function_for_proc(unrelated).is_none());
    }

    #[test]
    fn reachable_codegen_follows_super_and_roots_lazy_initializers() {
        let (tree, ir_module) = analyze(
            r#"
/datum/base/proc/step()
    return 1
/datum/base/child/step()
    return ..()
/datum/base/child/step()
    return ..()
/datum/holder
    var/value = initialize()
/proc/initialize()
    return 3
/proc/entry()
    var/datum/base/child/value = new
    var/datum/holder/holder = new
    return value.step() + holder.value
/proc/unused()
    return 4
"#,
        );
        let entry = proc_id(&tree, "/", "entry");
        let latest = proc_id(&tree, "/datum/base/child", "step");
        let previous = ir_module
            .proc(latest)
            .and_then(|procedure| procedure.previous)
            .expect("same-type previous");
        let inherited = proc_id(&tree, "/datum/base", "step");
        let initializer = tree
            .id_of(&TreePath::parse("/datum/holder"))
            .and_then(|holder| tree.var_inherited(holder, &"value".into()))
            .and_then(|variable| variable.initializer)
            .expect("runtime initializer");
        let initialize = proc_id(&tree, "/", "initialize");
        let unused = proc_id(&tree, "/", "unused");

        let module = generate_reachable(&ir_module, &tree, &[entry]).expect("generate selected procedures");

        for retained in [entry, latest, previous, inherited, initializer, initialize] {
            assert!(
                module.function_for_proc(retained).is_some(),
                "missing retained procedure {retained}"
            );
        }
        assert!(module.function_for_proc(unused).is_none());
    }

    #[test]
    fn reachable_codegen_omits_unread_lazy_initializers() {
        let (tree, ir_module) = analyze(
            r#"
/datum/holder
    var/value = initialize()
/proc/initialize()
    return 1
/proc/entry()
    return 2
"#,
        );
        let entry = proc_id(&tree, "/", "entry");
        let initialize = proc_id(&tree, "/", "initialize");
        let initializer = tree
            .id_of(&TreePath::parse("/datum/holder"))
            .and_then(|holder| tree.var_inherited(holder, &"value".into()))
            .and_then(|variable| variable.initializer)
            .expect("runtime initializer");

        let module = generate_reachable(&ir_module, &tree, &[entry]).expect("generate selected procedures");

        assert!(module.function_for_proc(entry).is_some());
        assert!(module.function_for_proc(initializer).is_none());
        assert!(module.function_for_proc(initialize).is_none());
    }

    #[test]
    fn reachable_codegen_bounds_opaque_calls_to_reachable_proc_paths() {
        let (tree, ir_module) = analyze(
            r#"
/proc/call(Target, ProcName)
    set __demir_intrin = 401
/proc/live()
    return 1
/proc/dead()
    return 2
/proc/invoke(callback)
    return call(callback)()
/proc/entry(callback = /proc/live)
    return invoke(callback)
"#,
        );
        let entry = proc_id(&tree, "/", "entry");
        let invoke = proc_id(&tree, "/", "invoke");
        let call = proc_id(&tree, "/", "call");
        let live = proc_id(&tree, "/", "live");
        let dead = proc_id(&tree, "/", "dead");

        let module = generate_reachable(&ir_module, &tree, &[entry]).expect("generate selected procedures");

        for retained in [entry, invoke, call, live] {
            assert!(
                module.function_for_proc(retained).is_some(),
                "missing retained procedure {retained}"
            );
        }
        assert!(module.function_for_proc(dead).is_none());
    }

    #[test]
    fn reachable_codegen_resolves_static_call_names() {
        let (tree, ir_module) = analyze(
            r#"
/proc/call(Target, ProcName)
    set __demir_intrin = 401
/datum/base/proc/live()
    return 1
/datum/base/proc/dead()
    return 2
/proc/entry()
    var/datum/base/value = new /datum/base
    return call(value, "live")()
"#,
        );
        let entry = proc_id(&tree, "/", "entry");
        let call = proc_id(&tree, "/", "call");
        let live = proc_id(&tree, "/datum/base", "live");
        let dead = proc_id(&tree, "/datum/base", "dead");

        let module = generate_reachable(&ir_module, &tree, &[entry]).expect("generate selected procedures");

        for retained in [entry, call, live] {
            assert!(
                module.function_for_proc(retained).is_some(),
                "missing retained procedure {retained}"
            );
        }
        assert!(module.function_for_proc(dead).is_none());
    }

    #[test]
    fn reachable_codegen_keeps_only_exact_constructor_targets() {
        let (tree, ir_module) = analyze(
            r#"
/datum/live/New()
    return live_helper()
/datum/dead/New()
    return dead_helper()
/proc/live_helper()
    return 1
/proc/dead_helper()
    return 2
/proc/entry()
    return new /datum/live
"#,
        );
        let entry = proc_id(&tree, "/", "entry");
        let live_new = proc_id(&tree, "/datum/live", "New");
        let dead_new = proc_id(&tree, "/datum/dead", "New");
        let live_helper = proc_id(&tree, "/", "live_helper");
        let dead_helper = proc_id(&tree, "/", "dead_helper");

        let module = generate_reachable(&ir_module, &tree, &[entry]).expect("generate selected procedures");

        for retained in [entry, live_new, live_helper] {
            assert!(
                module.function_for_proc(retained).is_some(),
                "missing retained procedure {retained}"
            );
        }
        assert!(module.function_for_proc(dead_new).is_none());
        assert!(module.function_for_proc(dead_helper).is_none());
    }

    #[test]
    fn reachable_codegen_keeps_every_constructor_for_a_computed_type() {
        let (tree, ir_module) = analyze(
            r#"
/datum/live/New()
    return 1
/datum/dead/New()
    return 2
/proc/entry(kind)
    return new kind
/proc/unrelated()
    return 3
"#,
        );
        let entry = proc_id(&tree, "/", "entry");
        let live_new = proc_id(&tree, "/datum/live", "New");
        let dead_new = proc_id(&tree, "/datum/dead", "New");
        let unrelated = proc_id(&tree, "/", "unrelated");

        let module = generate_reachable(&ir_module, &tree, &[entry]).expect("generate selected procedures");

        for retained in [entry, live_new, dead_new] {
            assert!(
                module.function_for_proc(retained).is_some(),
                "missing retained procedure {retained}"
            );
        }
        assert!(module.function_for_proc(unrelated).is_none());
    }

    #[test]
    fn reachable_codegen_falls_back_when_call_target_might_be_a_proc_path() {
        let (tree, ir_module) = analyze(
            r#"
/proc/call(Target, ProcName)
    set __demir_intrin = 401
/datum/base/proc/live()
    return 1
/datum/base/proc/dead()
    return 2
/proc/entry(target)
    return call(target, "live")()
/proc/unrelated()
    return 3
"#,
        );
        let entry = proc_id(&tree, "/", "entry");

        let module = generate_reachable(&ir_module, &tree, &[entry]).expect("generate selected procedures");

        assert_eq!(
            module
                .functions
                .iter()
                .filter(|function| function.proc.is_some())
                .count(),
            ir_module.procs.len()
        );
    }

    #[test]
    fn reachable_codegen_resolves_dynamic_values_from_typesof_iteration() {
        let (tree, ir_module) = analyze(
            r#"
/proc/call(Target, ProcName)
    set __demir_intrin = 401
/proc/typesof(Type1, Type2)
    set __demir_intrin = 377
/datum/controller/global_vars/proc/Initialize()
    var/list/global_procs = typesof(/datum/controller/global_vars/proc)
    for(var/proc_path in global_procs)
        call(src, proc_path)()
/proc/unrelated()
    return 1
"#,
        );
        let initialize = proc_id(&tree, "/datum/controller/global_vars", "Initialize");
        let call = proc_id(&tree, "/", "call");
        let typesof = proc_id(&tree, "/", "typesof");
        let unrelated = proc_id(&tree, "/", "unrelated");

        let module = generate_reachable(&ir_module, &tree, &[initialize]).expect("generate selected procedures");

        for retained in [initialize, call, typesof] {
            assert!(
                module.function_for_proc(retained).is_some(),
                "missing retained procedure {retained}"
            );
        }
        assert!(module.function_for_proc(unrelated).is_none());
    }

    #[test]
    fn reachable_codegen_falls_back_to_every_proc_for_an_opaque_callable() {
        let (tree, ir_module) = analyze(
            r#"
/proc/entry(callback)
    return (callback)()
/proc/otherwise()
    return 1
"#,
        );
        let entry = proc_id(&tree, "/", "entry");
        let otherwise = proc_id(&tree, "/", "otherwise");

        let module = generate_reachable(&ir_module, &tree, &[entry]).expect("generate selected procedures");

        assert!(module.function_for_proc(entry).is_some());
        assert!(module.function_for_proc(otherwise).is_some());
        assert_eq!(
            module
                .functions
                .iter()
                .filter(|function| function.proc.is_some())
                .count(),
            ir_module.procs.len()
        );
    }

    #[test]
    fn opcode_numbers_are_append_only() {
        assert_eq!(Op::Nop as u8, 0x00);
        assert_eq!(Op::Trap as u8, 0x25);
        assert_eq!(Op::DefaultParameter as u8, 0x29);
        assert_eq!(Op::CompoundBinary as u8, 0x2a);
        assert_eq!(Op::Initial as u8, 0x2b);
        assert_eq!(Op::Drop as u8, 0x2c);
        assert_eq!(Unary::Dereference as u8, 0x08);
        assert_eq!(Binary::In as u8, 0x17);
        assert_eq!(Builtin::SuperProc as u8, 0x08);
        assert_eq!(Access::Scope as u8, 0x04);
    }
}
