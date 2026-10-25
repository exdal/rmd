use core::{
    location::SourceLine,
    path::{PathFlags, TreePath},
    types::{Identifier, ProcId, Value},
    vars,
};
use std::{
    collections::{HashMap, HashSet, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
};

use codegen::{
    CodeOffset, CompiledFunction, ConstantId, FunctionId, Module, PathId, RegisterId, StringId,
    opcode::{ARGUMENT_KEY, ARGUMENT_VALUE, Access, Binary, Builtin, Op, OutputTargetKind, Unary},
};
use objtree::{ObjectTree, TypeId};

use crate::{
    Fault, FaultKind, GenericValue, Limits, StackFrame,
    bake::Placement,
    heap::{Heap, Object, ObjectId},
    value::{IteratorId, ListData, ListId, ModifiedType, ProcRef, RangeValue, Receiver},
    world::{Position, World},
};

type Result<T, E = Fault> = std::result::Result<T, E>;

#[derive(Debug, Default)]
pub struct Runtime {
    pub heap: Heap,
    pub world: World,
    pub icons: crate::IconStates,
    pub(crate) ui: crate::ui::Panel,
    pub(crate) groups: std::collections::HashMap<TypeId, u32>,
    pub(crate) node_groups: Vec<crate::bake::NodeGroup>,
    pub(crate) rotations: Vec<crate::bake::Rotation>,
    pub(crate) defining_groups: bool,
    output: Vec<String>,
    pub(crate) modular_roots: Vec<TypeId>,
    pub(crate) placement: crate::bake::PlacementState,
    pub(crate) coordinate_modules: HashMap<Position, Placement>,
    pub(crate) pending_placements: crate::bake::PendingPlacements,
    pub(crate) global: Option<ObjectId>,
    pub(crate) world_object: Option<ObjectId>,
    pub(crate) profile: Option<ObjectId>,
}

impl Runtime {
    pub fn output(&self) -> &[String] {
        &self.output
    }

    pub fn take_output(&mut self) -> Vec<String> {
        std::mem::take(&mut self.output)
    }

    /// `seed` is the atom a hook was called about, which is what its randomness and its cacheability
    /// are measured against. It is not `src`: a hook's `src` is the profile, which has no position.
    #[allow(clippy::too_many_arguments)]
    pub fn run(
        &mut self, tree: &ObjectTree, module: &Module, proc: ProcId, src: Option<ObjectId>, seed: Option<ObjectId>,
        args: Vec<GenericValue>, limits: Limits,
    ) -> Result<GenericValue> {
        self.ensure_global()?;
        self.ensure_world(tree)?;
        self.begin_journal();

        let result = {
            let mut evaluator = Evaluator::new(self, tree, module, limits, seed);
            evaluator.call(
                proc,
                src,
                args.into_iter().map(|value| (None, value)).collect::<Vec<_>>(),
            )
        };

        if result.is_ok() {
            self.commit_journal();
        } else {
            self.rollback_journal();
        }

        result
    }

    pub(crate) fn begin_journal(&mut self) {
        self.heap.begin();
        self.pending_placements.mark();
    }

    pub(crate) fn commit_journal(&mut self) {
        self.heap.commit();
    }

    pub(crate) fn rollback_journal(&mut self) {
        self.heap.rollback();
        self.pending_placements.discard();
    }

    pub fn run_world(
        &mut self, tree: &ObjectTree, module: &Module, proc: ProcId, args: Vec<GenericValue>, limits: Limits,
    ) -> Result<GenericValue> {
        let world = self.ensure_world(tree)?;

        self.run(tree, module, proc, Some(world), Some(world), args, limits)
    }

    pub fn set_var(
        &mut self, tree: &ObjectTree, module: &Module, target: GenericValue, name: Identifier, value: GenericValue,
        limits: Limits,
    ) -> Result<()> {
        self.ensure_global()?;

        let mut evaluator = Evaluator::new(self, tree, module, limits, None);
        evaluator.write_field(target, name, value)
    }

    pub(crate) fn ensure_global(&mut self) -> Result<()> {
        if self.global.is_none() {
            self.global = Some(
                self.heap
                    .alloc_object(Object::new(TypeId::ROOT))
                    .map_err(Fault::detached)?,
            );
        }

        Ok(())
    }

    fn ensure_world(&mut self, tree: &ObjectTree) -> Result<ObjectId> {
        if let Some(world) = self.world_object {
            return Ok(world);
        }

        let ty = tree.roots().world.unwrap_or(TypeId::ROOT);
        let world = self.heap.alloc_object(Object::new(ty)).map_err(Fault::detached)?;
        self.world_object = Some(world);

        Ok(world)
    }

    /// Allocate the bake profile and run its `New()`. The id is stored before `New()` runs so that
    /// `demir_profile()` answers inside it, and no journal is opened: the profile outlives every
    /// transaction taken against it, and `Heap::rollback` truncates by id.
    pub(crate) fn create_profile(
        &mut self, tree: &ObjectTree, module: &Module, ty: TypeId, constructor: Option<ProcId>, limits: Limits,
    ) -> Result<ObjectId> {
        self.modular_roots.clear();
        self.coordinate_modules.clear();
        self.ensure_global()?;
        self.ensure_world(tree)?;

        let id = self.heap.alloc_object(Object::new(ty)).map_err(Fault::detached)?;
        self.profile = Some(id);

        self.defining_groups = true;
        let result = {
            let mut evaluator = Evaluator::new(self, tree, module, limits, None);
            match constructor {
                Some(proc) => evaluator
                    .call_function_for_proc(proc, Receiver::Object(id), None, Vec::new())
                    .map(|_| ()),
                None => Ok(()),
            }
        };
        self.defining_groups = false;

        result?;

        Ok(id)
    }

    pub fn constant(&mut self, value: &Value, limits: Limits) -> std::result::Result<GenericValue, FaultKind> {
        self.constant_inner(value, limits, 0)
    }

    fn constant_inner(
        &mut self, value: &Value, limits: Limits, depth: usize,
    ) -> std::result::Result<GenericValue, FaultKind> {
        if depth >= limits.constant_depth {
            return Err(FaultKind::Memory);
        }

        Ok(match value {
            Value::Null => GenericValue::Null,
            Value::Num(number) => GenericValue::Num(*number),
            Value::Text(text) => GenericValue::Text(text.as_str().into()),
            Value::Resource(path) => GenericValue::Resource(path.as_str().into()),
            Value::Path(path) => GenericValue::Path(Box::new(path.clone())),
            Value::Unevaluated => return Err(FaultKind::Unsupported("nonconstant initializer".into())),
            Value::List(entries) => {
                let entries = entries
                    .iter()
                    .map(|entry| {
                        Ok((
                            self.constant_inner(&entry.key, limits, depth + 1)?,
                            entry
                                .value
                                .as_ref()
                                .map(|value| self.constant_inner(value, limits, depth + 1))
                                .transpose()?,
                        ))
                    })
                    .collect::<std::result::Result<Vec<_>, FaultKind>>()?;
                GenericValue::List(self.heap.alloc_list(ListData::new(entries))?)
            },
        })
    }
}

fn list_operation(
    op: Binary, entries: &mut Vec<(GenericValue, Option<GenericValue>)>,
    right: Vec<(GenericValue, Option<GenericValue>)>,
) {
    // TODO: THE VM SHOULD NOT HANDLE THIS PLEASE MOVE IT TO THE FUCKING SEMA
    match op {
        Binary::Add => entries.extend(right),
        Binary::Sub => entries.retain(|(key, _)| !right.iter().any(|(value, _)| value == key)),
        Binary::BitAnd => entries.retain(|(key, _)| right.iter().any(|(value, _)| value == key)),
        Binary::BitOr => {
            for entry in right {
                if !entries.iter().any(|(value, _)| *value == entry.0) {
                    entries.push(entry);
                }
            }
        },
        Binary::BitXor => {
            for entry in right {
                if entries.iter().any(|(value, _)| *value == entry.0) {
                    entries.retain(|(value, _)| *value != entry.0);
                } else {
                    entries.push(entry);
                }
            }
        },
        _ => {},
    }
}

fn fault_at(module: &Module, current: Option<ProcId>, offset: Option<CodeOffset>, kind: FaultKind) -> Fault {
    let frames = current
        .map(|proc| stack_frame(module, proc, offset))
        .into_iter()
        .collect::<Vec<_>>();

    Fault {
        kind,
        location: frames.first().map(|frame| frame.location).unwrap_or_default(),
        frames,
    }
}

fn stack_frame(module: &Module, proc: ProcId, offset: Option<CodeOffset>) -> StackFrame {
    let location = offset
        .and_then(|offset| module.line_at(offset))
        .map(SourceLine::location)
        .or_else(|| module.function_for_proc(proc).map(|function| function.location))
        .unwrap_or_default();

    StackFrame { proc, offset, location }
}

#[derive(Debug)]
pub(crate) struct Frame {
    registers: Vec<GenericValue>,
    variables: HashMap<Identifier, GenericValue>,
    pub(crate) src: Receiver,
    usr: Option<ObjectId>,
    args: Option<ListId>,
    extra_args: Vec<GenericValue>,
    dot: GenericValue,
    function: FunctionId,
    proc: ProcId,
    ip: usize,
    start: usize,
    end: usize,
}

#[derive(Debug, Clone)]
struct IteratorState {
    entries: Vec<(GenericValue, Option<GenericValue>)>,
    index: usize,
    current: Option<(GenericValue, Option<GenericValue>)>,
    ty: Option<TreePath>,
    value_is_associated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flow {
    Continue,
    Return,
    EndSpawn,
}

pub(crate) struct Evaluator<'a> {
    pub runtime: &'a mut Runtime,
    pub tree: &'a ObjectTree,
    pub module: &'a Module,
    pub limits: Limits,
    pub instruction_budget_remaining: u64,
    pub allocations: usize,
    pub depth: usize,
    pub current: Option<ProcId>,
    pub current_receiver: Receiver,
    pub current_offset: Option<CodeOffset>,
    pub is_cacheable: bool, // memoization
    pub position_sensitive: bool,
    pub origin: Option<ObjectId>,
    pub appearance_reads: HashSet<ObjectId>,
    pub rng: u64,
    thrown: Option<GenericValue>,
    iterators: Vec<IteratorState>,
    vars_lists: HashMap<ObjectId, ListId>,
}

struct CallFrameGuard<'e, 'a> {
    eval: &'e mut Evaluator<'a>,
    previous_proc: Option<ProcId>,
    previous_receiver: Receiver,
    previous_offset: Option<CodeOffset>,
}

impl<'e, 'a> CallFrameGuard<'e, 'a> {
    fn enter(eval: &'e mut Evaluator<'a>, proc: ProcId, receiver: Receiver) -> Self {
        let previous_proc = eval.current.replace(proc);
        let previous_receiver = std::mem::replace(&mut eval.current_receiver, receiver);
        let previous_offset = eval.current_offset.take();
        eval.depth += 1;

        Self {
            eval,
            previous_proc,
            previous_receiver,
            previous_offset,
        }
    }
}

impl CallFrameGuard<'_, '_> {
    fn unwound(&self, mut fault: Fault) -> Fault {
        if let Some(proc) = self.previous_proc {
            fault
                .frames
                .push(stack_frame(self.eval.module, proc, self.previous_offset));
        }

        fault
    }
}

impl<'a> std::ops::Deref for CallFrameGuard<'_, 'a> {
    type Target = Evaluator<'a>;

    fn deref(&self) -> &Self::Target {
        self.eval
    }
}

