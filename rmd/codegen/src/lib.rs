pub mod disasm;
pub mod error;
pub mod opcode;

use core::{
    location::Location,
    path::TreePath,
    types::{Identifier, IrNodeId, ProcId, Value},
};
use std::collections::{HashMap, HashSet, VecDeque};

pub use error::CodegenError;
use ir::{Argument, IrNode, Procedure};
use opcode::{ARGUMENT_KEY, ARGUMENT_VALUE, Access, Binary, Builtin, Op, Unary};

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
    pub paths: Vec<TreePath>,
    pub functions: Vec<CompiledFunction>,
    pub code: Vec<u8>,
}

impl Module {
    pub const MAGIC: [u8; 4] = *b"RMBC";
    pub const VERSION: u16 = 1;

    pub fn function(&self, id: FunctionId) -> Option<&CompiledFunction> { self.functions.get(id.0 as usize) }

    pub fn function_for_proc(&self, proc: ProcId) -> Option<&CompiledFunction> {
        self.functions.iter().find(|function| function.proc == Some(proc))
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
    pub location: Location,
}

pub fn generate(module: &ir::Module) -> Result<Module, CodegenError> { Generator::new().generate(module) }

#[derive(Debug)]
struct JumpPatch {
    operand: usize,
    target: IrNodeId,
}

struct FunctionState {
    blocks: Vec<IrNodeId>,
    locals: HashMap<IrNodeId, LocalId>,
    parameter_defaults: HashMap<IrNodeId, LocalId>,
    next_local: u32,
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
    labels: HashMap<IrNodeId, CodeOffset>,
    jumps: Vec<JumpPatch>,
    code: Vec<u8>,
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
            jumps: Vec::new(),
            code: Vec::new(),
        }
    }

    fn generate(mut self, module: &ir::Module) -> Result<Module, CodegenError> {
        self.register_constants(module)?;
        self.register_functions(module)?;

        for proc in &module.procs {
            self.generate_function(module, proc)?;
        }

        self.resolve_jumps()?;

        Ok(Module {
            magic: Module::MAGIC,
            version: Module::VERSION,
            constants: self.constants,
            strings: self.strings,
            paths: self.paths,
            functions: self.functions,
            code: self.code,
        })
    }

    fn register_constants(&mut self, module: &ir::Module) -> Result<(), CodegenError> {
        for node in &module.constants {
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

    fn register_functions(&mut self, module: &ir::Module) -> Result<(), CodegenError> {
        for (index, proc) in module.procs.iter().enumerate() {
            let id = FunctionId(u32::try_from(index).map_err(|_| CodegenError::PoolTooLarge("function"))?);
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
                proc: Some(ProcId(index as u32)),
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
                location: proc.location,
            });
        }

        for node in &module.external_functions {
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
                location: Location::default(),
            });
        }

        Ok(())
    }

    fn generate_function(&mut self, module: &ir::Module, proc: &Procedure) -> Result<(), CodegenError> {
        let blocks = reachable_blocks(module, proc.body)?;
        let mut state = FunctionState {
            blocks,
            locals: HashMap::new(),
            parameter_defaults: HashMap::new(),
            next_local: 0,
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

        for block in state.blocks.clone() {
            for instruction in module.block(block).ok_or(CodegenError::ExpectedBlock(block))? {
                if matches!(module.node(*instruction), Some(IrNode::Phi { .. })) {
                    state.reserve(*instruction);
                }
            }
        }

        for block in state.blocks.clone() {
            for instruction in module.block(block).ok_or(CodegenError::ExpectedBlock(block))? {
                if module.node(*instruction).is_some_and(produces_value) {
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
        for block in state.blocks.clone() {
            self.generate_block(module, block, &state)?;
        }

        let length = self
            .code_offset()?
            .0
            .checked_sub(address.0)
            .ok_or(CodegenError::CodeTooLarge)?;
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
        &mut self, module: &ir::Module, block: IrNodeId, state: &FunctionState,
    ) -> Result<(), CodegenError> {
        self.labels.insert(block, self.code_offset()?);
        let instructions = module.block(block).ok_or(CodegenError::ExpectedBlock(block))?;

        for instruction in instructions {
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
                    self.emit_jump(Op::Jump, *target);
                },
                IrNode::ConditionalBranch {
                    condition,
                    true_block,
                    false_block,
                } => {
                    self.emit_value(module, state, *condition)?;
                    self.emit_op(Op::JumpIfFalse);
                    let false_operand = self.reserve_address();

                    self.emit_phi_copies(module, state, block, *true_block)?;
                    self.emit_jump(Op::Jump, *true_block);

                    let false_offset = self.code_offset()?;
                    self.write_address(false_operand, false_offset)?;
                    self.emit_phi_copies(module, state, block, *false_block)?;
                    self.emit_jump(Op::Jump, *false_block);
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
            IrNode::AccessField { object, name, access } => {
                self.emit_value(module, state, *object)?;
                let name = self.intern_identifier(name)?;
                self.emit_op(Op::AccessField);
                self.emit_u32(name.0);
                self.emit_u8(access_code(*access) as u8);
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
                self.emit_op(Op::Call);
                self.emit_argument_layout(&shapes)?;
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
            IrNode::Super { args } => {
                let shapes = self.emit_arguments(module, state, args)?;
                self.emit_op(Op::SuperCall);
                self.emit_argument_layout(&shapes)?;
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
            IrNode::IterInit { list, ty } => {
                self.emit_value(module, state, *list)?;
                let ty = ty.clone().map(|ty| self.intern_path(ty)).transpose()?;
                self.emit_op(Op::IterInit);
                self.emit_optional_path(ty);
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
            IrNode::SetIndex { object, index, value } => {
                self.emit_value(module, state, *object)?;
                self.emit_value(module, state, *index)?;
                self.emit_value(module, state, *value)?;
                self.emit_op(Op::SetIndex);
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
                self.emit_value(module, state, *target)?;
                self.emit_value(module, state, *value)?;
                self.emit_op(Op::Output);
            },
            IrNode::TryCatch { body, catch } => {
                self.emit_op(Op::TryCatch);
                self.emit_block_address(*body);
                self.emit_block_address(*catch);
            },
            IrNode::Blocked(reason) => {
                let reason = self.intern_string((*reason).to_owned())?;
                self.emit_op(Op::Blocked);
                self.emit_u32(reason.0);
                self.store_result(state, id)?;
            },
            IrNode::Trap { reason } => {
                let reason = self.intern_string(reason.clone())?;
                self.emit_op(Op::Trap);
                self.emit_u32(reason.0);
                self.store_result(state, id)?;
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
            copies.push((source, state.local(*instruction)?));
        }

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

    fn emit_jump(&mut self, op: Op, target: IrNodeId) {
        self.emit_op(op);
        self.emit_block_address(target);
    }

    fn emit_block_address(&mut self, target: IrNodeId) {
        let operand = self.reserve_address();
        self.jumps.push(JumpPatch { operand, target });
    }

    fn write_address(&mut self, operand: usize, target: CodeOffset) -> Result<(), CodegenError> {
        let end = operand.checked_add(4).ok_or(CodegenError::CodeTooLarge)?;
        let bytes = self.code.get_mut(operand..end).ok_or(CodegenError::CodeTooLarge)?;
        bytes.copy_from_slice(&target.0.to_le_bytes());

        Ok(())
    }

    fn resolve_jumps(&mut self) -> Result<(), CodegenError> {
        for jump in std::mem::take(&mut self.jumps) {
            let target = self
                .labels
                .get(&jump.target)
                .copied()
                .ok_or(CodegenError::ExpectedBlock(jump.target))?;
            self.write_address(jump.operand, target)?;
        }

        Ok(())
    }
}

fn reachable_blocks(module: &ir::Module, entry: IrNodeId) -> Result<Vec<IrNodeId>, CodegenError> {
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
            | IrNode::AccessField { .. }
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
            | IrNode::Blocked(_)
            | IrNode::Trap { .. }
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

    #[test]
    fn arithmetic_ssa_values_become_stack_operations_and_locals() {
        let module = generate(&lower("/proc/add(a, b)\n\treturn a + b\n")).expect("generate");
        let output = disasm::dump(&module).expect("disassemble");

        assert_eq!(module.magic, Module::MAGIC);
        assert_eq!(module.version, Module::VERSION);
        assert_eq!(module.functions.len(), 1);
        assert_eq!(module.functions[0].parameter_count, 2);
        assert_eq!(module.functions[0].local_count, 3);
        assert!(output.contains("load_local local0"), "{output}");
        assert!(output.contains("load_local local1"), "{output}");
        assert!(output.contains("binary add"), "{output}");
        assert!(output.contains("return_value"), "{output}");
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
    fn unsupported_source_constructs_become_traps() {
        let module = generate(&lower("/proc/invalid()\n\tbreak\n")).expect("generate");
        let output = disasm::dump(&module).expect("disassemble");

        assert!(output.contains("trap break outside loop"), "{output}");
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
                location: Location::default(),
            }],
        };
        let module = generate(&module).expect("generate");
        let output = disasm::dump(&module).expect("disassemble");
        let instructions = output
            .lines()
            .filter(|line| line.trim_start().starts_with("0x"))
            .map(|line| line.split_whitespace().skip(1).collect::<Vec<_>>().join(" "))
            .collect::<Vec<_>>();

        assert_eq!(
            &instructions[5..10],
            [
                "load_local local3",
                "load_local local2",
                "store_local local3",
                "store_local local2",
                "jump 0x00000019",
            ]
        );
    }

    #[test]
    fn opcode_numbers_are_append_only() {
        assert_eq!(Op::Nop as u8, 0x00);
        assert_eq!(Op::Trap as u8, 0x25);
        assert_eq!(Unary::Dereference as u8, 0x08);
        assert_eq!(Binary::In as u8, 0x17);
        assert_eq!(Builtin::SuperProc as u8, 0x08);
        assert_eq!(Access::Scope as u8, 0x04);
    }
}
