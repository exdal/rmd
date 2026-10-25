pub mod disasm;
pub mod error;
pub mod opcode;
mod reachability;
mod registers;

use core::{
    location::{LineTable, Location, SourceLine},
    path::TreePath,
    types::{Identifier, IrNodeId, ProcId, Value},
};
use std::collections::VecDeque;

pub use error::CodegenError;
pub use ir::ProcIndex;
use ir::{Argument, IrNode, OutputTarget, Procedure};
use objtree::ObjectTree;
use opcode::{Access, Binary, Builtin, Op, Unary};
use prelude::Intrinsic;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::opcode::{ARGUMENT_KEY, ARGUMENT_VALUE, OutputTargetKind};

macro_rules! id_type {
    ($name:ident, $repr:ty, $prefix:literal) => {
        #[derive(Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $name(pub $repr);

        impl $name {
            pub const INVALID: Self = Self(<$repr>::MAX);

            pub const fn is_valid(self) -> bool {
                self.0 != <$repr>::MAX
            }

            pub const fn is_invalid(self) -> bool {
                self.0 == <$repr>::MAX
            }
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

id_type!(FunctionId, u32, "fn");
id_type!(RegisterId, u16, "r");
id_type!(ConstantId, u32, "const");
id_type!(StringId, u32, "str");
id_type!(PathId, u32, "path");

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CodeOffset(pub u32);

impl CodeOffset {
    pub const INVALID: Self = Self(u32::MAX);

    pub const fn is_valid(self) -> bool {
        self.0 != u32::MAX
    }

    pub const fn is_invalid(self) -> bool {
        self.0 == u32::MAX
    }
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
    pub lines: LineTable<CodeOffset>,
    proc_functions: Vec<FunctionId>,
    pub index: ProcIndex,
}

impl Module {
    pub const MAGIC: [u8; 4] = *b"DMIR";
    pub const VERSION: u16 = 3;

    pub fn function(&self, id: FunctionId) -> Option<&CompiledFunction> {
        self.functions.get(id.0 as usize)
    }

    pub fn line_at(&self, offset: CodeOffset) -> Option<SourceLine> {
        self.lines.get(offset)
    }

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
    pub register_count: u16,
    pub external: bool,
    pub intrinsic: Option<Intrinsic>,
    pub location: Location,
}

#[derive(Debug, Clone)]
pub enum ProcedureReachability {
    All,
    Selected(FxHashSet<ProcId>),
}

pub fn generate(module: &ir::Module) -> Result<Module, CodegenError> {
    Generator::new().generate(module, None)
}

/// `read_vars` names the vars the host reads off objects itself, which no reachable procedure shows
pub fn reachable_procedures(
    module: &ir::Module, tree: &ObjectTree, roots: &[ProcId], read_vars: &[Identifier],
) -> Result<ProcedureReachability, CodegenError> {
    reachability::reachable_procedures(module, tree, roots, read_vars)
}

pub fn generate_reachable(
    module: &ir::Module, tree: &ObjectTree, roots: &[ProcId], read_vars: &[Identifier],
) -> Result<Module, CodegenError> {
    match reachable_procedures(module, tree, roots, read_vars)? {
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
    registers: FxHashMap<IrNodeId, RegisterId>,
    parameter_defaults: FxHashMap<IrNodeId, RegisterId>,
    phi_temporaries: FxHashMap<IrNodeId, RegisterId>,
}

impl FunctionState {
    fn register(&self, node: IrNodeId) -> Result<RegisterId, CodegenError> {
        self.registers
            .get(&node)
            .copied()
            .ok_or(CodegenError::MissingRegister(node))
    }
}

struct Generator {
    constants: Vec<Value>,
    constant_ids: FxHashMap<IrNodeId, ConstantId>,
    strings: Vec<String>,
    string_ids: FxHashMap<String, StringId>,
    paths: Vec<TreePath>,
    path_ids: FxHashMap<TreePath, PathId>,
    functions: Vec<CompiledFunction>,
    function_ids: FxHashMap<IrNodeId, FunctionId>,
    labels: FxHashMap<Label, usize>,
    addresses: Vec<AddressPatch>,
    jumps: Vec<JumpPatch>,
    next_label: usize,
    code: Vec<u8>,
    lines: LineTable<CodeOffset>,
    /// usize is compiled codegen offset here (like jump offsets)
    function_lines: Vec<(usize, Option<SourceLine>)>,
    line_hint: usize,
    /// `a.b()` looks `b` up as a proc first, and plain `a.b` as a var first
    callees: FxHashSet<IrNodeId>,
    register_scratch: registers::Scratch,
}

impl Generator {
    fn new() -> Self {
        Self {
            constants: Vec::new(),
            constant_ids: FxHashMap::default(),
            strings: Vec::new(),
            string_ids: FxHashMap::default(),
            paths: Vec::new(),
            path_ids: FxHashMap::default(),
            functions: Vec::new(),
            function_ids: FxHashMap::default(),
            labels: FxHashMap::default(),
            addresses: Vec::new(),
            jumps: Vec::new(),
            next_label: 0,
            code: Vec::new(),
            lines: LineTable::default(),
            function_lines: Vec::new(),
            line_hint: 0,
            callees: FxHashSet::default(),
            register_scratch: registers::Scratch::default(),
        }
    }

    fn generate(mut self, module: &ir::Module, procedures: Option<&FxHashSet<ProcId>>) -> Result<Module, CodegenError> {
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
            lines: self.lines,
            index: module.index.clone(),
        })
    }

    fn register_constants(
        &mut self, module: &ir::Module, referenced: Option<&FxHashSet<IrNodeId>>,
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
            self.constants.push(Value::clone(value));
        }

        Ok(())
    }

    fn register_functions(
        &mut self, module: &ir::Module, procedures: Option<&FxHashSet<ProcId>>,
        referenced: Option<&FxHashSet<IrNodeId>>,
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
                register_count: 0,
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
                register_count: 0,
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
        let constants = proc_constants(module, proc, &blocks)?;
        let allocation = registers::allocate(&mut self.register_scratch, module, proc, &blocks, &constants)?;
        let mut state = FunctionState {
            registers: allocation.registers,
            parameter_defaults: FxHashMap::default(),
            phi_temporaries: allocation.phi_temporaries,
        };

        let mut constant_defaults = Vec::new();
        for (index, parameter) in proc.params.iter().enumerate() {
            let Some(default) = parameter.default else {
                continue;
            };

            let destination = state.register(proc.parameters[index])?;
            if matches!(module.node(default), Some(IrNode::Constant(_))) {
                constant_defaults.push((default, destination));
            } else {
                state.parameter_defaults.insert(default, destination);
            }
        }

        let address = self.code_offset()?;
        let mut loaded = FxHashSet::default();
        for constant in constants {
            if !loaded.insert(constant) {
                continue;
            }

            let id = self
                .constant_ids
                .get(&constant)
                .copied()
                .ok_or(CodegenError::MissingConstant(constant))?;
            self.emit_op(Op::MoveConst);
            self.emit_register(state.register(constant)?);
            self.emit_u32(id.0);
        }

        for (default, destination) in constant_defaults {
            self.emit_op(Op::DefaultParameter);
            self.emit_register(destination);
            self.emit_register(state.register(default)?);
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
        function.register_count = allocation.register_count;

        Ok(())
    }

    fn generate_block(
        &mut self, module: &ir::Module, block: IrNodeId, next: Option<IrNodeId>, state: &FunctionState,
        edge_stubs: &mut Vec<EdgeStub>,
    ) -> Result<(), CodegenError> {
        self.mark_label(Label::Block(block));
        let instructions = module.block(block).ok_or(CodegenError::ExpectedBlock(block))?;
        for instruction in instructions {
            let line = module.node_lines.get_near(*instruction, &mut self.line_hint);
            self.function_lines.push((self.code.len(), line));
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
                    let condition = state.register(*condition)?;
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
                    self.emit_register(condition);

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
                self.emit_op(Op::LoadVar);
                self.emit_register(state.register(id)?);
                self.emit_u32(name.0);
            },
            IrNode::Builtin(builtin) => {
                self.emit_op(Op::LoadBuiltin);
                self.emit_register(state.register(id)?);
                self.emit_u8(builtin_code(*builtin) as u8);
            },
            IrNode::Interpolate(interpolation) => {
                let values = interpolation
                    .values
                    .iter()
                    .map(|value| state.register(*value))
                    .collect::<Result<Vec<_>, _>>()?;
                let chunks = interpolation
                    .chunks
                    .iter()
                    .map(|chunk| self.intern_string(chunk.clone()))
                    .collect::<Result<Vec<_>, _>>()?;
                self.emit_op(Op::Interpolate);
                self.emit_register(state.register(id)?);
                self.emit_count(values.len(), "interpolation value")?;
                self.emit_count(chunks.len(), "interpolation chunks")?;
                for chunk in chunks {
                    self.emit_u32(chunk.0);
                }

                for value in values {
                    self.emit_register(value);
                }
            },
            IrNode::Unary { op, operand } => {
                let operand = state.register(*operand)?;
                self.emit_op(Op::Unary);
                self.emit_register(state.register(id)?);
                self.emit_u8(unary_code(*op) as u8);
                self.emit_register(operand);
            },
            IrNode::Binary { op, lhs, rhs } => {
                let lhs = state.register(*lhs)?;
                let rhs = state.register(*rhs)?;
                self.emit_op(Op::Binary);
                self.emit_register(state.register(id)?);
                self.emit_u8(binary_code(*op) as u8);
                self.emit_register(lhs);
                self.emit_register(rhs);
            },
            IrNode::CompoundBinary { op, lhs, rhs } => {
                let lhs = state.register(*lhs)?;
                let rhs = state.register(*rhs)?;
                self.emit_op(Op::CompoundBinary);
                self.emit_register(state.register(id)?);
                self.emit_u8(binary_code(*op) as u8);
                self.emit_register(lhs);
                self.emit_register(rhs);
            },
            IrNode::AccessField { object, name, access } => {
                let object = state.register(*object)?;
                let name = self.intern_identifier(name)?;
                self.emit_op(Op::AccessField);
                self.emit_register(state.register(id)?);
                self.emit_u32(name.0);
                self.emit_u8(access_code(*access) as u8);
                self.emit_bool(self.callees.contains(&id));
                self.emit_register(object);
            },
            IrNode::Initial { object, name } => {
                let object = object.map(|object| state.register(object)).transpose()?;
                let name = self.intern_identifier(name)?;
                self.emit_op(Op::Initial);
                self.emit_register(state.register(id)?);
                self.emit_bool(object.is_some());
                self.emit_u32(name.0);
                if let Some(object) = object {
                    self.emit_register(object);
                }
            },
            IrNode::Index {
                object,
                index,
                conditional,
            } => {
                let object = state.register(*object)?;
                let index = state.register(*index)?;
                self.emit_op(Op::Index);
                self.emit_register(state.register(id)?);
                self.emit_bool(*conditional);
                self.emit_register(object);
                self.emit_register(index);
            },
            IrNode::Call { callee, args } => {
                let conditional = matches!(
                    module.node(*callee),
                    Some(IrNode::AccessField {
                        access: ir::AccessKind::SafeDot | ir::AccessKind::SafeColon,
                        ..
                    })
                );
                let callee = state.register(*callee)?;
                self.emit_op(Op::Call);
                self.emit_register(state.register(id)?);
                self.emit_bool(conditional);
                self.emit_register(callee);
                self.emit_arguments(state, args)?;
            },
            IrNode::FunctionCall { function, args } => {
                let function = self
                    .function_ids
                    .get(function)
                    .copied()
                    .ok_or(CodegenError::MissingFunction(*function))?;
                self.emit_op(Op::FunctionCall);
                self.emit_register(state.register(id)?);
                self.emit_u32(function.0);
                self.emit_arguments(state, args)?;
            },
            IrNode::Super {
                args,
                forwards_extra_args,
            } => {
                self.emit_op(Op::SuperCall);
                self.emit_register(state.register(id)?);
                self.emit_bool(*forwards_extra_args);
                self.emit_arguments(state, args)?;
            },
            IrNode::New { ty, args } => {
                let ty = ty.map(|ty| state.register(ty)).transpose()?;

                self.emit_op(Op::New);
                self.emit_register(state.register(id)?);
                self.emit_bool(ty.is_some());
                if let Some(ty) = ty {
                    self.emit_register(ty);
                }
                self.emit_arguments(state, args)?;
            },
            IrNode::ModifiedType { path, overrides } => {
                let path = self.intern_path(TreePath::clone(path))?;
                let overrides = overrides
                    .iter()
                    .map(|(name, value)| Ok((self.intern_identifier(name)?, state.register(*value)?)))
                    .collect::<Result<Vec<_>, _>>()?;

                self.emit_op(Op::ModifiedType);
                self.emit_register(state.register(id)?);
                self.emit_u32(path.0);
                self.emit_count(overrides.len(), "modified type override")?;
                for (name, value) in overrides {
                    self.emit_u32(name.0);
                    self.emit_register(value);
                }
            },
            IrNode::List(args) => {
                self.emit_op(Op::MakeList);
                self.emit_register(state.register(id)?);
                self.emit_arguments(state, args)?;
            },
            IrNode::Pick(choices) => {
                let choices = choices
                    .iter()
                    .map(|(weight, value)| {
                        Ok((
                            weight.map(|weight| state.register(weight)).transpose()?,
                            state.register(*value)?,
                        ))
                    })
                    .collect::<Result<Vec<_>, _>>()?;

                self.emit_op(Op::Pick);
                self.emit_register(state.register(id)?);
                self.emit_count(choices.len(), "pick choice")?;
                for (weight, value) in choices {
                    self.emit_bool(weight.is_some());
                    if let Some(weight) = weight {
                        self.emit_register(weight);
                    }

                    self.emit_register(value);
                }
            },
            IrNode::InRange {
                value,
                start,
                end,
                step,
            } => {
                let value = state.register(*value)?;
                let start = state.register(*start)?;
                let end = state.register(*end)?;
                let step = step.map(|step| state.register(step)).transpose()?;
                self.emit_op(Op::InRange);
                self.emit_register(state.register(id)?);
                self.emit_bool(step.is_some());
                self.emit_register(value);
                self.emit_register(start);
                self.emit_register(end);
                if let Some(step) = step {
                    self.emit_register(step);
                }
            },
            IrNode::Range { start, end, step } => {
                let start = state.register(*start)?;
                let end = state.register(*end)?;
                let step = step.map(|step| state.register(step)).transpose()?;
                self.emit_op(Op::Range);
                self.emit_register(state.register(id)?);
                self.emit_bool(step.is_some());
                self.emit_register(start);
                self.emit_register(end);
                if let Some(step) = step {
                    self.emit_register(step);
                }
            },
            IrNode::IterInit {
                list,
                ty,
                value_is_associated,
            } => {
                let list = state.register(*list)?;
                let ty = ty.as_deref().cloned().map(|ty| self.intern_path(ty)).transpose()?;
                self.emit_op(Op::IterInit);
                self.emit_register(state.register(id)?);
                self.emit_optional_path(ty);
                self.emit_bool(*value_is_associated);
                self.emit_register(list);
            },
            IrNode::IterNext(iter) | IrNode::IterValue(iter) | IrNode::IterKey(iter) => {
                let op = match node {
                    IrNode::IterNext(_) => Op::IterNext,
                    IrNode::IterValue(_) => Op::IterValue,
                    _ => Op::IterKey,
                };
                let iter = state.register(*iter)?;
                self.emit_op(op);
                self.emit_register(state.register(id)?);
                self.emit_register(iter);
            },
            IrNode::RangeTest { current, end, step } => {
                let current = state.register(*current)?;
                let end = state.register(*end)?;
                let step = state.register(*step)?;
                self.emit_op(Op::RangeTest);
                self.emit_register(state.register(id)?);
                self.emit_register(current);
                self.emit_register(end);
                self.emit_register(step);
            },
            IrNode::SetField {
                object,
                name,
                access,
                value,
            } => {
                let object = state.register(*object)?;
                let value = state.register(*value)?;
                let name = self.intern_identifier(name)?;
                self.emit_op(Op::SetField);
                self.emit_u32(name.0);
                self.emit_u8(access_code(*access) as u8);
                self.emit_register(object);
                self.emit_register(value);
            },
            IrNode::SetIndex {
                object,
                index,
                value,
                conditional,
            } => {
                let object = state.register(*object)?;
                let index = state.register(*index)?;
                let value = state.register(*value)?;
                self.emit_op(Op::SetIndex);
                self.emit_bool(*conditional);
                self.emit_register(object);
                self.emit_register(index);
                self.emit_register(value);
            },
            IrNode::Store { pointer, value } | IrNode::Initialize { pointer, value } => {
                let op = match node {
                    IrNode::Store { .. } => Op::StoreVar,
                    _ => Op::InitVar,
                };
                let value = state.register(*value)?;
                let name = variable_name(module, *pointer)?;
                let name = self.intern_identifier(name)?;
                self.emit_op(op);
                self.emit_u32(name.0);
                self.emit_register(value);
            },
            IrNode::StoreBuiltin { builtin, value } => {
                let value = state.register(*value)?;
                self.emit_op(Op::StoreBuiltin);
                self.emit_u8(builtin_code(*builtin) as u8);
                self.emit_register(value);
            },
            IrNode::CatchValue => {
                self.emit_op(Op::CatchValue);
                self.emit_register(state.register(id)?);
            },
            IrNode::Return(value) => match value {
                Some(value) => {
                    let value = state.register(*value)?;
                    self.emit_op(Op::ReturnValue);
                    self.emit_register(value);
                },
                None => self.emit_op(Op::Return),
            },
            IrNode::Del(value) | IrNode::Throw(value) => {
                let op = match node {
                    IrNode::Del(_) => Op::Del,
                    _ => Op::Throw,
                };
                let value = state.register(*value)?;
                self.emit_op(op);
                self.emit_register(value);
            },
            IrNode::Output { target, value } => {
                self.emit_op(Op::Output);
                match target {
                    OutputTarget::Value(target) => {
                        self.emit_u8(OutputTargetKind::Value as u8);
                        self.emit_register(state.register(*target)?);
                        self.emit_register(state.register(*value)?);
                    },
                    OutputTarget::Field { object, name, access } => {
                        let name = self.intern_identifier(name)?;

                        self.emit_u8(OutputTargetKind::Field as u8);
                        self.emit_u32(name.0);
                        self.emit_u8(access_code(*access) as u8);
                        self.emit_register(state.register(*object)?);
                        self.emit_register(state.register(*value)?);
                    },
                    OutputTarget::Index {
                        object,
                        index,
                        conditional,
                    } => {
                        self.emit_u8(OutputTargetKind::Index as u8);
                        self.emit_bool(*conditional);
                        self.emit_register(state.register(*object)?);
                        self.emit_register(state.register(*index)?);
                        self.emit_register(state.register(*value)?);
                    },
                }
            },
            IrNode::TryCatch { body, catch, merge } => {
                self.emit_op(Op::TryCatch);
                self.emit_address(Label::Block(*body));
                self.emit_address(Label::Block(*catch));
                self.emit_address(Label::Block(*merge));
            },
            IrNode::Spawn { body, .. } => {
                self.emit_op(Op::Spawn);
                self.emit_address(Label::Block(*body));
            },
            IrNode::EndSpawn => self.emit_op(Op::EndSpawn),
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
            self.emit_op(Op::DefaultParameter);
            self.emit_register(*parameter);
            self.emit_register(state.register(id)?);
        }

        Ok(())
    }

    fn emit_phi_copies(
        &mut self, module: &ir::Module, state: &FunctionState, predecessor: IrNodeId, target: IrNodeId,
    ) -> Result<(), CodegenError> {
        let copies = phi_copies(module, state, predecessor, target)?;

        // the copies happen at once, a phi that reads another phi of the same block needs
        // that phi's old value, so go through the temporaries when any source is overwritten
        let is_overlapping = copies
            .iter()
            .any(|copy| copies.iter().any(|other| other.destination == copy.source));
        if !is_overlapping {
            for copy in &copies {
                self.emit_move(copy.destination, copy.source);
            }

            return Ok(());
        }

        for copy in &copies {
            self.emit_move(copy.temporary, copy.source);
        }

        for copy in &copies {
            self.emit_move(copy.destination, copy.temporary);
        }

        Ok(())
    }

    fn emit_move(&mut self, destination: RegisterId, source: RegisterId) {
        self.emit_op(Op::Move);
        self.emit_register(destination);
        self.emit_register(source);
    }

    fn emit_arguments(&mut self, state: &FunctionState, args: &[Argument]) -> Result<(), CodegenError> {
        self.emit_count(args.len(), "argument")?;
        for argument in args {
            let mut shape = 0;
            if argument.key.is_some() {
                shape |= ARGUMENT_KEY;
            }

            if argument.value.is_some() {
                shape |= ARGUMENT_VALUE;
            }

            self.emit_u8(shape);
            if let Some(key) = argument.key {
                self.emit_register(state.register(key)?);
            }

            if let Some(value) = argument.value {
                self.emit_register(state.register(value)?);
            }
        }

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

    fn emit_op(&mut self, op: Op) {
        self.emit_u8(op as u8);
    }

    fn emit_bool(&mut self, value: bool) {
        self.emit_u8(u8::from(value));
    }

    fn emit_u8(&mut self, value: u8) {
        self.code.push(value);
    }

    fn emit_u16(&mut self, value: u16) {
        self.code.extend_from_slice(&value.to_le_bytes());
    }

    fn emit_u32(&mut self, value: u32) {
        self.code.extend_from_slice(&value.to_le_bytes());
    }

    fn emit_register(&mut self, register: RegisterId) {
        self.emit_u16(register.0);
    }

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

    fn mark_label(&mut self, label: Label) {
        self.labels.insert(label, self.code.len());
    }

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

        // keep track of removed jumps, they are used for function line tracking
        let removed_jumps = self
            .jumps
            .iter()
            .zip(&removed)
            .filter_map(|(jump, removed)| removed.then_some(jump.instruction))
            .collect::<Vec<_>>();
        let mut skipped = 0;
        for (offset, line) in std::mem::take(&mut self.function_lines) {
            while removed_jumps
                .get(skipped)
                .is_some_and(|instruction| *instruction < offset)
            {
                skipped += 1;
            }

            let offset = u32::try_from(offset - skipped * 5).map_err(|_| CodegenError::CodeTooLarge)?;
            self.lines.push(CodeOffset(offset), line);
        }

        let end = u32::try_from(self.code.len()).map_err(|_| CodegenError::CodeTooLarge)?;
        self.lines.push(CodeOffset(end), None);

        let labels = std::mem::take(&mut self.labels)
            .into_iter()
            .map(|(label, offset)| (label, compact_offset(offset, &self.jumps, &removed)))
            .collect::<FxHashMap<_, _>>();
        let removed_addresses = self
            .jumps
            .iter()
            .enumerate()
            .filter_map(|(index, jump)| removed[index].then_some(jump.address))
            .collect::<FxHashSet<_>>();
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
            .collect::<FxHashMap<_, _>>();

        for jump in &self.jumps {
            let original = self.addresses[jump.address].target;
            let mut target = original;
            let mut seen = FxHashSet::default();
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

fn label_offset(labels: &FxHashMap<Label, usize>, label: Label) -> Result<usize, CodegenError> {
    match labels.get(&label).copied() {
        Some(offset) => Ok(offset),
        None => match label {
            Label::Block(block) => Err(CodegenError::ExpectedBlock(block)),
            Label::Synthetic(_) => unreachable!("generated synthetic target should have a label"),
        },
    }
}

struct PhiCopy {
    source: RegisterId,
    destination: RegisterId,
    temporary: RegisterId,
}

fn phi_copies(
    module: &ir::Module, state: &FunctionState, predecessor: IrNodeId, target: IrNodeId,
) -> Result<Vec<PhiCopy>, CodegenError> {
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
        let source = state.register(source)?;
        let destination = state.register(*instruction)?;
        if source == destination {
            continue;
        }

        let temporary = state
            .phi_temporaries
            .get(instruction)
            .copied()
            .ok_or(CodegenError::MissingRegister(*instruction))?;
        copies.push(PhiCopy {
            source,
            destination,
            temporary,
        });
    }

    Ok(copies)
}

fn has_phi_copies(
    module: &ir::Module, state: &FunctionState, predecessor: IrNodeId, target: IrNodeId,
) -> Result<bool, CodegenError> {
    Ok(!phi_copies(module, state, predecessor, target)?.is_empty())
}

fn block_layout(module: &ir::Module, entry: IrNodeId, reachable: &[IrNodeId]) -> Result<Vec<IrNodeId>, CodegenError> {
    let reachable_set = reachable.iter().copied().collect::<FxHashSet<_>>();
    let mut placed = FxHashSet::default();
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

fn referenced_nodes(module: &ir::Module, procedures: &FxHashSet<ProcId>) -> Result<FxHashSet<IrNodeId>, CodegenError> {
    let mut referenced = FxHashSet::default();

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

fn proc_constants(module: &ir::Module, proc: &Procedure, blocks: &[IrNodeId]) -> Result<Vec<IrNodeId>, CodegenError> {
    let mut constants = Vec::new();
    for parameter in &proc.params {
        if let Some(default) = parameter.default
            && matches!(module.node(default), Some(IrNode::Constant(_)))
        {
            constants.push(default);
        }
    }

    for block in blocks {
        for instruction in module.block(*block).ok_or(CodegenError::ExpectedBlock(*block))? {
            let node = module
                .node(*instruction)
                .ok_or(CodegenError::MissingNode(*instruction))?;
            node.for_each_operand(|operand| {
                if matches!(module.node(operand), Some(IrNode::Constant(_))) {
                    constants.push(operand);
                }
            });
        }
    }

    Ok(constants)
}

pub(crate) fn reachable_blocks(module: &ir::Module, entry: IrNodeId) -> Result<Vec<IrNodeId>, CodegenError> {
    let mut blocks = Vec::new();
    let mut seen = FxHashSet::default();
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
                IrNode::Spawn { body, .. } => pending.push_back(*body),
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
            | IrNode::Interpolate(_)
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
    macro_rules! fixture {
        ($path:literal) => {
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/", $path))
        };
    }

    use core::{location::Location, path::TreePath, types::ProcId};

    use super::*;

    pub(crate) fn lower(source: &str) -> ir::Module {
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

    fn analyze(source: &str) -> (ObjectTree, ir::Module) {
        let (tokens, errors) = lexer::tokenize(source);
        assert!(errors.is_empty(), "{errors:?}");
        let ast = ast::parse(&tokens).expect("fixture should parse");
        let (tree, errors) = sema::analyze(&ast);
        assert!(errors.is_empty(), "{errors:?}");
        let (module, _) = ir::lower(&ast, &tree, true);

        (tree, module)
    }

    fn proc_id(tree: &ObjectTree, module: &ir::Module, owner: &str, name: &str) -> ProcId {
        tree.id_of(&TreePath::parse(owner))
            .and_then(|owner| module.index.inherited_body(tree, owner, &name.into()))
            .expect("fixture procedure should exist")
    }

    fn bytecode_instructions(output: &str) -> Vec<&str> {
        output
            .lines()
            .filter(|line| line.trim_start().starts_with("0x"))
            .map(str::trim)
            .collect::<Vec<_>>()
    }

    fn operation<'a>(instructions: &[&'a str], mnemonic: &str) -> (usize, &'a str) {
        instructions
            .iter()
            .enumerate()
            .find_map(|(index, line)| {
                let (_, instruction) = line.split_once(' ')?;
                instruction
                    .starts_with(&format!("{mnemonic} "))
                    .then_some((index, instruction))
            })
            .unwrap_or_else(|| panic!("missing {mnemonic}"))
    }

    fn destination(instruction: &str) -> &str {
        instruction
            .split_once(' ')
            .and_then(|(_, operands)| operands.split_once(','))
            .map(|(destination, _)| destination)
            .expect("instruction should write a register")
    }

    #[test]
    fn binary_operands_read_parameter_registers_directly() {
        let module = generate(&lower(fixture!(
            "programs/binary_operands_read_parameter_registers_directly.dm"
        )))
        .expect("generate");
        let output = disasm::dump(&module).expect("disassemble");
        let instructions = bytecode_instructions(&output);
        let (_, add) = operation(&instructions, "binary");

        assert_eq!(module.magic, Module::MAGIC);
        assert_eq!(module.version, Module::VERSION);
        assert_eq!(module.functions.len(), 1);
        assert_eq!(module.functions[0].parameter_count, 2);
        assert!(add.ends_with(", add r0, r1"), "{output}");
        assert!(
            output.contains(&format!("return_value {}\n", destination(add))),
            "{output}"
        );
        assert!(!output.contains(" move "), "{output}");
    }

    #[test]
    fn named_calls_use_function_table_ids() {
        let module = generate(&lower(fixture!("programs/named_calls_use_function_table_ids.dm"))).expect("generate");
        let output = disasm::dump(&module).expect("disassemble");
        let instructions = bytecode_instructions(&output);
        let (_, call) = operation(&instructions, "function_call");

        assert_eq!(module.functions.len(), 2);
        assert!(call.ends_with(", fn1 args=[r0]"), "{output}");
    }

    #[test]
    fn dynamic_calls_look_up_the_method_before_evaluating_arguments() {
        let module = generate(&lower(fixture!(
            "programs/dynamic_calls_look_up_the_method_before_evaluating_arguments.dm"
        )))
        .expect("generate");
        let output = disasm::dump(&module).expect("disassemble");
        let instructions = bytecode_instructions(&output);
        let (lookup, method) = operation(&instructions, "access_field");
        let (argument, add) = operation(&instructions, "binary");
        let (call, dynamic) = operation(&instructions, "call");

        assert!(method.ends_with(", r0.run method"), "{output}");
        assert!(add.ends_with(", add r1, r2"), "{output}");
        assert!(
            dynamic.ends_with(&format!(", {} args=[{}]", destination(method), destination(add))),
            "{output}"
        );
        assert!(lookup < argument && argument < call, "{output}");
    }

    // fn0 = function /::count params=1 registers=5 address=0x00000000
    //   0x00000000 move_const r1, const0 ; 0
    //   0x00000007 move_const r2, const1 ; 1
    //   0x0000000e binary r3, comp_less r1, r0
    //   0x00000016 jump_if_false 0x0000002a, r3
    //   0x0000001d binary r1, add r1, r2
    //   0x00000025 jump 0x0000000e
    //   0x0000002a return_value r1
    #[test]
    fn a_loop_counter_is_updated_in_its_own_register() {
        let module = generate(&lower(fixture!("programs/counting_loop.dm"))).expect("generate");
        let output = disasm::dump(&module).expect("disassemble");
        let instructions = bytecode_instructions(&output);
        let (_, returned) = operation(&instructions, "return_value");
        let counter = returned.strip_prefix("return_value ").expect("returned register");

        assert!(
            output.contains(&format!("binary {counter}, add {counter}, ")),
            "{output}"
        );
        assert!(!output.contains(" move "), "{output}");
    }

    // fn0 = function /::select params=3 registers=6 address=0x00000000
    //   0x00000000 move_const r3, const1 ; 1
    //   0x00000007 move_const r4, const2 ; 2
    //   0x0000000e jump_if_false 0x00000020, r0
    //   0x00000015 binary r3, add r1, r3
    //   0x0000001d return_value r3
    //   0x00000020 binary r3, add r2, r4
    //   0x00000028 jump 0x0000001d
    #[test]
    fn both_arms_write_the_merge_register() {
        let module = generate(&lower(fixture!(
            "programs/phi_sources_are_live_only_on_their_own_edge.dm"
        )))
        .expect("generate");
        let output = disasm::dump(&module).expect("disassemble");
        let instructions = bytecode_instructions(&output);
        let (_, returned) = operation(&instructions, "return_value");
        let merged = returned.strip_prefix("return_value ").expect("returned register");

        assert!(output.contains(&format!("binary {merged}, add r1, ")), "{output}");
        assert!(output.contains(&format!("binary {merged}, add r2, ")), "{output}");
        assert!(!output.contains(" move "), "{output}");
    }

    // fn0 = function /::support params=1 registers=4 address=0x00000000
    //   0x00000000 move_const r1, const1 ; 1
    //   0x00000007 move_const r2, const0 ; null
    //   0x0000000e binary r1, comp_greater r0, r1
    //   0x00000016 jump_if_false 0x00000022, r1
    //   0x0000001d move r2, r0
    //   0x00000022 return_value r2
    #[test]
    fn a_phi_takes_a_free_source_register() {
        let module = generate(&lower(fixture!("programs/a_phi_takes_a_free_source_register.dm"))).expect("generate");
        let output = disasm::dump(&module).expect("disassemble");
        let instructions = bytecode_instructions(&output);
        let null = instructions
            .iter()
            .find(|line| line.ends_with("; null"))
            .and_then(|line| line.split_once(' '))
            .map(|(_, instruction)| destination(instruction))
            .expect("null constant");
        let (_, returned) = operation(&instructions, "return_value");

        assert_eq!(returned, format!("return_value {null}"), "{output}");
        assert_eq!(output.matches(" move ").count(), 1, "{output}");
        assert!(output.contains(&format!(" move {null}, r0\n")), "{output}");
    }

    // fn0 = function /::scale params=2 registers=4 address=0x00000000
    //   0x00000000 move_const r2, const0 ; 2
    //   0x00000007 move_const r3, const1 ; 1
    //   0x0000000e binary r2, mul r0, r2
    //   0x00000016 binary r2, add r2, r3
    //   0x0000001e return_value r2
    #[test]
    fn parameters_keep_their_registers_after_their_last_use() {
        let module = generate(&lower(fixture!(
            "programs/parameters_keep_their_registers_after_their_last_use.dm"
        )))
        .expect("generate");
        let output = disasm::dump(&module).expect("disassemble");

        for line in bytecode_instructions(&output) {
            let (_, instruction) = line.split_once(' ').expect("address");
            if ["binary ", "move_const ", "move "]
                .iter()
                .any(|mnemonic| instruction.starts_with(mnemonic))
            {
                assert!(!["r0", "r1"].contains(&destination(instruction)), "{output}");
            }
        }
    }

    #[test]
    fn conditional_true_successor_falls_through() {
        let module =
            generate(&lower(fixture!("programs/conditional_true_successor_falls_through.dm"))).expect("generate");
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
        let ir = lower(fixture!("programs/false_edge_phi_copies_stay_on_the_false_path.dm"));
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
            .nth(2)
            .map(|target| target.trim_end_matches(','))
            .expect("conditional target");
        let stub = instructions
            .iter()
            .position(|instruction| instruction.starts_with(target))
            .expect("false-edge stub");
        let (return_value, _) = operation(&instructions, "return_value");

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
        let module = generate(&lower(fixture!(
            "programs/compacted_functions_keep_contiguous_ranges.dm"
        )))
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
    fn lines_survive_jump_compaction() {
        let module = generate(&lower(fixture!("programs/lines_survive_jump_compaction.dm"))).expect("generate");
        let function = &module.functions[0];
        let lines = (function.address.0..function.address.0 + function.length)
            .filter_map(|offset| module.line_at(CodeOffset(offset)))
            .map(|line| line.line)
            .collect::<std::collections::BTreeSet<_>>();

        assert_eq!(lines.into_iter().collect::<Vec<_>>(), [2, 3, 5, 6]);
    }

    #[test]
    fn unsupported_source_constructs_become_traps() {
        let module = generate(&lower(fixture!(
            "programs/unsupported_source_constructs_become_traps.dm"
        )))
        .expect("generate");
        let output = disasm::dump(&module).expect("disassemble");

        assert!(output.contains("trap break outside loop"), "{output}");
    }

    #[test]
    fn output_preserves_field_reference_shape() {
        let module =
            generate(&lower(fixture!("programs/output_preserves_field_reference_shape.dm"))).expect("generate");
        let output = disasm::dump(&module).expect("disassemble");
        let instructions = bytecode_instructions(&output);
        let (_, hello) = operation(&instructions, "move_const");
        let (_, world) = operation(&instructions, "load_builtin");
        let (_, write) = operation(&instructions, "output");

        assert_eq!(
            write,
            format!("output field {}.log, {}", destination(world), destination(hello)),
            "{output}"
        );
        assert!(!output.contains("access_field"), "{output}");
    }

    #[test]
    fn try_inside_a_loop_does_not_leave_orphaned_ssa_loads() {
        let module = lower(fixture!(
            "programs/try_inside_a_loop_does_not_leave_orphaned_ssa_loads.dm"
        ));

        generate(&module).expect("generate");
    }

    #[test]
    fn phi_cycles_read_every_source_before_writing_any_destination() {
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
            index: ir::ProcIndex::default(),
            node_lines: Default::default(),
        };
        let module = generate(&module).expect("generate");
        let output = disasm::dump(&module).expect("disassemble");
        let instructions = bytecode_instructions(&output);
        let (_, back_edge) = operation(&instructions, "jump");
        let header = back_edge.strip_prefix("jump ").expect("jump target");

        let mut entry = FxHashMap::default();
        let mut values = FxHashMap::default();
        let mut is_loop = false;
        for line in &instructions {
            let (address, instruction) = line.split_once(' ').expect("address");
            if address == header {
                is_loop = true;
            }

            if instruction.starts_with("jump ") {
                break;
            }

            let (destination, source) = instruction
                .strip_prefix("move ")
                .and_then(|operands| operands.split_once(", "))
                .unwrap_or_else(|| panic!("only phi copies expected: {output}"));
            if is_loop {
                let value = values.get(source).copied().unwrap_or(source);
                values.insert(destination, value);
            } else {
                entry.insert(source, destination);
            }
        }

        let first = entry["r0"];
        let second = entry["r1"];
        assert_eq!(values.get(first).copied(), Some(second), "{output}");
        assert_eq!(values.get(second).copied(), Some(first), "{output}");
    }

    #[test]
    fn reachable_codegen_keeps_named_dispatch_candidates_and_stable_proc_ids() {
        let (tree, ir_module) = analyze(fixture!(
            "programs/reachable_codegen_keeps_named_dispatch_candidates_and_stable_proc_ids.dm"
        ));
        let entry = proc_id(&tree, &ir_module, "/", "entry");
        let base_live = proc_id(&tree, &ir_module, "/datum/base", "live");
        let child_live = proc_id(&tree, &ir_module, "/datum/base/child", "live");
        let dead = proc_id(&tree, &ir_module, "/datum/base", "dead");
        let unused = proc_id(&tree, &ir_module, "/", "unused");

        let module = generate_reachable(&ir_module, &tree, &[entry], &[]).expect("generate selected procedures");

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
        let (tree, ir_module) = analyze(fixture!(
            "programs/reachable_codegen_does_not_treat_field_reads_as_method_calls.dm"
        ));
        let entry = proc_id(&tree, &ir_module, "/", "entry");
        let status = proc_id(&tree, &ir_module, "/datum/base", "status");

        let module = generate_reachable(&ir_module, &tree, &[entry], &[]).expect("generate selected procedures");

        assert!(module.function_for_proc(entry).is_some());
        assert!(module.function_for_proc(status).is_none());
    }

    #[test]
    fn reachable_codegen_limits_typed_receiver_dispatch_to_its_type_family() {
        let (tree, ir_module) = analyze(fixture!(
            "programs/reachable_codegen_limits_typed_receiver_dispatch_to_its_type_family.dm"
        ));
        let entry = proc_id(&tree, &ir_module, "/", "entry");
        let wanted = proc_id(&tree, &ir_module, "/datum/wanted", "Initialize");
        let child = proc_id(&tree, &ir_module, "/datum/wanted/child", "Initialize");
        let unrelated = proc_id(&tree, &ir_module, "/datum/unrelated", "Initialize");

        let module = generate_reachable(&ir_module, &tree, &[entry], &[]).expect("generate selected procedures");

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
        let (tree, ir_module) = analyze(fixture!(
            "programs/reachable_codegen_follows_super_and_roots_lazy_initializers.dm"
        ));
        let entry = proc_id(&tree, &ir_module, "/", "entry");
        let latest = proc_id(&tree, &ir_module, "/datum/base/child", "step");
        let previous = ir_module
            .proc(latest)
            .and_then(|procedure| procedure.previous)
            .expect("same-type previous");
        let inherited = proc_id(&tree, &ir_module, "/datum/base", "step");
        let initializer = tree
            .id_of(&TreePath::parse("/datum/holder"))
            .and_then(|holder| ir_module.index.inherited_initializer(&tree, holder, &"value".into()))
            .expect("runtime initializer");
        let initialize = proc_id(&tree, &ir_module, "/", "initialize");
        let unused = proc_id(&tree, &ir_module, "/", "unused");

        let module = generate_reachable(&ir_module, &tree, &[entry], &[]).expect("generate selected procedures");

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
        let (tree, ir_module) = analyze(fixture!("programs/reachable_codegen_omits_unread_lazy_initializers.dm"));
        let entry = proc_id(&tree, &ir_module, "/", "entry");
        let initialize = proc_id(&tree, &ir_module, "/", "initialize");
        let initializer = tree
            .id_of(&TreePath::parse("/datum/holder"))
            .and_then(|holder| ir_module.index.inherited_initializer(&tree, holder, &"value".into()))
            .expect("runtime initializer");

        let module = generate_reachable(&ir_module, &tree, &[entry], &[]).expect("generate selected procedures");

        assert!(module.function_for_proc(entry).is_some());
        assert!(module.function_for_proc(initializer).is_none());
        assert!(module.function_for_proc(initialize).is_none());
    }

    #[test]
    fn reachable_codegen_bounds_opaque_calls_to_reachable_proc_paths() {
        let (tree, ir_module) = analyze(fixture!(
            "programs/reachable_codegen_bounds_opaque_calls_to_reachable_proc_paths.dm"
        ));
        let entry = proc_id(&tree, &ir_module, "/", "entry");
        let invoke = proc_id(&tree, &ir_module, "/", "invoke");
        let call = proc_id(&tree, &ir_module, "/", "call");
        let live = proc_id(&tree, &ir_module, "/", "live");
        let dead = proc_id(&tree, &ir_module, "/", "dead");

        let module = generate_reachable(&ir_module, &tree, &[entry], &[]).expect("generate selected procedures");

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
        let (tree, ir_module) = analyze(fixture!("programs/reachable_codegen_resolves_static_call_names.dm"));
        let entry = proc_id(&tree, &ir_module, "/", "entry");
        let call = proc_id(&tree, &ir_module, "/", "call");
        let live = proc_id(&tree, &ir_module, "/datum/base", "live");
        let dead = proc_id(&tree, &ir_module, "/datum/base", "dead");

        let module = generate_reachable(&ir_module, &tree, &[entry], &[]).expect("generate selected procedures");

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
        let (tree, ir_module) = analyze(fixture!(
            "programs/reachable_codegen_keeps_only_exact_constructor_targets.dm"
        ));
        let entry = proc_id(&tree, &ir_module, "/", "entry");
        let live_new = proc_id(&tree, &ir_module, "/datum/live", "New");
        let dead_new = proc_id(&tree, &ir_module, "/datum/dead", "New");
        let live_helper = proc_id(&tree, &ir_module, "/", "live_helper");
        let dead_helper = proc_id(&tree, &ir_module, "/", "dead_helper");

        let module = generate_reachable(&ir_module, &tree, &[entry], &[]).expect("generate selected procedures");

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
        let (tree, ir_module) = analyze(fixture!(
            "programs/reachable_codegen_keeps_every_constructor_for_a_computed_type.dm"
        ));
        let entry = proc_id(&tree, &ir_module, "/", "entry");
        let live_new = proc_id(&tree, &ir_module, "/datum/live", "New");
        let dead_new = proc_id(&tree, &ir_module, "/datum/dead", "New");
        let unrelated = proc_id(&tree, &ir_module, "/", "unrelated");

        let module = generate_reachable(&ir_module, &tree, &[entry], &[]).expect("generate selected procedures");

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
        let (tree, ir_module) = analyze(fixture!(
            "programs/reachable_codegen_falls_back_when_call_target_might_be_a_proc_path.dm"
        ));
        let entry = proc_id(&tree, &ir_module, "/", "entry");

        let module = generate_reachable(&ir_module, &tree, &[entry], &[]).expect("generate selected procedures");

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
        let (tree, ir_module) = analyze(fixture!(
            "programs/reachable_codegen_resolves_dynamic_values_from_typesof_iteration.dm"
        ));
        let initialize = proc_id(&tree, &ir_module, "/datum/controller/global_vars", "Initialize");
        let call = proc_id(&tree, &ir_module, "/", "call");
        let typesof = proc_id(&tree, &ir_module, "/", "typesof");
        let unrelated = proc_id(&tree, &ir_module, "/", "unrelated");

        let module = generate_reachable(&ir_module, &tree, &[initialize], &[]).expect("generate selected procedures");

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
        let (tree, ir_module) = analyze(fixture!(
            "programs/reachable_codegen_falls_back_to_every_proc_for_an_opaque_callable.dm"
        ));
        let entry = proc_id(&tree, &ir_module, "/", "entry");
        let otherwise = proc_id(&tree, &ir_module, "/", "otherwise");

        let module = generate_reachable(&ir_module, &tree, &[entry], &[]).expect("generate selected procedures");

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
        assert_eq!(Op::Spawn as u8, 0x2d);
        assert_eq!(Op::EndSpawn as u8, 0x2e);
        assert_eq!(Unary::Dereference as u8, 0x08);
        assert_eq!(Binary::In as u8, 0x17);
        assert_eq!(Builtin::SuperProc as u8, 0x08);
        assert_eq!(Access::Scope as u8, 0x04);
    }
}