impl std::ops::DerefMut for CallFrameGuard<'_, '_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.eval
    }
}

impl Drop for CallFrameGuard<'_, '_> {
    fn drop(&mut self) {
        self.eval.depth -= 1;
        self.eval.current = self.previous_proc;
        self.eval.current_receiver = self.previous_receiver;
        self.eval.current_offset = self.previous_offset;
    }
}

impl<'a> Evaluator<'a> {
    pub(crate) fn new(
        runtime: &'a mut Runtime, tree: &'a ObjectTree, module: &'a Module, limits: Limits, src: Option<ObjectId>,
    ) -> Self {
        let mut seed = DefaultHasher::new();
        if let Some(id) = src
            && let Some(object) = runtime.heap.object(id)
        {
            tree.get(object.ty)
                .map(|declaration| declaration.path.to_string())
                .hash(&mut seed);
            runtime.world.position(&runtime.heap, id).hash(&mut seed);
        }

        Self {
            runtime,
            tree,
            module,
            limits,
            instruction_budget_remaining: limits.instruction_budget,
            allocations: 0,
            depth: 0,
            current: None,
            current_receiver: Receiver::None,
            current_offset: None,
            is_cacheable: true,
            position_sensitive: false,
            origin: src,
            appearance_reads: HashSet::new(),
            rng: seed.finish().max(1),
            thrown: None,
            iterators: Vec::new(),
            vars_lists: HashMap::new(),
        }
    }

    /// Callers add their frames as the fault unwinds through them
    pub fn fault(&self, kind: FaultKind) -> Fault {
        fault_at(self.module, self.current, self.current_offset, kind)
    }

    fn get<'f>(&self, frame: &'f Frame, register: RegisterId) -> Result<&'f GenericValue> {
        let value = frame
            .registers
            .get(usize::from(register.0))
            .ok_or_else(|| self.fault(FaultKind::InvalidReference))?;

        match value {
            GenericValue::Omitted => Ok(&GenericValue::Null),
            _ => Ok(value),
        }
    }

    fn set(&self, frame: &mut Frame, register: RegisterId, value: GenericValue) -> Result<()> {
        let slot = frame
            .registers
            .get_mut(usize::from(register.0))
            .ok_or_else(|| self.fault(FaultKind::InvalidReference))?;

        *slot = value;

        Ok(())
    }

    pub fn charge(&mut self, amount: usize) -> Result<()> {
        self.instruction_budget_remaining = self
            .instruction_budget_remaining
            .checked_sub(amount as u64)
            .ok_or_else(|| self.fault(FaultKind::InstructionBudget))?;

        Ok(())
    }

    pub fn reserve(&mut self, count: usize) -> Result<()> {
        self.allocations = self
            .allocations
            .checked_add(count)
            .filter(|allocations| *allocations <= self.limits.allocations)
            .ok_or_else(|| self.fault(FaultKind::Memory))?;

        Ok(())
    }

    pub fn list(&mut self, entries: Vec<(GenericValue, Option<GenericValue>)>) -> Result<GenericValue> {
        self.reserve(entries.len().max(1))?;
        self.runtime
            .heap
            .alloc_list(ListData::new(entries))
            .map(GenericValue::List)
            .map_err(|kind| self.fault(kind))
    }

    pub fn alist(&mut self, entries: Vec<(GenericValue, Option<GenericValue>)>) -> Result<GenericValue> {
        self.reserve(entries.len().max(1))?;
        self.runtime
            .heap
            .alloc_list(ListData::alist(entries))
            .map(GenericValue::List)
            .map_err(|kind| self.fault(kind))
    }

    pub fn text(&mut self, text: String) -> Result<GenericValue> {
        if text.len() > self.limits.text_bytes {
            return Err(self.fault(FaultKind::Memory));
        }

        self.charge(text.len().div_ceil(32).max(1))?;

        Ok(text.into())
    }

    pub fn number(&self, value: &GenericValue) -> Result<f32> {
        value
            .num()
            .ok_or_else(|| self.fault(FaultKind::InvalidOperation("expected number".into())))
    }

    pub fn random(&mut self) -> f32 {
        self.position_sensitive = true;
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        ((self.rng >> 40) as f32) / 16_777_216.0
    }

    pub fn find_proc(&self, ty: TypeId, name: &Identifier) -> Option<ProcId> {
        let declaration = self
            .tree
            .ancestors(ty)
            .take_while(|declaration| ty == TypeId::ROOT || declaration.id != TypeId::ROOT)
            .find(|declaration| declaration.procs.contains_key(name))?;

        self.module.index.body(declaration.id, name)
    }

    pub fn call(
        &mut self, proc: ProcId, src: Option<ObjectId>, args: Vec<(Option<Identifier>, GenericValue)>,
    ) -> Result<GenericValue> {
        let module = self.module;
        let function = module
            .function_for_proc(proc)
            .ok_or_else(|| self.fault(FaultKind::InvalidReference))?;

        self.call_function(function, src.into(), None, args)
    }

    fn call_function(
        &mut self, function: &CompiledFunction, src: Receiver, usr: Option<ObjectId>,
        args: Vec<(Option<Identifier>, GenericValue)>,
    ) -> Result<GenericValue> {
        if function.external {
            return Err(self.fault(FaultKind::InvalidReference));
        }

        if self.depth >= self.limits.call_depth {
            return Err(self.fault(FaultKind::CallDepth));
        }

        if let Some(id) = function.intrinsic {
            self.charge(1)?;

            let params = function
                .parameter_names
                .iter()
                .map(|id| {
                    self.module
                        .symbols
                        .get(id.0 as usize)
                        .cloned()
                        .unwrap_or_else(|| Identifier::from(""))
                })
                .collect::<Vec<_>>();

            let name = self.function_name(function);
            return self.intrinsic_impl(id, src, &name, &params, args);
        }

        let register_count = usize::from(function.register_count);
        self.reserve(register_count.saturating_add(args.len()).saturating_add(1))?;
        let start = function.address.0 as usize;
        let end = start
            .checked_add(function.length as usize)
            .filter(|end| *end <= self.module.code.len())
            .ok_or_else(|| self.fault(FaultKind::InvalidReference))?;
        let proc = function.proc.ok_or_else(|| self.fault(FaultKind::InvalidReference))?;
        let parameter_count = function.parameter_count as usize;
        let mut registers = vec![GenericValue::Omitted; register_count];
        let mut used = vec![false; args.len()];
        let mut next_positional = 0;
        for index in 0..parameter_count {
            let name = function
                .parameter_names
                .get(index)
                .and_then(|id| self.module.strings.get(id.0 as usize));
            let named = name.and_then(|name| {
                args.iter()
                    .enumerate()
                    .find(|(_, (key, _))| key.as_ref().is_some_and(|key| key.as_str() == name))
            });
            let supplied = named.or_else(|| {
                let found = args
                    .iter()
                    .enumerate()
                    .skip(next_positional)
                    .find(|(_, (key, _))| key.is_none());
                if let Some((position, _)) = found {
                    next_positional = position + 1;
                }
                found
            });

            if let Some((position, (_, value))) = supplied {
                used[position] = true;
                if let Some(slot) = registers.get_mut(index)
                    && !matches!(value, GenericValue::Omitted | GenericValue::Null)
                {
                    *slot = value.clone();
                }
            }
        }
        let extra_args = args
            .iter()
            .zip(used)
            .filter(|(_, used)| !used)
            .map(|((_, value), _)| value.clone())
            .collect::<Vec<_>>();
        let mut frame = Frame {
            registers,
            variables: HashMap::new(),
            src,
            usr,
            args: None,
            extra_args,
            dot: GenericValue::Null,
            function: function.id,
            proc,
            ip: start,
            start,
            end,
        };

        let mut call = CallFrameGuard::enter(self, proc, src);
        call.execute_until(&mut frame, None)
            .map(|_| frame.dot)
            .map_err(|fault| call.unwound(fault))
    }

    fn function_name(&self, function: &CompiledFunction) -> String {
        let proc_name = self
            .module
            .strings
            .get(function.proc_name.0 as usize)
            .map(String::as_str)
            .unwrap_or("<intrinsic>");
        function
            .owner
            .name()
            .map(|owner| format!("{owner}.{proc_name}"))
            .unwrap_or_else(|| proc_name.to_owned())
    }

    fn execute_until(&mut self, frame: &mut Frame, stop: Option<usize>) -> Result<Flow> {
        loop {
            if frame.ip >= frame.end {
                return Ok(Flow::Return);
            }

            if stop == Some(frame.ip) {
                return Ok(Flow::Continue);
            }

            self.charge(1)?;
            let instruction = frame.ip;
            self.current_offset = Some(CodeOffset(instruction as u32));
            let op = self.enum_operand::<Op>(frame, "opcode")?;
            match op {
                Op::Nop => {},
                Op::MoveConst => {
                    let destination = self.register(frame)?;
                    let id = ConstantId(self.u32(frame)?);
                    let value = self
                        .module
                        .constants
                        .get(id.0 as usize)
                        .cloned()
                        .ok_or_else(|| self.fault(FaultKind::InvalidReference))?;
                    let value = self.constant(&value)?;
                    self.set(frame, destination, value)?;
                },
                Op::MoveImm => {
                    let destination = self.register(frame)?;
                    let value = self.f32(frame)?;
                    self.set(frame, destination, value.into())?;
                },
                Op::Move => {
                    let destination = self.register(frame)?;
                    let source = self.register(frame)?;
                    let value = self.get(frame, source)?.clone();
                    self.set(frame, destination, value)?;
                },
                Op::LoadVar => {
                    let destination = self.register(frame)?;
                    let name = self.identifier(frame)?;
                    let value = self.read_name(frame, &name)?;
                    self.set(frame, destination, value)?;
                },
                Op::StoreVar => {
                    let name = self.identifier(frame)?;
                    let source = self.register(frame)?;
                    let value = self.get(frame, source)?.clone();
                    self.write_name(frame, name, value)?;
                },
                Op::InitVar => {
                    let name = self.identifier(frame)?;
                    let source = self.register(frame)?;
                    let value = self.get(frame, source)?.clone();
                    self.initialize_name(frame, name, value)?;
                },
                Op::LoadBuiltin => {
                    let destination = self.register(frame)?;
                    let builtin = self.enum_operand::<Builtin>(frame, "builtin")?;
                    let value = self.builtin_value(frame, builtin)?;
                    self.set(frame, destination, value)?;
                },
                Op::StoreBuiltin => {
                    let builtin = self.enum_operand::<Builtin>(frame, "builtin")?;
                    let source = self.register(frame)?;
                    let value = self.get(frame, source)?.clone();
                    match builtin {
                        Builtin::ThisProc => frame.dot = value,
                        _ => return Err(self.fault(FaultKind::InvalidOperation("builtin is not assignable".into()))),
                    }
                },
                Op::Interpolate => {
                    let destination = self.register(frame)?;
                    let value_count = self.u32(frame)? as usize;
                    let chunk_count = self.u32(frame)? as usize;
                    let chunks = (0..chunk_count)
                        .map(|_| self.string(frame).map(ToOwned::to_owned))
                        .collect::<Result<Vec<_>>>()?;
                    let registers = (0..value_count)
                        .map(|_| self.register(frame))
                        .collect::<Result<Vec<_>>>()?;
                    let mut output = String::new();
                    for (index, chunk) in chunks.iter().enumerate() {
                        output.push_str(chunk);
                        if let Some(value) = registers.get(index) {
                            let value = self.get(frame, *value)?;
                            if matches!(value, GenericValue::Object(_) | GenericValue::Proc(_)) {
                                self.is_cacheable = false;
                            }

                            output.push_str(&value.display());
                        }

                        if output.len() > self.limits.text_bytes {
                            return Err(self.fault(FaultKind::Memory));
                        }
                    }

                    let value = self.text(output)?;
                    self.set(frame, destination, value)?;
                },
                Op::Unary => {
                    let destination = self.register(frame)?;
                    let op = self.enum_operand::<Unary>(frame, "unary")?;
                    let source = self.register(frame)?;
                    let value = self.get(frame, source)?.clone();
                    let value = self.unary(op, value)?;
                    self.set(frame, destination, value)?;
                },
                Op::Binary => {
                    let destination = self.register(frame)?;
                    let op = self.enum_operand::<Binary>(frame, "binary")?;
                    let left = self.register(frame)?;
                    let right = self.register(frame)?;
                    let left = self.get(frame, left)?.clone();
                    let right = self.get(frame, right)?.clone();
                    let value = self.binary(op, left, right)?;
                    self.set(frame, destination, value)?;
                },
                Op::BinaryI => {
                    let destination = self.register(frame)?;
                    let op = self.enum_operand::<Binary>(frame, "binary")?;
                    let left = self.register(frame)?;
                    let right = self.f32(frame)?;
                    let left = self.get(frame, left)?.clone();
                    let value = self.binary(op, left, right.into())?;
                    self.set(frame, destination, value)?;
                },
                Op::CompoundBinary => {
                    let destination = self.register(frame)?;
                    let op = self.enum_operand::<Binary>(frame, "binary")?;
                    let left = self.register(frame)?;
                    let right = self.register(frame)?;
                    let left = self.get(frame, left)?.clone();
                    let right = self.get(frame, right)?.clone();
                    let value = self.compound_binary(op, left, right)?;
                    self.set(frame, destination, value)?;
                },
                Op::CompoundBinaryI => {
                    let destination = self.register(frame)?;
                    let op = self.enum_operand::<Binary>(frame, "binary")?;
                    let left = self.register(frame)?;
                    let right = self.f32(frame)?;
                    let left = self.get(frame, left)?.clone();
                    let value = self.compound_binary(op, left, right.into())?;
                    self.set(frame, destination, value)?;
                },
                Op::AccessField => {
                    let destination = self.register(frame)?;
                    let name = self.identifier(frame)?;
                    let access = self.enum_operand::<Access>(frame, "access")?;
                    let method = self.boolean(frame)?;
                    let object = self.register(frame)?;
                    let object = self.get(frame, object)?.clone();
                    let value = self.access_field(object, name, access, method)?;
                    self.set(frame, destination, value)?;
                },
                Op::Initial => {
                    let destination = self.register(frame)?;
                    let has_object = self.boolean(frame)?;
                    let name = self.identifier(frame)?;
                    let object = if has_object { Some(self.register(frame)?) } else { None };
                    let object = object.map(|object| self.get(frame, object).cloned()).transpose()?;
                    let value = self.initial_value(frame, object, &name)?;
                    self.set(frame, destination, value)?;
                },
                Op::Index => {
                    let destination = self.register(frame)?;
                    let conditional = self.boolean(frame)?;
                    let object = self.register(frame)?;
                    let key = self.register(frame)?;
                    let object = self.get(frame, object)?.clone();
                    let key = self.get(frame, key)?.clone();
                    let value = if conditional && object == GenericValue::Null {
                        GenericValue::Null
                    } else {
                        self.get_index(object, key)?
                    };
                    self.set(frame, destination, value)?;
                },
                Op::IndexI => {
                    let destination = self.register(frame)?;
                    let conditional = self.boolean(frame)?;
                    let object = self.register(frame)?;
                    let key = self.f32(frame)?;
                    let object = self.get(frame, object)?.clone();
                    let value = if conditional && object == GenericValue::Null {
                        GenericValue::Null
                    } else {
                        self.get_index(object, key.into())?
                    };
                    self.set(frame, destination, value)?;
                },
                Op::Call => {
                    let destination = self.register(frame)?;
                    let conditional = self.boolean(frame)?;
                    let callee = self.register(frame)?;
                    let args = self.arguments(frame)?;
                    let callee = self.get(frame, callee)?.clone();
                    let value = if conditional && callee == GenericValue::Null {
                        GenericValue::Null
                    } else {
                        self.invoke(callee, args, frame)?
                    };
                    self.set(frame, destination, value)?;
                },
                Op::FunctionCall => {
                    let destination = self.register(frame)?;
                    let function = FunctionId(self.u32(frame)?);
                    let args = self.arguments(frame)?;
                    let value = self.invoke_function(function, args, frame)?;
                    self.set(frame, destination, value)?;
                },
                Op::SuperCall => {
                    let destination = self.register(frame)?;
                    let forwards_extra_args = self.boolean(frame)?;
                    let mut args = self.arguments(frame)?;
                    if forwards_extra_args {
                        args.extend(frame.extra_args.iter().cloned().map(|value| (None, value)));
                    }

                    let value = self.invoke_super(args, frame)?;
                    self.set(frame, destination, value)?;
                },
                Op::New => {
                    let destination = self.register(frame)?;
                    let has_type = self.boolean(frame)?;
                    let ty = if has_type {
                        let ty = self.register(frame)?;
                        self.get(frame, ty)?.clone()
                    } else {
                        GenericValue::Null
                    };
                    let args = self.arguments(frame)?;
                    let value = self.new_object(ty, args)?;
                    self.set(frame, destination, value)?;
                },
                Op::ModifiedType => {
                    let destination = self.register(frame)?;
                    let path = self.path(frame)?;
                    let count = self.u32(frame)? as usize;
                    let overrides = (0..count)
                        .map(|_| {
                            let name = self.identifier(frame)?;
                            let register = self.register(frame)?;
                            Ok((name, self.get(frame, register)?.clone()))
                        })
                        .collect::<Result<Vec<_>>>()?;
                    let value = GenericValue::ModifiedType(Box::new(ModifiedType { path, overrides }));
                    self.set(frame, destination, value)?;
                },
                Op::MakeList => {
                    let destination = self.register(frame)?;
                    let args = self.argument_values(frame)?;
                    let value = self.make_list(args)?;
                    self.set(frame, destination, value)?;
                },
                Op::Pick => {
                    let destination = self.register(frame)?;
                    let count = self.u32(frame)? as usize;
                    let mut weighted = Vec::with_capacity(count);
                    let mut operands = Vec::with_capacity(count);
                    for _ in 0..count {
                        let is_weighted = self.boolean(frame)?;
                        weighted.push(is_weighted);
                        if is_weighted {
                            let weight = self.register(frame)?;
                            operands.push(self.get(frame, weight)?.clone());
                        }

                        let value = self.register(frame)?;
                        operands.push(self.get(frame, value)?.clone());
                    }

                    let value = self.pick(&weighted, operands)?;
                    self.set(frame, destination, value)?;
                },
                Op::InRange => {
                    let destination = self.register(frame)?;
                    let has_step = self.boolean(frame)?;
                    let value = self.register(frame)?;
                    let start = self.register(frame)?;
                    let end = self.register(frame)?;
                    let step = if has_step { Some(self.register(frame)?) } else { None };
                    let value = self.get(frame, value)?.clone();
                    let start = self.get(frame, start)?.clone();
                    let end = self.get(frame, end)?.clone();
                    let step = match step {
                        Some(step) => self.get(frame, step)?.clone(),
                        None => 1.into(),
                    };
                    let value = self.in_range(value, start, end, step)?;
                    self.set(frame, destination, value)?;
                },
                Op::Range => {
                    let destination = self.register(frame)?;
                    let has_step = self.boolean(frame)?;
                    let start = self.register(frame)?;
                    let end = self.register(frame)?;
                    let step = if has_step { Some(self.register(frame)?) } else { None };
                    let start = self.number(self.get(frame, start)?)?;
                    let end = self.number(self.get(frame, end)?)?;
                    let step = match step {
                        Some(step) => self.number(self.get(frame, step)?)?,
                        None => 1.0,
                    };
                    self.set(frame, destination, GenericValue::Range(RangeValue { start, end, step }))?;
                },
                Op::IterInit => {
                    let destination = self.register(frame)?;
                    let ty = if self.boolean(frame)? {
                        Some(self.path(frame)?)
                    } else {
                        None
                    };

                    let value_is_associated = self.boolean(frame)?;
                    let list = self.register(frame)?;
                    let value = self.get(frame, list)?.clone();
                    let iterator = self.iterator(value, ty, value_is_associated)?;
                    self.set(frame, destination, GenericValue::Iterator(iterator))?;
                },
                Op::IterNext => {
                    let destination = self.register(frame)?;
                    let iterator = self.register(frame)?;
                    let iterator = self.iterator_id(self.get(frame, iterator)?.clone())?;
                    let value = self.iterator_next(iterator)?;
                    self.set(frame, destination, value.into())?;
                },
                Op::IterValue => {
                    let destination = self.register(frame)?;
                    let iterator = self.register(frame)?;
                    let iterator = self.iterator_id(self.get(frame, iterator)?.clone())?;
                    let value = self.iterator_value(iterator)?;
                    self.set(frame, destination, value)?;
                },
                Op::IterKey => {
                    let destination = self.register(frame)?;
                    let iterator = self.register(frame)?;
                    let iterator = self.iterator_id(self.get(frame, iterator)?.clone())?;
                    let value = self.iterator_key(iterator)?;
                    self.set(frame, destination, value)?;
                },
                Op::RangeTest => {
                    let destination = self.register(frame)?;
                    let current = self.register(frame)?;
                    let end = self.register(frame)?;
                    let step = self.register(frame)?;
                    let current = self.number(self.get(frame, current)?)?;
                    let end = self.number(self.get(frame, end)?)?;
                    let step = self.number(self.get(frame, step)?)?;
                    let value = match step.partial_cmp(&0.0) {
                        Some(std::cmp::Ordering::Greater) => current <= end,
                        Some(std::cmp::Ordering::Less) => current >= end,
                        _ => return Err(self.fault(FaultKind::InvalidOperation("range step must be nonzero".into()))),
                    };
                    self.set(frame, destination, value.into())?;
                },
                Op::SetField => {
                    let name = self.identifier(frame)?;
                    let access = self.enum_operand::<Access>(frame, "access")?;
                    let object = self.register(frame)?;
                    let value = self.register(frame)?;
                    let object = self.get(frame, object)?.clone();
                    let value = self.get(frame, value)?.clone();
                    if !(object == GenericValue::Null && matches!(access, Access::SafeDot | Access::SafeColon)) {
                        self.write_field(object, name, value)?;
                    }
                },
                Op::SetIndex => {
                    let conditional = self.boolean(frame)?;
                    let object = self.register(frame)?;
                    let key = self.register(frame)?;
                    let value = self.register(frame)?;
                    let object = self.get(frame, object)?.clone();
                    let key = self.get(frame, key)?.clone();
                    let value = self.get(frame, value)?.clone();

                    if conditional && object == GenericValue::Null {
                        continue;
                    }

                    self.set_index(object.clone(), key.clone(), value.clone())?;

                    if matches!(object, GenericValue::ArgList(id) | GenericValue::List(id) if Some(id) == frame.args)
                        && let GenericValue::Num(index) = key
                    {
                        let index = (index as usize).wrapping_sub(1);
                        if index < self.function(frame.function)?.parameter_count as usize
                            && let Some(slot) = frame.registers.get_mut(index)
                        {
                            *slot = value;
                        }
                    }
                },
                Op::Jump => {
                    let target = self.jump_target(frame)?;
                    if stop == Some(target) {
                        return Ok(Flow::Continue);
                    }

                    frame.ip = target;
                },
                Op::JumpIfFalse => {
                    let target = self.jump_target(frame)?;
                    let condition = self.register(frame)?;
                    if !self.get(frame, condition)?.truthy() {
                        if stop == Some(target) {
                            return Ok(Flow::Continue);
                        }

                        frame.ip = target;
                    }
                },
                Op::Return => {
                    return Ok(Flow::Return);
                },
                Op::ReturnValue => {
                    let source = self.register(frame)?;
                    frame.dot = self.get(frame, source)?.clone();
                    return Ok(Flow::Return);
                },
                Op::ReturnValueI => {
                    frame.dot = self.f32(frame)?.into();
                    return Ok(Flow::Return);
                },
                Op::ReturnNull => {
                    frame.dot = GenericValue::Null;
                    return Ok(Flow::Return);
                },
                Op::Del => {
                    let source = self.register(frame)?;
                    let value = self.get(frame, source)?.clone();
                    self.delete(value)?;
                },
                Op::Throw => {
                    let source = self.register(frame)?;
                    let value = self.get(frame, source)?.clone();

                    return Err(self.throw(value));
                },
                Op::Output => match self.enum_operand::<OutputTargetKind>(frame, "output target")? {
                    OutputTargetKind::Value | OutputTargetKind::Index => {
                        return Err(self.fault(FaultKind::Blocked("output".into())));
                    },
                    OutputTargetKind::Field => {
                        let name = self.identifier(frame)?;
                        let _access = self.enum_operand::<Access>(frame, "access")?;
                        let object = self.register(frame)?;
                        let value = self.register(frame)?;
                        if *self.get(frame, object)? != GenericValue::World || name.as_str() != vars::LOG {
                            return Err(self.fault(FaultKind::Blocked("output".into())));
                        }

                        let text = self.get(frame, value)?.display();
                        self.runtime.output.push(text);
                    },
                },
                Op::TryCatch => {
                    let body = self.jump_target(frame)?;
                    let catch = self.jump_target(frame)?;
                    let merge = self.jump_target(frame)?;
                    let resume = frame.ip;
                    frame.ip = body;
                    match self.execute_until(frame, Some(merge)) {
                        Ok(Flow::Continue) => {},
                        Ok(flow) => return Ok(flow),
                        Err(fault) if matches!(fault.kind, FaultKind::Thrown(_)) => {
                            frame.ip = catch;
                            match self.execute_until(frame, Some(merge))? {
                                Flow::Continue => {},
                                flow => return Ok(flow),
                            }
                        },
                        Err(fault) => return Err(fault),
                    }
                    frame.ip = resume;
                },
                Op::Spawn => {
                    let body = self.jump_target(frame)?;
                    let resume = frame.ip;
                    frame.ip = body;
                    match self.execute_until(frame, None) {
                        Ok(Flow::Return) => return Ok(Flow::Return),
                        Ok(_) => {},
                        Err(fault) if matches!(fault.kind, FaultKind::Thrown(_)) => self.thrown = None,
                        Err(fault) if fault.kind == FaultKind::Sleep => {},
                        Err(fault) => return Err(fault),
                    }

                    frame.ip = resume;
                },
                Op::EndSpawn => return Ok(Flow::EndSpawn),
                Op::CatchValue => {
                    let destination = self.register(frame)?;
                    let value = self.thrown.take().unwrap_or_default();
                    self.set(frame, destination, value)?;
                },
                Op::Blocked => {
                    let reason = self.string(frame)?.to_owned();
                    return Err(self.fault(FaultKind::Blocked(reason)));
                },
                Op::Trap => {
                    let reason = self.string(frame)?.to_owned();
                    return Err(self.fault(FaultKind::Trap(reason)));
                },
                Op::DefaultParameter => {
                    let parameter = self.register(frame)?;
                    let source = self.register(frame)?;
                    let value = self.get(frame, source)?.clone();
                    let slot = frame
                        .registers
                        .get_mut(usize::from(parameter.0))
                        .ok_or_else(|| self.fault(FaultKind::InvalidReference))?;
                    if *slot == GenericValue::Omitted {
                        *slot = value;
                    }
                },
            }
        }
    }

    fn bytes<const N: usize>(&self, frame: &mut Frame) -> Result<[u8; N]> {
        let end = frame
            .ip
            .checked_add(N)
            .ok_or_else(|| self.fault(FaultKind::InvalidReference))?;
        let bytes = self
            .module
            .code
            .get(frame.ip..end)
            .filter(|_| end <= frame.end)
            .ok_or_else(|| self.fault(FaultKind::InvalidReference))?;
        frame.ip = end;

        Ok(bytes.try_into().expect("slice of length N"))
    }

    fn u8(&self, frame: &mut Frame) -> Result<u8> {
        let [value] = self.bytes::<1>(frame)?;

        Ok(value)
    }

    fn u32(&self, frame: &mut Frame) -> Result<u32> {
        Ok(u32::from_le_bytes(self.bytes::<4>(frame)?))
    }

    fn f32(&self, frame: &mut Frame) -> Result<f32> {
        Ok(f32::from_le_bytes(self.bytes::<4>(frame)?))
    }

    fn register(&self, frame: &mut Frame) -> Result<RegisterId> {
        Ok(RegisterId(u16::from_le_bytes(self.bytes::<2>(frame)?)))
    }

    fn enum_operand<T>(&self, frame: &mut Frame, kind: &str) -> Result<T>
    where
        T: TryFrom<u8, Error = u8>,
    {
        let value = self.u8(frame)?;
        T::try_from(value).map_err(|value| {
            self.fault(FaultKind::InvalidOperation(format!(
                "invalid {kind} operand 0x{value:02x}"
            )))
        })
    }

    fn boolean(&self, frame: &mut Frame) -> Result<bool> {
        match self.u8(frame)? {
            0 => Ok(false),
            1 => Ok(true),
            value => Err(self.fault(FaultKind::InvalidOperation(format!("invalid boolean operand {value}")))),
        }
    }

    fn string<'b>(&self, frame: &mut Frame) -> Result<&'b str>
    where
        'a: 'b,
    {
        let id = StringId(self.u32(frame)?);
        self.module
            .strings
            .get(id.0 as usize)
            .map(String::as_str)
            .ok_or_else(|| self.fault(FaultKind::InvalidReference))
    }

    fn identifier(&self, frame: &mut Frame) -> Result<Identifier> {
        let id = StringId(self.u32(frame)?);
        self.module
            .symbols
            .get(id.0 as usize)
            .cloned()
            .ok_or_else(|| self.fault(FaultKind::InvalidReference))
    }

    fn path(&self, frame: &mut Frame) -> Result<TreePath> {
        let id = PathId(self.u32(frame)?);
        self.module
            .paths
            .get(id.0 as usize)
            .cloned()
            .ok_or_else(|| self.fault(FaultKind::InvalidReference))
    }

    fn function(&self, id: FunctionId) -> Result<&CompiledFunction> {
        self.module
            .function(id)
            .ok_or_else(|| self.fault(FaultKind::InvalidReference))
    }

    fn jump_target(&self, frame: &mut Frame) -> Result<usize> {
        let target = self.u32(frame)? as usize;
        if target < frame.start || target >= frame.end {
            return Err(self.fault(FaultKind::InvalidOperation("jump outside function".into())));
        }
        Ok(target)
    }
}

impl Evaluator<'_> {
    pub(crate) fn constant(&mut self, value: &Value) -> Result<GenericValue> {
        let mut pending = vec![value];
        while let Some(value) = pending.pop() {
            match value {
                Value::Text(text) | Value::Resource(text) if text.len() > self.limits.text_bytes => {
                    return Err(self.fault(FaultKind::Memory));
                },
                Value::List(entries) => {
                    self.reserve(entries.len().saturating_mul(2).saturating_add(1))?;
                    for entry in entries {
                        pending.push(&entry.key);
                        if let Some(value) = &entry.value {
                            pending.push(value);
                        }
                    }
                },
                _ => {},
            }
        }

        self.runtime
            .constant(value, self.limits)
            .map_err(|kind| self.fault(kind))
    }

    pub(crate) fn object_mut(&mut self, id: ObjectId) -> Result<&mut Object> {
        let (module, current, offset) = (self.module, self.current, self.current_offset);
        self.runtime
            .heap
            .object_mut(id)
            .map_err(|kind| fault_at(module, current, offset, kind))
    }

    pub(crate) fn list_mut(&mut self, id: ListId) -> Result<&mut ListData> {
        let (module, current, offset) = (self.module, self.current, self.current_offset);
        self.runtime
            .heap
            .list_mut(id)
            .map_err(|kind| fault_at(module, current, offset, kind))
    }

    fn name_target(&self, frame: &Frame, name: &Identifier) -> GenericValue {
        if let Some(id) = frame.src.list()
            && name.as_str() == vars::LEN
        {
            return GenericValue::List(id);
        }

        if let Some(src) = frame.src.object()
            && let Some(object) = self.runtime.heap.object(src)
            && (object.vars.contains_key(name)
                || self
                    .tree
                    .var_declaration(object.ty, name)
                    .is_some_and(|(owner, _)| owner.id != TypeId::ROOT)
                || matches!(
                    name.as_str(),
                    vars::LOC
                        | vars::CONTENTS
                        | vars::X
                        | vars::Y
                        | vars::Z
                        | vars::TYPE
                        | vars::PARENT_TYPE
                        | vars::OVERLAYS
                        | vars::UNDERLAYS
                        | vars::APPEARANCE
                ))
        {
            return GenericValue::Object(src);
        }

        GenericValue::Global
    }

    pub(crate) fn receiver_type(&self, receiver: Receiver) -> Option<TypeId> {
        match receiver {
            Receiver::None => None,
            Receiver::Object(id) => self.runtime.heap.object(id).map(|object| object.ty),
            Receiver::List(id) => match self.runtime.heap.list(id)?.kind {
                crate::value::ListKind::List | crate::value::ListKind::Vars(_) => self.tree.roots().list,
                crate::value::ListKind::Alist => self.tree.roots().alist,
            },
        }
    }

    fn read_name(&mut self, frame: &Frame, name: &Identifier) -> Result<GenericValue> {
        if name.as_str().starts_with("__dmed_frame_local_") {
            return Ok(frame.variables.get(name).cloned().unwrap_or_default());
        }

        self.read_field(self.name_target(frame, name), name)
    }

    fn write_name(&mut self, frame: &mut Frame, name: Identifier, value: GenericValue) -> Result<()> {
        if name.as_str().starts_with("__dmed_frame_local_") {
            frame.variables.insert(name, value);
            return Ok(());
        }

        self.write_field(self.name_target(frame, &name), name, value)
    }

    fn initialize_name(&mut self, frame: &mut Frame, name: Identifier, value: GenericValue) -> Result<()> {
        if name.as_str().starts_with("__dmed_frame_local_") {
            frame.variables.insert(name, value);
            return Ok(());
        }

        let target = self.name_target(frame, &name);
        let storage = match target {
            GenericValue::Object(id) => id,
            GenericValue::Global => self
                .runtime
                .global
                .ok_or_else(|| self.fault(FaultKind::InvalidReference))?,
            _ => return Err(self.fault(FaultKind::InvalidReference)),
        };

        if !self
            .runtime
            .heap
            .object(storage)
            .is_some_and(|object| object.vars.contains_key(&name))
        {
            self.object_mut(storage)?.vars.insert(name, value);
        }

        Ok(())
    }

    fn builtin_value(&mut self, frame: &mut Frame, builtin: Builtin) -> Result<GenericValue> {
        Ok(match builtin {
            Builtin::Src => frame.src.value(),
            Builtin::Usr => frame.usr.map(GenericValue::Object).unwrap_or_default(),
            Builtin::World => GenericValue::World,
            Builtin::Global => GenericValue::Global,
            Builtin::Args => {
                if let Some(args) = frame.args {
                    GenericValue::List(args)
                } else {
                    let entries = self
                        .frame_args(frame)
                        .into_iter()
                        .map(|value| (value, None))
                        .collect::<Vec<_>>();
                    let value = self.list(entries)?;
                    let GenericValue::List(id) = value else {
                        return Err(self.fault(FaultKind::InvalidReference));
                    };
                    frame.args = Some(id);
                    GenericValue::List(id)
                }
            },
            Builtin::Callee => GenericValue::Proc(ProcRef {
                src: frame.src,
                proc: frame.proc,
            }),
            Builtin::Caller => return Err(self.fault(FaultKind::Unsupported("caller".into()))),
            Builtin::ThisProc => frame.dot.clone(),
            Builtin::SuperProc => return Err(self.fault(FaultKind::Unsupported("super proc value".into()))),
        })
    }

    fn frame_args(&self, frame: &Frame) -> Vec<GenericValue> {
        if let Some(list) = frame.args.and_then(|id| self.runtime.heap.list(id)) {
            return list.entries.iter().map(|(value, _)| value.clone()).collect::<Vec<_>>();
        }

        let count = self
            .module
            .function(frame.function)
            .map_or(0, |function| function.parameter_count as usize);
        frame
            .registers
            .iter()
            .take(count)
            .map(|value| {
                if *value == GenericValue::Omitted {
                    GenericValue::Null
                } else {
                    value.clone()
                }
            })
            .chain(frame.extra_args.iter().cloned())
            .collect::<Vec<_>>()
    }

    fn arguments(&mut self, frame: &mut Frame) -> Result<Vec<(Option<Identifier>, GenericValue)>> {
        Ok(self
            .argument_values(frame)?
            .into_iter()
            .map(|(key, value)| match key {
                Some(GenericValue::Text(name)) => (Some(name.as_ref().into()), value),
                Some(key) => (None, key),
                None => (None, value),
            })
            .collect::<Vec<_>>())
    }

    fn argument_values(&mut self, frame: &mut Frame) -> Result<Vec<(Option<GenericValue>, GenericValue)>> {
        let count = self.u32(frame)? as usize;
        let mut args = Vec::with_capacity(count);
        for _ in 0..count {
            let shape = self.u8(frame)?;
            if shape & !(ARGUMENT_KEY | ARGUMENT_VALUE) != 0 {
                return Err(self.fault(FaultKind::InvalidOperation(format!(
                    "invalid argument shape 0x{shape:02x}"
                ))));
            }

            let key = if shape & ARGUMENT_KEY != 0 {
                let key = self.register(frame)?;
                Some(self.get(frame, key)?.clone())
            } else {
                None
            };

            let value = if shape & ARGUMENT_VALUE != 0 {
                let value = self.register(frame)?;
                self.get(frame, value)?.clone()
            } else {
                GenericValue::Omitted
            };

            if let GenericValue::ArgList(id) = value {
                let entries = self
                    .runtime
                    .heap
                    .list(id)
                    .map(|list| list.entries.clone())
                    .ok_or_else(|| self.fault(FaultKind::InvalidReference))?;
                self.charge(entries.len())?;
                args.extend(
                    entries
                        .into_iter()
                        .map(|(entry, association)| match (entry, association) {
                            (name, Some(value)) => (Some(name), value),
                            (value, _) => (None, value),
                        }),
                );
            } else {
                args.push((key, value));
            }
        }

        Ok(args)
    }

    fn invoke_function(
        &mut self, id: FunctionId, args: Vec<(Option<Identifier>, GenericValue)>, frame: &mut Frame,
    ) -> Result<GenericValue> {
        let module = self.module;
        let function = module
            .function(id)
            .ok_or_else(|| self.fault(FaultKind::InvalidReference))?;
        if !function.external {
            return self.call_function(function, Receiver::None, frame.usr, args);
        }

        let identifier = module
            .symbols
            .get(function.name.0 as usize)
            .ok_or_else(|| self.fault(FaultKind::InvalidReference))?;

        // `initial(local)` is lowered as an external call because it has no declaration to
        // inspect
        if identifier.as_str() == "initial" {
            return Ok(args.first().map(|(_, value)| value.clone()).unwrap_or_default());
        }

        let src_type = self.receiver_type(frame.src);

        if let Some(proc) = src_type.and_then(|ty| self.find_proc(ty, identifier)) {
            return self.call_function_for_proc(proc, frame.src, frame.usr, args);
        }

        if let Some(proc) = self.find_proc(TypeId::ROOT, identifier) {
            return self.call_function_for_proc(proc, Receiver::None, frame.usr, args);
        }

        Err(self.fault(FaultKind::MissingProc(identifier.as_str().to_owned())))
    }

    fn call_function_for_proc(
        &mut self, proc: ProcId, src: Receiver, usr: Option<ObjectId>, args: Vec<(Option<Identifier>, GenericValue)>,
    ) -> Result<GenericValue> {
        let module = self.module;
        let function = module
            .function_for_proc(proc)
            .ok_or_else(|| self.fault(FaultKind::InvalidReference))?;

        self.call_function(function, src, usr, args)
    }

    fn invoke(
        &mut self, callee: GenericValue, args: Vec<(Option<Identifier>, GenericValue)>, frame: &mut Frame,
    ) -> Result<GenericValue> {
        match callee {
            GenericValue::Proc(reference) => {
                self.call_function_for_proc(reference.proc, reference.src, frame.usr, args)
            },
            GenericValue::Path(path) if path.flags.intersects(PathFlags::IS_PROC | PathFlags::IS_VERB) => {
                let name = path
                    .name()
                    .cloned()
                    .ok_or_else(|| self.fault(FaultKind::InvalidReference))?;
                let owner = TreePath::new(path.declaration_owner().to_vec(), true);
                let proc = self
                    .tree
                    .id_of(&owner)
                    .and_then(|ty| self.find_proc(ty, &name))
                    .ok_or_else(|| self.fault(FaultKind::MissingProc(path.to_string())))?;
                self.call_function_for_proc(proc, frame.src, frame.usr, args)
            },
            _ => Err(self.fault(FaultKind::Blocked("dynamic call".into()))),
        }
    }

    fn invoke_super(&mut self, args: Vec<(Option<Identifier>, GenericValue)>, frame: &Frame) -> Result<GenericValue> {
        let module = self.module;
        let function = module
            .function(frame.function)
            .ok_or_else(|| self.fault(FaultKind::InvalidReference))?;
        let parent = self
            .tree
            .id_of(&function.owner)
            .and_then(|id| self.tree.get(id))
            .and_then(|declaration| declaration.parent);
        let name = module
            .symbols
            .get(function.proc_name.0 as usize)
            .cloned()
            .ok_or_else(|| self.fault(FaultKind::InvalidReference))?;
        let proc = function
            .previous
            .or_else(|| parent.and_then(|parent| self.find_proc(parent, &name)));

        match proc {
            Some(proc) => self.call_function_for_proc(proc, frame.src, frame.usr, args),
            None => Ok(GenericValue::Null),
        }
    }

    /// `obj.foo()` finds the proc `foo`, and `obj.foo` finds the var when the type has one of each
    fn access_field(
        &mut self, object: GenericValue, name: Identifier, access: Access, method: bool,
    ) -> Result<GenericValue> {
        if object == GenericValue::Null && matches!(access, Access::SafeDot | Access::SafeColon) {
            return Ok(GenericValue::Null);
        }

        match object {
            GenericValue::Object(id) => {
                let has_var = !method
                    && self.runtime.heap.object(id).is_some_and(|object| {
                        object.vars.contains_key(&name) || self.tree.var_declaration(object.ty, &name).is_some()
                    });
                if !has_var
                    && let Some(proc) = self
                        .runtime
                        .heap
                        .object(id)
                        .and_then(|object| self.find_proc(object.ty, &name))
                {
                    Ok(GenericValue::Proc(ProcRef {
                        src: Receiver::Object(id),
                        proc,
                    }))
                } else {
                    self.read_field(GenericValue::Object(id), &name)
                }
            },
            GenericValue::List(id) | GenericValue::ArgList(id) if name.as_str() == vars::LEN => {
                self.read_field(GenericValue::List(id), &name)
            },
            GenericValue::List(id) | GenericValue::ArgList(id) => {
                let receiver = Receiver::List(id);
                let proc = self
                    .receiver_type(receiver)
                    .and_then(|ty| self.find_proc(ty, &name))
                    .ok_or_else(|| self.fault(FaultKind::MissingProc(format!("list.{name}"))))?;
                Ok(GenericValue::Proc(ProcRef { src: receiver, proc }))
            },
            GenericValue::Global => {
                let has_var = !method && self.tree.var_declaration(TypeId::ROOT, &name).is_some();
                if !has_var && let Some(proc) = self.find_proc(TypeId::ROOT, &name) {
                    Ok(GenericValue::Proc(ProcRef {
                        src: Receiver::None,
                        proc,
                    }))
                } else {
                    self.read_field(GenericValue::Global, &name)
                }
            },
            GenericValue::World => match self.world_type().and_then(|ty| self.find_proc(ty, &name)) {
                Some(proc) => Ok(GenericValue::Proc(ProcRef {
                    src: self.runtime.world_object.into(),
                    proc,
                })),
                None => self.read_field(GenericValue::World, &name),
            },
            value => self.read_field(value, &name),
        }
    }

    fn initial_value(
        &mut self, frame: &Frame, object: Option<GenericValue>, name: &Identifier,
    ) -> Result<GenericValue> {
        let object = object.unwrap_or_else(|| self.name_target(frame, name));
        let ty = match object {
            GenericValue::Object(id) => self.runtime.heap.object(id).map(|object| object.ty),
            GenericValue::Global => Some(TypeId::ROOT),
            GenericValue::Path(path) => self.tree.id_of(&path),
            _ => None,
        };
        if let Some(ty) = ty {
            match name.as_str() {
                vars::TYPE => {
                    return Ok(self
                        .tree
                        .get(ty)
                        .map(|declaration| GenericValue::Path(Box::new(declaration.path.clone())))
                        .unwrap_or_default());
                },
                vars::PARENT_TYPE => return Ok(self.parent_path(ty)),
                _ => {},
            }
        }
        let value = ty
            .and_then(|ty| self.tree.var_inherited(ty, name))
            .map(|variable| variable.value.clone())
            .unwrap_or(Value::Null);

        self.constant(&value)
    }
}

impl Evaluator<'_> {
    fn read_vars(&mut self, id: ObjectId) -> Result<GenericValue> {
        let cached = self.vars_lists.get(&id).copied();
        let object = self
            .runtime
            .heap
            .object(id)
            .ok_or_else(|| self.fault(FaultKind::InvalidReference))?;
        let ty = object.ty;

        let mut names = object.vars.keys().cloned().collect::<HashSet<_>>();
        names.extend([vars::TYPE, vars::PARENT_TYPE, vars::VARS, vars::TAG].map(Identifier::from));
        // instance vars only: no globals, which live on the root, and no static vars
        for declaration in self
            .tree
            .ancestors(ty)
            .filter(|declaration| declaration.id != TypeId::ROOT)
        {
            names.extend(
                declaration
                    .vars
                    .iter()
                    .filter(|(_, variable)| !(variable.modifiers.is_global || variable.modifiers.is_static))
                    .map(|(name, _)| name.clone()),
            );
        }

        let mut names = names.into_iter().collect::<Vec<_>>();
        names.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        if cached.is_none() {
            self.reserve(names.len())?;
        }

        let mut entries = Vec::with_capacity(names.len());
        for name in names {
            let value = match name.as_str() {
                vars::VARS => GenericValue::Null,
                _ => self.read_field(GenericValue::Object(id), &name)?,
            };
            entries.push((GenericValue::Text(name.as_str().into()), Some(value)));
        }

        // one list per object, refreshed on every read so writes made since show up and edits to it don't stick
        if let Some(list) = cached {
            self.list_mut(list)?.replace(entries);

            return Ok(GenericValue::List(list));
        }

        let list = self.list(entries)?;
        if let GenericValue::List(list_id) = list {
            self.list_mut(list_id)?.kind = crate::value::ListKind::Vars(id);
            self.vars_lists.insert(id, list_id);
        }

        Ok(list)
    }

    pub fn read_field(&mut self, object: GenericValue, name: &Identifier) -> Result<GenericValue> {
        let key = name.as_str();
        match object {
            GenericValue::World => Ok(match key {
                "time" | "timeofday" | "tick_usage" => 0.0.into(),
                "tick_lag" => 1.0.into(),
                "maxx" => (self.runtime.world.size[0] as f32).into(),
                "maxy" => (self.runtime.world.size[1] as f32).into(),
                "maxz" => (self.runtime.world.size[2] as f32).into(),
                vars::LOG => GenericValue::Null,
                vars::CONTENTS => {
                    self.is_cacheable = false;
                    let values = self
                        .runtime
                        .heap
                        .objects()
                        .filter(|(_, object)| object.instance.is_some())
                        .map(|(id, _)| (GenericValue::Object(id), None))
                        .collect::<Vec<_>>();
                    self.list(values)?
                },
                vars::TYPE => self
                    .world_type()
                    .and_then(|ty| self.tree.get(ty))
                    .map(|declaration| GenericValue::Path(Box::new(declaration.path.clone())))
                    .unwrap_or_default(),
                _ => {
                    let world = self
                        .runtime
                        .world_object
                        .ok_or_else(|| self.fault(FaultKind::MissingVariable(format!("world.{key}"))))?;

                    return self.read_field(GenericValue::Object(world), name);
                },
            }),
            GenericValue::Global => {
                let global = self.runtime.global.map(GenericValue::Object).unwrap_or_default();
                self.read_field(global, name)
            },
            GenericValue::List(id) | GenericValue::ArgList(id) if key == vars::LEN => {
                Ok((self.runtime.heap.list(id).map_or(0, |list| list.entries.len()) as f32).into())
            },
            GenericValue::Path(path) => {
                let ty = self
                    .tree
                    .id_of(&path)
                    .ok_or_else(|| self.fault(FaultKind::InvalidReference))?;

                match key {
                    vars::TYPE => {
                        return Ok(GenericValue::Path(path));
                    },
                    vars::PARENT_TYPE => {
                        return Ok(self.parent_path(ty));
                    },
                    _ => {},
                }

                if let Some(initializer) = self.module.index.inherited_initializer(self.tree, ty, name) {
                    self.reserve(1)?;
                    let dummy = self
                        .runtime
                        .heap
                        .alloc_object(Object::new(ty))
                        .map_err(|kind| self.fault(kind))?;
                    return self.call(initializer, Some(dummy), Vec::new());
                }

                let value = self
                    .tree
                    .var_inherited(ty, name)
                    .map(|variable| variable.value.clone())
                    .ok_or_else(|| self.fault(FaultKind::MissingVariable(key.into())))?;

                self.constant(&value)
            },
            GenericValue::Object(id) => {
                if matches!(key, vars::OVERLAYS | vars::UNDERLAYS) {
                    self.appearance_reads.insert(id);
                }

                if key == vars::VARS {
                    return self.read_vars(id);
                }

                if let Some(origin) = self.origin
                    && let Some(object) = self.runtime.heap.object(id)
                    && object.instance.is_some()
                {
                    let origin_position = self.runtime.world.position(&self.runtime.heap, origin);
                    let object_position = self.runtime.world.position(&self.runtime.heap, id);
                    if let (Some(origin), Some(object)) = (origin_position, object_position) {
                        if origin.z != object.z || origin.x.abs_diff(object.x) > 1 || origin.y.abs_diff(object.y) > 1 {
                            self.is_cacheable = false;
                        }
                    } else if !self
                        .tree
                        .roots()
                        .area
                        .is_some_and(|area| self.tree.is_subtype_of(object.ty, area))
                    {
                        self.is_cacheable = false;
                    }
                }

                let object = self
                    .runtime
                    .heap
                    .object(id)
                    .ok_or_else(|| self.fault(FaultKind::InvalidReference))?;

                match key {
                    vars::APPEARANCE => {
                        let vars = object.vars.clone();
                        let ty = object.ty;
                        self.reserve(vars.len() + 1)?;
                        let mut copy = Object::new(ty);
                        copy.vars = vars;
                        return self
                            .runtime
                            .heap
                            .alloc_object(copy)
                            .map(GenericValue::Object)
                            .map_err(|kind| self.fault(kind));
                    },
                    vars::TYPE => {
                        return Ok(self
                            .tree
                            .get(object.ty)
                            .map(|declaration| GenericValue::Path(Box::new(declaration.path.clone())))
                            .unwrap_or_default());
                    },
                    vars::PARENT_TYPE => return Ok(self.parent_path(object.ty)),
                    vars::LOC => return Ok(object.loc.map(GenericValue::Object).unwrap_or_default()),
                    vars::X | vars::Y | vars::Z if self.has_coordinates(object.ty) => {
                        self.position_sensitive = true;
                        let position = self.runtime.world.position(&self.runtime.heap, id);
                        return Ok(position
                            .map(|position| match key {
                                vars::X => position.x,
                                vars::Y => position.y,
                                _ => position.z,
                            })
                            .unwrap_or(0)
                            .into());
                    },
                    vars::CONTENTS => {
                        if self
                            .tree
                            .roots()
                            .area
                            .is_some_and(|area| self.tree.is_subtype_of(object.ty, area))
                        {
                            self.is_cacheable = false;
                        }
                        let entries = object
                            .contents
                            .iter()
                            .copied()
                            .map(|id| (GenericValue::Object(id), None))
                            .collect::<Vec<_>>();
                        return self.list(entries);
                    },
                    _ => {},
                }

                if let Some(value) = object.vars.get(name) {
                    return Ok(value.clone());
                }

                let object_type = object.ty;
                let declaration = self.tree.var_declaration(object_type, name);
                let shared = declaration.and_then(|(owner, variable)| shared_key(owner, variable, name));

                if let Some(shared) = &shared
                    && let Some(value) = self
                        .runtime
                        .global
                        .and_then(|global| self.runtime.heap.object(global))
                        .and_then(|global| global.vars.get(shared))
                {
                    return Ok(value.clone());
                }

                let initializer = declaration.and_then(|(owner, _)| self.module.index.initializer(owner.id, name));
                let declaration = declaration.map(|(_, variable)| variable);
                if let Some(initializer) = initializer {
                    let value = self.call(initializer, Some(id), Vec::new())?;
                    let (storage, key) = if let Some(shared) = shared {
                        (
                            self.runtime
                                .global
                                .ok_or_else(|| self.fault(FaultKind::InvalidReference))?,
                            shared,
                        )
                    } else {
                        (id, name.clone())
                    };
                    self.object_mut(storage)?.vars.insert(key, value.clone());
                    return Ok(value);
                }

                let value = declaration.map(|variable| variable.value.clone());
                let value = match value {
                    // `var/list/overlays = null` in the prelude, which BYOND never leaves null
                    None | Some(Value::Null)
                        if matches!(key, vars::OVERLAYS | vars::UNDERLAYS | vars::VIS_CONTENTS) =>
                    {
                        self.list(Vec::new())?
                    },
                    Some(Value::Unevaluated) => {
                        return Err(self.fault(FaultKind::Unsupported(format!(
                            "the initializer of {key} was not compiled"
                        ))));
                    },
                    Some(value) => self.constant(&value)?,
                    None if key == vars::TAG => GenericValue::Null,
                    None => return Err(self.fault(FaultKind::MissingVariable(key.into()))),
                };

                if matches!(value, GenericValue::List(_)) || shared.is_some() {
                    let (storage, key) = if let Some(shared) = shared {
                        (
                            self.runtime
                                .global
                                .ok_or_else(|| self.fault(FaultKind::InvalidReference))?,
                            shared,
                        )
                    } else {
                        (id, name.clone())
                    };
                    self.object_mut(storage)?.vars.insert(key, value.clone());
                }

                Ok(value)
            },
            GenericValue::Null => Err(self.fault(FaultKind::Null(format!("read null.{key}")))),
            _ => Err(self.fault(FaultKind::InvalidReference)),
        }
    }

    fn parent_path(&self, ty: TypeId) -> GenericValue {
        self.tree
            .get(ty)
            .and_then(|declaration| declaration.parent)
            .and_then(|parent| self.tree.get(parent))
            .map(|declaration| GenericValue::Path(Box::new(declaration.path.clone())))
            .unwrap_or_default()
    }

    fn world_type(&self) -> Option<TypeId> {
        self.tree.roots().world
    }

    fn shared_key(&self, ty: TypeId, name: &Identifier) -> Option<Identifier> {
        let (owner, variable) = self.tree.var_declaration(ty, name)?;

        shared_key(owner, variable, name)
    }

    /// `/datum/light` declares its own `x`, which only atoms and images derive from their location
    fn has_coordinates(&self, ty: TypeId) -> bool {
        let roots = self.tree.roots();

        [roots.atom, roots.image]
            .into_iter()
            .flatten()
            .any(|root| self.tree.is_subtype_of(ty, root))
    }

    pub fn write_field(&mut self, object: GenericValue, name: Identifier, value: GenericValue) -> Result<()> {
        let key = name.as_str();
        match object {
            GenericValue::Global => {
                let global = self.runtime.global.map(GenericValue::Object).unwrap_or_default();
                self.write_field(global, name, value)
            },
            GenericValue::World if key == vars::LOG => Ok(()),
            GenericValue::World => match self.runtime.world_object {
                Some(world) => self.write_field(GenericValue::Object(world), name, value),
                None => Ok(()),
            },
            GenericValue::List(id) | GenericValue::ArgList(id) if key == vars::LEN => {
                let length = self.number(&value)?;
                if length < 0.0 || !length.is_finite() {
                    return Err(self.fault(FaultKind::InvalidOperation("invalid list length".into())));
                }
                self.reserve(length as usize)?;
                let list = self.list_mut(id)?;
                list.entries.resize(length as usize, (GenericValue::Null, None));
                list.reindex();
                Ok(())
            },
            GenericValue::Object(id) if key == vars::APPEARANCE => {
                let source = value
                    .object()
                    .and_then(|source| self.runtime.heap.object(source))
                    .cloned()
                    .ok_or_else(|| self.fault(FaultKind::InvalidReference))?;
                let mut vars = source.vars;
                for declaration in self.tree.ancestors(source.ty) {
                    for (key, variable) in &declaration.vars {
                        if !vars.contains_key(key) && variable.value.is_constant() {
                            let value = self
                                .runtime
                                .constant(&variable.value, self.limits)
                                .map_err(|kind| self.fault(kind))?;
                            vars.insert(key.clone(), value);
                        }
                    }
                }
                self.object_mut(id)?.vars.extend(vars);
                Ok(())
            },
            // BYOND keeps these as the atom's own lists: null empties one and a list is copied in
            GenericValue::Object(id) if matches!(key, vars::OVERLAYS | vars::UNDERLAYS | vars::VIS_CONTENTS) => {
                let entries = match value {
                    GenericValue::Null => Vec::new(),
                    GenericValue::List(list) | GenericValue::ArgList(list) => self
                        .runtime
                        .heap
                        .list(list)
                        .ok_or_else(|| self.fault(FaultKind::InvalidReference))?
                        .entries
                        .clone(),
                    value => vec![(value, None)],
                };
                let list = self.list(entries)?;
                self.object_mut(id)?.vars.insert(name, list);
                Ok(())
            },
            GenericValue::Object(id) if key == vars::LOC => self
                .runtime
                .heap
                .relocate(id, value.object())
                .map_err(|kind| self.fault(kind)),
            GenericValue::Object(id)
                if !(matches!(key, vars::TYPE | vars::PARENT_TYPE | vars::CONTENTS)
                    || matches!(key, vars::X | vars::Y | vars::Z)
                        && self
                            .runtime
                            .heap
                            .object(id)
                            .is_none_or(|object| self.has_coordinates(object.ty))) =>
            {
                let shared = self
                    .runtime
                    .heap
                    .object(id)
                    .and_then(|object| self.shared_key(object.ty, &name));
                let (storage, key) = if let Some(shared) = shared {
                    (
                        self.runtime
                            .global
                            .ok_or_else(|| self.fault(FaultKind::InvalidReference))?,
                        shared,
                    )
                } else {
                    (id, name)
                };
                self.object_mut(storage)?.vars.insert(key, value);
                Ok(())
            },
            GenericValue::Null => Err(self.fault(FaultKind::Null(format!("modify null.{key}")))),
            _ => Err(self.fault(FaultKind::InvalidOperation(format!("cannot write {key}")))),
        }
    }

    pub fn set_index(&mut self, object: GenericValue, key: GenericValue, value: GenericValue) -> Result<()> {
        let id = match object {
            GenericValue::List(id) | GenericValue::ArgList(id) => id,
            _ => return Err(self.fault(FaultKind::InvalidReference)),
        };

        if let Some(owner) = self.vars_owner(id)
            && let GenericValue::Text(name) = &key
        {
            self.write_field(GenericValue::Object(owner), Identifier::from(&**name), value.clone())?;
            let list = self.list_mut(id)?;
            if let Some(index) = list.position(&key)
                && let Some(entry) = list.entries.get_mut(index)
            {
                entry.1 = Some(value);
            }

            return Ok(());
        }

        if let GenericValue::Num(number) = key {
            let index = (number as usize).wrapping_sub(1);
            let list = self.list_mut(id)?;
            let Some(entry) = list.entries.get_mut(index) else {
                return Err(self.fault(FaultKind::InvalidOperation("list index out of bounds".into())));
            };
            entry.0 = value;
            entry.1 = None;
            list.reindex();
        } else {
            self.reserve(1)?;
            let list = self.list_mut(id)?;
            if let Some(index) = list.position(&key) {
                if let Some(entry) = list.entries.get_mut(index) {
                    entry.1 = Some(value);
                }
            } else {
                list.push(key, Some(value));
            }
        }

        Ok(())
    }

    fn vars_owner(&self, list: ListId) -> Option<ObjectId> {
        match self.runtime.heap.list(list)?.kind {
            crate::value::ListKind::Vars(owner) => Some(owner),
            _ => None,
        }
    }

    pub fn get_index(&mut self, object: GenericValue, key: GenericValue) -> Result<GenericValue> {
        if let GenericValue::List(id) = object
            && let Some(owner) = self.vars_owner(id)
            && let GenericValue::Text(name) = &key
        {
            return self.read_field(GenericValue::Object(owner), &Identifier::from(&**name));
        }

        match object {
            GenericValue::Null => Ok(GenericValue::Null),
            GenericValue::List(id) | GenericValue::ArgList(id) => self
                .runtime
                .heap
                .list(id)
                .map(|list| list.get(&key))
                .ok_or_else(|| self.fault(FaultKind::InvalidReference)),
            GenericValue::Text(text) => {
                let index = self.number(&key)? as usize;
                Ok(text
                    .chars()
                    .nth(index.wrapping_sub(1))
                    .map(|character| character.to_string().into())
                    .unwrap_or_default())
            },
            _ => Err(self.fault(FaultKind::InvalidReference)),
        }
    }
}

impl Evaluator<'_> {
    fn unary(&mut self, op: Unary, value: GenericValue) -> Result<GenericValue> {
        if op == Unary::BitNot
            && let Some(matrix) = self.matrix(&value)
        {
            let inverted = matrix
                .invert()
                .ok_or_else(|| self.fault(FaultKind::InvalidOperation("singular matrix".into())))?;

            return self.store_matrix(None, inverted);
        }

        Ok(match op {
            Unary::Neg => (-self.number(&value)?).into(),
            Unary::Not => (!value.truthy()).into(),
            Unary::BitNot => ((!(self.number(&value)? as u32) & 0x00ff_ffff) as f32).into(),
            Unary::PreIncrement | Unary::PostIncrement => (self.number(&value)? + 1.0).into(),
            Unary::PreDecrement | Unary::PostDecrement => (self.number(&value)? - 1.0).into(),
            Unary::Reference | Unary::Dereference => {
                return Err(self.fault(FaultKind::Unsupported(format!("{op:?}"))));
            },
        })
    }

    pub fn binary(&mut self, op: Binary, left: GenericValue, right: GenericValue) -> Result<GenericValue> {
        use Binary::*;
        match op {
            CompEq | CompEquiv => return Ok((left == right).into()),
            CompNotEq | CompNotEquiv => return Ok((left != right).into()),
            LogicalAnd => return Ok(if left.truthy() { right } else { left }),
            LogicalOr => return Ok(if left.truthy() { left } else { right }),
            In => {
                let entries = self.iter_values(right)?;
                return Ok(entries.iter().any(|(value, _)| *value == left).into());
            },
            Add if matches!(left, GenericValue::Text(_)) && matches!(right, GenericValue::Text(_)) => {
                let left = left.display();
                let right = right.display();
                if left.len().saturating_add(right.len()) > self.limits.text_bytes {
                    return Err(self.fault(FaultKind::Memory));
                }
                return self.text(left + &right);
            },
            _ => {},
        }

        if let Some(value) = self.matrix_binary(op, &left, &right)? {
            return Ok(value);
        }

        if left == GenericValue::Null && matches!(right, GenericValue::List(_)) && matches!(op, Add | BitOr) {
            let GenericValue::List(id) = right else {
                return Err(self.fault(FaultKind::InvalidReference));
            };
            let entries = self
                .runtime
                .heap
                .list(id)
                .map(|list| list.entries.clone())
                .unwrap_or_default();
            return self.list(entries);
        }

        if left == GenericValue::Null && op == Add && right.num().is_none() {
            return Ok(right);
        }

        if matches!(left, GenericValue::Text(_)) && right == GenericValue::Null && op == Add {
            return Ok(left);
        }

        if let GenericValue::List(id) = left
            && matches!(op, Add | Sub | BitOr | BitAnd | BitXor)
        {
            let mut entries = self
                .runtime
                .heap
                .list(id)
                .map(|list| list.entries.clone())
                .ok_or_else(|| self.fault(FaultKind::InvalidReference))?;

            let right = self.list_operand(right);
            list_operation(op, &mut entries, right);

            return self.list(entries);
        }

        if let (Some(left), Some(right)) = (left.text(), right.text()) {
            return Ok(match op {
                CompLess => (left < right).into(),
                CompLessEq => (left <= right).into(),
                CompGreater => (left > right).into(),
                CompGreaterEq => (left >= right).into(),
                _ => return Err(self.fault(FaultKind::InvalidOperation("text operator".into()))),
            });
        }
        let left = self.number(&left)?;
        let right = self.number(&right)?;

        if matches!(op, Div | Mod | FloatMod) && right == 0.0 {
            return Err(self.fault(FaultKind::InvalidOperation("division by zero".into())));
        }

        Ok(match op {
            Add => (left + right).into(),
            Sub => (left - right).into(),
            Mul => (left * right).into(),
            Div => (left / right).into(),
            Mod => {
                if right.trunc() == 0.0 {
                    return Err(self.fault(FaultKind::InvalidOperation("division by zero".into())));
                }
                (left.trunc() % right.trunc()).into()
            },
            FloatMod => (left % right).into(),
            Pow => left.powf(right).into(),
            BitAnd => (((left as u32) & (right as u32)) as f32).into(),
            BitOr => (((left as u32) | (right as u32)) as f32).into(),
            BitXor => (((left as u32) ^ (right as u32)) as f32).into(),
            ShiftLeft => (((left as u32).checked_shl(right as u32).unwrap_or(0) & 0x00ff_ffff) as f32).into(),
            ShiftRight => (((left as u32).checked_shr(right as u32).unwrap_or(0)) as f32).into(),
            CompLess => (left < right).into(),
            CompLessEq => (left <= right).into(),
            CompGreater => (left > right).into(),
            CompGreaterEq => (left >= right).into(),
            CompThreeWay => (if left < right {
                -1.0
            } else if left > right {
                1.0
            } else {
                0.0
            })
            .into(),
            _ => return Err(self.fault(FaultKind::InvalidOperation(format!("{op:?}")))),
        })
    }

    /// `L += x` changes `L` itself, so every alias of it sees the change
    fn compound_binary(&mut self, op: Binary, left: GenericValue, right: GenericValue) -> Result<GenericValue> {
        let GenericValue::List(id) = left else {
            return self.binary(op, left, right);
        };

        // TODO: Same complaint about before, I don't like handling lists like this,
        // they should be either handled during semantic analysis or IR lowering

        if !matches!(
            op,
            Binary::Add | Binary::Sub | Binary::BitOr | Binary::BitAnd | Binary::BitXor
        ) {
            return self.binary(op, left, right);
        }

        let right = self.list_operand(right);
        if op == Binary::Add {
            self.reserve(right.len())?;
            let list = self.list_mut(id)?;
            for (key, value) in right {
                list.push(key, value);
            }

            return Ok(GenericValue::List(id));
        }

        let list = self.list_mut(id)?;
        let mut entries = std::mem::take(&mut list.entries);
        list_operation(op, &mut entries, right);
        list.replace(entries);

        Ok(GenericValue::List(id))
    }

    /// The entries `list OP value` combines with: a list's own, or the value alone
    fn list_operand(&self, value: GenericValue) -> Vec<(GenericValue, Option<GenericValue>)> {
        match value {
            GenericValue::List(id) | GenericValue::ArgList(id) => self
                .runtime
                .heap
                .list(id)
                .map(|list| list.entries.clone())
                .unwrap_or_default(),
            value => vec![(value, None)],
        }
    }

    pub fn iter_values(&mut self, value: GenericValue) -> Result<Vec<(GenericValue, Option<GenericValue>)>> {
        let entries = match value {
            GenericValue::Null => Vec::new(),
            GenericValue::List(id) | GenericValue::ArgList(id) => self
                .runtime
                .heap
                .list(id)
                .map(|list| list.entries.clone())
                .ok_or_else(|| self.fault(FaultKind::InvalidReference))?,
            GenericValue::Object(id) => self
                .runtime
                .heap
                .object(id)
                .map(|object| {
                    object
                        .contents
                        .iter()
                        .map(|id| (GenericValue::Object(*id), None))
                        .collect::<Vec<_>>()
                })
                .ok_or_else(|| self.fault(FaultKind::InvalidReference))?,
            GenericValue::World => {
                self.is_cacheable = false;
                self.runtime
                    .heap
                    .objects()
                    .filter(|(_, object)| object.instance.is_some())
                    .map(|(id, _)| (GenericValue::Object(id), None))
                    .collect::<Vec<_>>()
            },
            GenericValue::Range(range) => {
                if !range.start.is_finite() || !range.end.is_finite() || !range.step.is_finite() || range.step == 0.0 {
                    return Err(self.fault(FaultKind::InvalidOperation("invalid range".into())));
                }

                let mut current = range.start;
                let mut entries = Vec::new();

                while if range.step > 0.0 {
                    current <= range.end
                } else {
                    current >= range.end
                } {
                    self.reserve(1)?;
                    entries.push((GenericValue::Num(current), None));
                    let next = current + range.step;
                    if next == current || entries.len() > self.limits.allocations {
                        return Err(self.fault(FaultKind::Memory));
                    }

                    current = next;
                }

                entries
            },
            _ => return Err(self.fault(FaultKind::InvalidOperation("not iterable".into()))),
        };

        Ok(entries)
    }

    pub(crate) fn matches_type(&self, value: &GenericValue, path: &Option<TreePath>) -> bool {
        let Some(path) = path else {
            return true;
        };

        if matches!(path.to_string().as_str(), "/list" | "/alist") {
            let Some(id) = (match value {
                GenericValue::List(id) | GenericValue::ArgList(id) => Some(*id),
                _ => None,
            }) else {
                return false;
            };
            return path.to_string() == "/list"
                || self
                    .runtime
                    .heap
                    .list(id)
                    .is_some_and(|list| list.kind == crate::value::ListKind::Alist);
        }

        let Some(ancestor) = self.tree.id_of(path) else {
            return false;
        };

        value
            .object()
            .and_then(|id| self.runtime.heap.object(id))
            .is_some_and(|object| self.tree.is_subtype_of(object.ty, ancestor))
    }

    fn iterator(&mut self, value: GenericValue, ty: Option<TreePath>, value_is_associated: bool) -> Result<IteratorId> {
        let entries = self.iter_values(value)?;
        self.reserve(entries.len().max(1))?;
        let id = IteratorId(u32::try_from(self.iterators.len()).map_err(|_| self.fault(FaultKind::Memory))?);
        self.iterators.push(IteratorState {
            entries,
            index: 0,
            current: None,
            ty,
            value_is_associated,
        });

        Ok(id)
    }

    fn iterator_id(&self, value: GenericValue) -> Result<IteratorId> {
        match value {
            GenericValue::Iterator(id) => Ok(id),
            _ => Err(self.fault(FaultKind::InvalidReference)),
        }
    }

    fn iterator_next(&mut self, id: IteratorId) -> Result<bool> {
        loop {
            let (entry, ty, associated) = {
                let state = self
                    .iterators
                    .get(id.0 as usize)
                    .ok_or_else(|| self.fault(FaultKind::InvalidReference))?;
                let Some(entry) = state.entries.get(state.index).cloned() else {
                    if let Some(state) = self.iterators.get_mut(id.0 as usize) {
                        state.current = None;
                    }
                    return Ok(false);
                };
                (entry, state.ty.clone(), state.value_is_associated)
            };

            let value = if associated {
                entry.1.as_ref().unwrap_or(&GenericValue::Null)
            } else {
                &entry.0
            };

            let matches = self.matches_type(value, &ty);
            let Some(state) = self.iterators.get_mut(id.0 as usize) else {
                return Err(self.fault(FaultKind::InvalidReference));
            };

            state.index += 1;

            if matches {
                state.current = Some(entry);
                return Ok(true);
            }
        }
    }

    fn iterator_value(&self, id: IteratorId) -> Result<GenericValue> {
        let state = self
            .iterators
            .get(id.0 as usize)
            .ok_or_else(|| self.fault(FaultKind::InvalidReference))?;
        let (entry, association) = state
            .current
            .as_ref()
            .ok_or_else(|| self.fault(FaultKind::InvalidOperation("iterator has no current value".into())))?;

        Ok(if state.value_is_associated {
            association.clone().unwrap_or_default()
        } else {
            entry.clone()
        })
    }

    fn iterator_key(&self, id: IteratorId) -> Result<GenericValue> {
        self.iterators
            .get(id.0 as usize)
            .and_then(|state| state.current.as_ref())
            .map(|(entry, _)| entry.clone())
            .ok_or_else(|| self.fault(FaultKind::InvalidOperation("iterator has no current key".into())))
    }

    fn make_list(&mut self, args: Vec<(Option<GenericValue>, GenericValue)>) -> Result<GenericValue> {
        let mut entries = Vec::new();
        for (key, value) in args {
            if let Some(key) = key {
                if let Some((_, association)) = entries.iter_mut().find(|(entry, _)| *entry == key) {
                    *association = Some(value);
                } else {
                    entries.push((key, Some(value)));
                }
            } else {
                entries.push((value, None));
            }
        }

        self.list(entries)
    }

    fn pick(&mut self, weighted: &[bool], operands: Vec<GenericValue>) -> Result<GenericValue> {
        let mut operands = operands.into_iter();
        let mut choices = Vec::with_capacity(weighted.len());
        let mut total = 0.0;
        for weighted in weighted {
            let weight = if *weighted {
                let weight = operands.next().ok_or_else(|| self.fault(FaultKind::InvalidReference))?;
                self.number(&weight)?.max(0.0)
            } else {
                100.0
            };
            let value = operands.next().ok_or_else(|| self.fault(FaultKind::InvalidReference))?;
            total += weight;
            choices.push((total, value));
        }

        if choices.len() == 1
            && !weighted[0]
            && let GenericValue::List(id) = &choices[0].1
        {
            let length = self.runtime.heap.list(*id).map_or(0, |list| list.entries.len());
            let index = (self.random() * length as f32) as usize;

            return Ok(self
                .runtime
                .heap
                .list(*id)
                .and_then(|list| list.entries.get(index))
                .map(|(value, _)| value.clone())
                .unwrap_or_default());
        }

        let choice = self.random() * total;

        Ok(choices
            .into_iter()
            .find(|(limit, _)| choice < *limit)
            .map(|(_, value)| value)
            .unwrap_or_default())
    }

    fn in_range(
        &self, value: GenericValue, start: GenericValue, end: GenericValue, step: GenericValue,
    ) -> Result<GenericValue> {
        let value = self.number(&value)?;
        let start = self.number(&start)?;
        let end = self.number(&end)?;
        let step = self.number(&step)?;

        if step == 0.0 || !step.is_finite() {
            return Ok(false.into());
        }

        let within = if step > 0.0 {
            value >= start && value <= end
        } else {
            value <= start && value >= end
        };

        Ok((within && (value - start) % step == 0.0).into())
    }

    fn delete(&mut self, value: GenericValue) -> Result<()> {
        if let Some(id) = value.object() {
            self.runtime.heap.relocate(id, None).map_err(|kind| self.fault(kind))?;
            self.object_mut(id)?.deleted = true;
        }

        Ok(())
    }
}

impl Evaluator<'_> {
    // what an uncaught throw reports: an /exception's name, or the value itself
    pub(crate) fn throw(&mut self, value: GenericValue) -> Fault {
        let name = match &value {
            GenericValue::Object(_) => self
                .read_field(value.clone(), &Identifier::from("name"))
                .ok()
                .and_then(|name| name.text().map(ToOwned::to_owned)),
            _ => None,
        };
        let description = name.unwrap_or_else(|| value.display());
        self.thrown = Some(value);

        self.fault(FaultKind::Thrown(description))
    }

    pub(crate) fn throw_exception(&mut self, message: String) -> Fault {
        let exception = self
            .new_object(GenericValue::Path(Box::new(TreePath::parse("/exception"))), Vec::new())
            .and_then(|exception| {
                let name = self.text(message.clone())?;
                self.write_field(exception.clone(), Identifier::from("name"), name)?;

                Ok(exception)
            });

        match exception {
            Ok(exception) => self.throw(exception),
            Err(fault) => fault,
        }
    }

    pub fn new_object(
        &mut self, ty: GenericValue, args: Vec<(Option<Identifier>, GenericValue)>,
    ) -> Result<GenericValue> {
        let (path, overrides) = match ty {
            GenericValue::Path(path) => (*path, Vec::new()),
            GenericValue::ModifiedType(modified) => {
                let ModifiedType { path, overrides } = *modified;

                (path, overrides)
            },
            _ => return Err(self.fault(FaultKind::InvalidOperation("new requires type".into()))),
        };

        if path.to_string() == "/list" {
            let size = args
                .first()
                .map(|(_, value)| self.number(value))
                .transpose()?
                .unwrap_or(0.0);
            if !size.is_finite() || size < 0.0 {
                return Err(self.fault(FaultKind::InvalidOperation("invalid list length".into())));
            }
            let size = size as usize;
            self.reserve(size)?;
            let value = self.list(vec![(GenericValue::Null, None); size])?;
            if let GenericValue::List(id) = &value
                && let Some(ty) = self.tree.id_of(&path)
                && let Some(proc) = self.find_proc(ty, &"New".into())
            {
                self.call_function_for_proc(proc, Receiver::List(*id), None, args)?;
            }
            return Ok(value);
        }

        if path.to_string() == "/alist" {
            let entries = match args.first().map(|(_, value)| value.clone()) {
                Some(value @ (GenericValue::List(_) | GenericValue::ArgList(_))) => self.iter_values(value)?,
                _ => Vec::new(),
            };
            self.reserve(entries.len())?;
            let value = self.alist(entries)?;
            if let GenericValue::List(id) = &value
                && let Some(ty) = self.tree.id_of(&path)
                && let Some(proc) = self.find_proc(ty, &"New".into())
            {
                self.call_function_for_proc(proc, Receiver::List(*id), None, args)?;
            }
            return Ok(value);
        }

        let ty = self
            .tree
            .id_of(&path)
            .ok_or_else(|| self.fault(FaultKind::InvalidOperation(format!("unknown type {path}"))))?;
        self.reserve(1)?;

        let id = self
            .runtime
            .heap
            .alloc_object(Object::new(ty))
            .map_err(|kind| self.fault(kind))?;

        for (name, value) in overrides {
            self.write_field(GenericValue::Object(id), name, value)?;
        }

        // only a movable's first argument is where it is made, a datum just receives it
        let is_movable = self
            .tree
            .roots()
            .movable
            .is_some_and(|movable| self.tree.is_subtype_of(ty, movable));
        if is_movable && let Some(loc) = args.first().and_then(|(_, value)| value.object()) {
            self.runtime
                .heap
                .relocate(id, Some(loc))
                .map_err(|kind| self.fault(kind))?;
        }

        if let Some(proc) = self.find_proc(ty, &"New".into()) {
            self.call_function_for_proc(proc, Receiver::Object(id), None, args)?;
        }

        Ok(GenericValue::Object(id))
    }
}

/// `var/static/x` and `var/global/x` share one slot on the global object, keyed by the type that
/// declares them
fn shared_key(owner: &objtree::TypeDecl, variable: &objtree::VarDecl, name: &Identifier) -> Option<Identifier> {
    (owner.id != TypeId::ROOT && (variable.modifiers.is_static || variable.modifiers.is_global))
        .then(|| format!("__dmed_static_{}_{}", owner.id.0, name).into())
}
