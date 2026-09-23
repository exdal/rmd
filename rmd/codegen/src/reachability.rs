use core::{
    path::TreePath,
    types::{Identifier, IrNodeId, ProcId, Value},
};
use std::collections::{HashMap, HashSet, VecDeque};

use ir::{Argument, Builtin, IrNode};
use objtree::{ObjectTree, TypeId};
use prelude::Intrinsic;

use crate::{CodegenError, ProcedureReachability, reachable_blocks};

pub(super) fn reachable_procedures(
    module: &ir::Module, tree: &ObjectTree, roots: &[ProcId],
) -> Result<ProcedureReachability, CodegenError> {
    Reachability::new(module, tree).collect(roots)
}

struct Reachability<'a> {
    module: &'a ir::Module,
    tree: &'a ObjectTree,
    by_name: HashMap<Identifier, Vec<ProcId>>,
    reachable: HashSet<ProcId>,
    pending: VecDeque<ProcId>,
    address_taken: HashSet<ProcId>,
    opaque_callable: bool,
    synthesizes_proc_paths: bool,
    retain_all: bool,
}

#[derive(Default)]
struct DynamicCallTarget {
    paths: Vec<TreePath>,
    accepts_name: bool,
    unknown: bool,
}

impl DynamicCallTarget {
    fn merge(&mut self, other: Self) {
        self.paths.extend(other.paths);
        self.accepts_name |= other.accepts_name;
        self.unknown |= other.unknown;
    }
}

impl<'a> Reachability<'a> {
    fn new(module: &'a ir::Module, tree: &'a ObjectTree) -> Self {
        let mut by_name = HashMap::<Identifier, Vec<ProcId>>::new();
        for declaration in tree.iter() {
            for procedure in declaration.procs.values() {
                if let Some(body) = procedure.body {
                    by_name.entry(procedure.name.clone()).or_default().push(body);
                }
            }
        }

        Self {
            module,
            tree,
            by_name,
            reachable: HashSet::new(),
            pending: VecDeque::new(),
            address_taken: HashSet::new(),
            opaque_callable: false,
            synthesizes_proc_paths: false,
            retain_all: false,
        }
    }

    fn collect(mut self, roots: &[ProcId]) -> Result<ProcedureReachability, CodegenError> {
        for root in roots {
            self.retain(*root);
        }

        while let Some(proc_id) = self.pending.pop_front() {
            self.visit_procedure(proc_id)?;

            if self.retain_all {
                return Ok(ProcedureReachability::All);
            }
        }

        if self.opaque_callable && (self.synthesizes_proc_paths || self.address_taken.is_empty()) {
            return Ok(ProcedureReachability::All);
        }

        Ok(ProcedureReachability::Selected(self.reachable))
    }

    fn visit_procedure(&mut self, proc_id: ProcId) -> Result<(), CodegenError> {
        let Some(proc) = self.module.proc(proc_id) else {
            return Ok(());
        };

        let body = proc.body;
        self.collect_proc_references(proc_id)?;

        for block in reachable_blocks(self.module, body)? {
            let instructions = self
                .module
                .block(block)
                .ok_or(CodegenError::ExpectedBlock(block))?
                .to_vec();

            for instruction in instructions {
                let node = self
                    .module
                    .node(instruction)
                    .cloned()
                    .ok_or(CodegenError::MissingNode(instruction))?;
                self.visit_instruction(proc_id, &node);

                if self.retain_all {
                    return Ok(());
                }
            }
        }

        Ok(())
    }

    fn visit_instruction(&mut self, current: ProcId, node: &IrNode) {
        match node {
            IrNode::FunctionCall { function, args } => {
                let target = self.module.node(*function).cloned();
                match target {
                    Some(IrNode::Function(proc)) => {
                        self.retain(proc);
                        if let Some(intrinsic) = self.module.proc(proc).and_then(|procedure| procedure.intrinsic) {
                            if matches!(intrinsic, Intrinsic::Call | Intrinsic::CallExt)
                                && !self.retain_dynamic_call(args)
                            {
                                if args.len() == 1 {
                                    self.retain_opaque_callable();
                                } else {
                                    self.retain_all = true;
                                }
                            }
                            if intrinsic == Intrinsic::Text2path {
                                self.synthesizes_proc_paths = true;
                                if self.opaque_callable {
                                    self.retain_all = true;
                                }
                            }
                        }
                    },
                    Some(IrNode::ExternalFunction(name)) if name.as_str() != "initial" => self.retain_name(&name),
                    _ => {},
                }
            },
            IrNode::Call { callee, .. } => {
                let mut seen = HashSet::new();
                if !self.retain_callable(*callee, current, &mut seen) {
                    self.retain_opaque_callable();
                }
            },
            IrNode::Load { pointer } => {
                if let Some(IrNode::Variable(name)) = self.module.node(*pointer) {
                    self.retain_variable_initializer(current, name);
                }
            },
            IrNode::AccessField { object, name, .. }
            | IrNode::Initial {
                object: Some(object),
                name,
            } => {
                self.retain_field_initializers(*object, current, name);
            },
            IrNode::Initial { object: None, name } => self.retain_variable_initializer(current, name),
            IrNode::Super { .. } => self.retain_super(current),
            IrNode::New { ty: Some(ty), .. } => {
                let mut seen = HashSet::new();
                match self.type_paths(*ty, &mut seen) {
                    Some(paths) => {
                        for path in paths {
                            self.retain_constructor(&path);
                        }
                    },
                    None => self.retain_name(&Identifier::from("New")),
                }
            },
            _ => {},
        }
    }

    fn collect_proc_references(&mut self, proc_id: ProcId) -> Result<(), CodegenError> {
        let Some(proc) = self.module.proc(proc_id) else {
            return Ok(());
        };
        let mut pending = proc
            .params
            .iter()
            .flat_map(|parameter| {
                parameter
                    .default
                    .into_iter()
                    .chain(parameter.in_list)
                    .chain(parameter.spec.dimensions.iter().flatten().copied())
            })
            .chain(
                proc.vars
                    .iter()
                    .flat_map(|variable| variable.dimensions.iter().flatten().copied()),
            )
            .collect::<Vec<_>>();
        for block in reachable_blocks(self.module, proc.body)? {
            pending.extend(
                self.module
                    .block(block)
                    .ok_or(CodegenError::ExpectedBlock(block))?
                    .iter()
                    .copied(),
            );
        }

        let mut seen = HashSet::new();
        while let Some(id) = pending.pop() {
            if !seen.insert(id) {
                continue;
            }
            let node = self.module.node(id).cloned().ok_or(CodegenError::MissingNode(id))?;
            if let IrNode::Constant(value) = &node {
                self.note_value(value);
            }
            node.for_each_operand(|operand| pending.push(operand));
        }

        Ok(())
    }

    fn retain_opaque_callable(&mut self) {
        self.opaque_callable = true;
        for proc in self.address_taken.clone() {
            self.retain(proc);
        }
        if self.synthesizes_proc_paths {
            self.retain_all = true;
        }
    }

    fn note_value(&mut self, value: &Value) {
        match value {
            Value::Path(path) => {
                if let Some(proc) = self.proc_for_path(path)
                    && self.address_taken.insert(proc)
                    && self.opaque_callable
                {
                    self.retain(proc);
                }
            },
            Value::List(entries) => {
                for entry in entries {
                    self.note_value(&entry.key);
                    if let Some(value) = &entry.value {
                        self.note_value(value);
                    }
                }
            },
            _ => {},
        }
    }

    fn retain_variable_initializer(&mut self, current: ProcId, name: &Identifier) {
        let owner = self
            .module
            .proc(current)
            .and_then(|procedure| self.tree.id_of(&procedure.owner));
        let declaration = owner
            .and_then(|owner| self.tree.var_declaration(owner, name))
            .or_else(|| self.tree.var_declaration(TypeId::ROOT, name));
        if let Some((_, variable)) = declaration {
            self.retain(variable.initializer.unwrap_or(ProcId::INVALID));
            self.note_value(&variable.value);
        }
    }

    fn retain_field_initializers(&mut self, object: IrNodeId, current: ProcId, name: &Identifier) {
        let mut seen = HashSet::new();
        let Some(types) = self.receiver_types(object, current, &mut seen) else {
            if name.as_str() == "vars" {
                self.retain_every_initializer();
            } else {
                self.retain_initializers_named(name);
            }
            return;
        };

        for (ty, includes_descendants) in types {
            let candidates = match includes_descendants {
                true => self.tree.descendants(ty),
                false => vec![ty],
            };
            for candidate in candidates {
                if name.as_str() == "vars" {
                    self.retain_all_initializers_for_type(candidate);
                } else if let Some((_, variable)) = self.tree.var_declaration(candidate, name) {
                    self.retain(variable.initializer.unwrap_or(ProcId::INVALID));
                    self.note_value(&variable.value);
                }
            }
        }
    }

    fn retain_initializers_named(&mut self, name: &Identifier) {
        let variables = self
            .tree
            .iter()
            .filter_map(|declaration| declaration.vars.get(name))
            .map(|variable| (variable.initializer, variable.value.clone()))
            .collect::<Vec<_>>();
        for (initializer, value) in variables {
            self.retain(initializer.unwrap_or(ProcId::INVALID));
            self.note_value(&value);
        }
    }

    fn retain_all_initializers_for_type(&mut self, ty: TypeId) {
        let variables = self
            .tree
            .ancestors(ty)
            .flat_map(|declaration| declaration.vars.values())
            .map(|variable| (variable.initializer, variable.value.clone()))
            .collect::<Vec<_>>();
        for (initializer, value) in variables {
            self.retain(initializer.unwrap_or(ProcId::INVALID));
            self.note_value(&value);
        }
    }

    fn retain_every_initializer(&mut self) {
        let variables = self
            .tree
            .iter()
            .flat_map(|declaration| declaration.vars.values())
            .map(|variable| (variable.initializer, variable.value.clone()))
            .collect::<Vec<_>>();
        for (initializer, value) in variables {
            self.retain(initializer.unwrap_or(ProcId::INVALID));
            self.note_value(&value);
        }
    }

    /// returns true when every value reaching `node` has a bounded call target or cannot call
    fn retain_callable(&mut self, node: IrNodeId, current: ProcId, seen: &mut HashSet<IrNodeId>) -> bool {
        if !seen.insert(node) {
            return true;
        }

        match self.module.node(node).cloned() {
            Some(IrNode::Function(proc)) => {
                self.retain(proc);
                true
            },
            Some(IrNode::ExternalFunction(name)) => {
                if name.as_str() != "initial" {
                    self.retain_name(&name);
                }
                true
            },
            Some(IrNode::AccessField { object, name, .. }) => {
                let mut seen = HashSet::new();
                match self.receiver_types(object, current, &mut seen) {
                    Some(types) => {
                        for (ty, includes_descendants) in types {
                            if includes_descendants {
                                for candidate in self.tree.descendants(ty) {
                                    self.retain_inherited(candidate, &name);
                                }
                            } else {
                                self.retain_inherited(ty, &name);
                            }
                        }
                    },
                    None => self.retain_name(&name),
                }
                true
            },
            Some(IrNode::Constant(Value::Path(path))) => {
                self.retain_proc_path(&path);
                true
            },
            Some(IrNode::Constant(_)) | Some(IrNode::ModifiedType { .. }) => true,
            Some(IrNode::Builtin(Builtin::Callee)) => {
                self.retain(current);
                true
            },
            Some(IrNode::Phi { operands }) => operands
                .into_iter()
                .all(|operand| self.retain_callable(operand.value, current, seen)),
            Some(IrNode::Pick(choices)) => choices
                .into_iter()
                .all(|(_, value)| self.retain_callable(value, current, seen)),
            Some(IrNode::FunctionCall { function, args }) => {
                let Some(IrNode::Function(proc)) = self.module.node(function) else {
                    return false;
                };
                self.module
                    .proc(*proc)
                    .and_then(|procedure| procedure.intrinsic)
                    .filter(|intrinsic| matches!(intrinsic, Intrinsic::Call | Intrinsic::CallExt))
                    .is_some_and(|_| self.retain_dynamic_call(&args))
            },
            _ => false,
        }
    }

    /// resolve the procedure reference produced by the `call()` and `call_ext()` intrinsics
    fn retain_dynamic_call(&mut self, args: &[Argument]) -> bool {
        let Some(first) = args.first().and_then(|argument| argument.value) else {
            return true;
        };

        if args.len() == 1 {
            let mut seen = HashSet::new();
            let Some(values) = self.constant_values(first, &mut seen) else {
                return false;
            };

            for value in values {
                match value {
                    Value::Path(path) => self.retain_proc_path(&path),
                    Value::Text(name) => self.retain_inherited(TypeId::ROOT, &Identifier::from(name)),
                    _ => {},
                }
            }

            return true;
        }

        let mut seen = HashSet::new();
        let target = self.dynamic_call_target(first, &mut seen);

        if target.unknown {
            return false;
        }

        for path in target.paths {
            self.retain_proc_path(&path);
        }

        if !target.accepts_name {
            return true;
        }

        let Some(second) = args.get(1).and_then(|argument| argument.value) else {
            return true;
        };
        let mut seen = HashSet::new();
        let Some(names) = self.constant_values(second, &mut seen) else {
            return false;
        };

        for name in names {
            if let Value::Text(name) = name {
                self.retain_name(&Identifier::from(name));
            }
        }

        true
    }

    fn dynamic_call_target(&self, node: IrNodeId, seen: &mut HashSet<IrNodeId>) -> DynamicCallTarget {
        if !seen.insert(node) {
            return DynamicCallTarget::default();
        }

        match self.module.node(node) {
            Some(IrNode::Constant(Value::Path(path))) => DynamicCallTarget {
                paths: vec![path.clone()],
                ..DynamicCallTarget::default()
            },
            Some(IrNode::Constant(Value::List(_)))
            | Some(IrNode::Builtin(Builtin::Src | Builtin::Args))
            | Some(IrNode::New { .. })
            | Some(IrNode::List(_)) => DynamicCallTarget {
                accepts_name: true,
                ..DynamicCallTarget::default()
            },
            Some(IrNode::Constant(_)) | Some(IrNode::ModifiedType { .. }) => DynamicCallTarget::default(),
            Some(IrNode::Phi { operands }) => {
                let mut target = DynamicCallTarget::default();
                for operand in operands {
                    target.merge(self.dynamic_call_target(operand.value, seen));
                }
                target
            },
            Some(IrNode::Pick(choices)) => {
                let mut target = DynamicCallTarget::default();
                for (_, value) in choices {
                    target.merge(self.dynamic_call_target(*value, seen));
                }
                target
            },
            _ => DynamicCallTarget {
                unknown: true,
                ..DynamicCallTarget::default()
            },
        }
    }

    /// resolve receiver types for a field call, `bool` is true when any subtype can reach the
    /// value, as with a typed parameter or iterator, and false for an exact `new` expression
    fn receiver_types(
        &self, node: IrNodeId, current: ProcId, seen: &mut HashSet<IrNodeId>,
    ) -> Option<Vec<(TypeId, bool)>> {
        if !seen.insert(node) {
            return Some(Vec::new());
        }

        match self.module.node(node)? {
            IrNode::Builtin(Builtin::Src) => {
                let owner = &self.module.proc(current)?.owner;
                Some(vec![(self.tree.id_of(owner)?, true)])
            },
            IrNode::Builtin(Builtin::Global) => Some(vec![(TypeId::ROOT, false)]),
            IrNode::Builtin(Builtin::World) => Some(vec![(self.tree.id_of(&TreePath::parse("/world"))?, false)]),
            IrNode::Builtin(Builtin::Args) | IrNode::List(_) | IrNode::Constant(Value::List(_)) => {
                Some(vec![(self.tree.roots().list?, false)])
            },
            IrNode::Constant(Value::Path(path)) => Some(vec![(self.tree.id_of(path)?, false)]),
            IrNode::Load { pointer } => {
                let IrNode::Variable(name) = self.module.node(*pointer)? else {
                    return None;
                };
                let owner = self
                    .module
                    .proc(current)
                    .and_then(|procedure| self.tree.id_of(&procedure.owner));
                let variable = owner
                    .and_then(|owner| self.tree.var_declaration(owner, name))
                    .or_else(|| self.tree.var_declaration(TypeId::ROOT, name))?
                    .1;
                Some(vec![(self.tree.id_of(variable.declared_type.as_ref()?)?, true)])
            },
            IrNode::FunctionParameter(index) => {
                let procedure = self.module.proc(current)?;
                let parameter = procedure.params.get(*index as usize)?;
                let ty = parameter
                    .spec
                    .var_type
                    .as_ref()
                    .and_then(|path| self.tree.id_of(path))?;
                Some(vec![(ty, true)])
            },
            IrNode::IterValue(iterator) => {
                let IrNode::IterInit { ty: Some(path), .. } = self.module.node(*iterator)? else {
                    return None;
                };
                Some(vec![(self.tree.id_of(path)?, true)])
            },
            IrNode::New { ty: Some(ty), .. } => {
                let mut paths_seen = HashSet::new();
                let paths = self.type_paths(*ty, &mut paths_seen)?;
                Some(
                    paths
                        .into_iter()
                        .filter_map(|path| self.tree.id_of(&path).map(|ty| (ty, false)))
                        .collect::<Vec<_>>(),
                )
            },
            IrNode::Phi { operands } => {
                let mut types = Vec::new();
                for operand in operands {
                    types.extend(self.receiver_types(operand.value, current, seen)?);
                }
                Some(types)
            },
            IrNode::Pick(choices) => {
                let mut types = Vec::new();
                for (_, value) in choices {
                    types.extend(self.receiver_types(*value, current, seen)?);
                }
                Some(types)
            },
            _ => None,
        }
    }

    fn type_paths(&self, node: IrNodeId, seen: &mut HashSet<IrNodeId>) -> Option<Vec<TreePath>> {
        if !seen.insert(node) {
            return Some(Vec::new());
        }

        match self.module.node(node)? {
            IrNode::Constant(Value::Path(path)) | IrNode::ModifiedType { path, .. } => Some(vec![path.clone()]),
            IrNode::Constant(_) => Some(Vec::new()),
            IrNode::Phi { operands } => {
                let mut paths = Vec::new();
                for operand in operands {
                    paths.extend(self.type_paths(operand.value, seen)?);
                }
                Some(paths)
            },
            IrNode::Pick(choices) => {
                let mut paths = Vec::new();
                for (_, value) in choices {
                    paths.extend(self.type_paths(*value, seen)?);
                }
                Some(paths)
            },
            _ => None,
        }
    }

    fn constant_values(&self, node: IrNodeId, seen: &mut HashSet<IrNodeId>) -> Option<Vec<Value>> {
        if !seen.insert(node) {
            return Some(Vec::new());
        }

        match self.module.node(node)? {
            IrNode::Constant(value) => Some(vec![value.clone()]),
            IrNode::IterValue(iterator) => self.iterated_values(*iterator, seen),
            IrNode::Phi { operands } => {
                let mut values = Vec::new();
                for operand in operands {
                    values.extend(self.constant_values(operand.value, seen)?);
                }
                Some(values)
            },
            IrNode::Pick(choices) => {
                let mut values = Vec::new();
                for (_, value) in choices {
                    values.extend(self.constant_values(*value, seen)?);
                }
                Some(values)
            },
            _ => None,
        }
    }

    fn iterated_values(&self, iterator: IrNodeId, seen: &mut HashSet<IrNodeId>) -> Option<Vec<Value>> {
        if !seen.insert(iterator) {
            return Some(Vec::new());
        }

        let IrNode::IterInit { list, .. } = self.module.node(iterator)? else {
            return None;
        };
        let IrNode::FunctionCall { function, args } = self.module.node(*list)? else {
            return None;
        };
        let IrNode::Function(proc) = self.module.node(*function)? else {
            return None;
        };
        if self.module.proc(*proc)?.intrinsic != Some(Intrinsic::Typesof) {
            return None;
        }

        let mut values = Vec::new();
        for argument in args {
            let Some(argument) = argument.value else {
                continue;
            };

            for value in self.constant_values(argument, seen)? {
                let Value::Path(path) = value else {
                    continue;
                };
                let Some(ty) = self.tree.id_of(&path) else {
                    continue;
                };
                values.extend(self.tree.descendants(ty).into_iter().filter_map(|id| {
                    self.tree
                        .get(id)
                        .map(|declaration| Value::Path(declaration.path.clone()))
                }));
            }
        }

        Some(values)
    }

    fn retain(&mut self, proc: ProcId) {
        if self.module.proc(proc).is_some() && self.reachable.insert(proc) {
            self.pending.push_back(proc);
        }
    }

    fn retain_name(&mut self, name: &Identifier) {
        let candidates = self.by_name.get(name).cloned().unwrap_or_default();
        for candidate in candidates {
            self.retain(candidate);
        }
    }

    fn retain_inherited(&mut self, ty: TypeId, name: &Identifier) {
        if let Some(proc) = self.tree.proc_inherited(ty, name).and_then(|procedure| procedure.body) {
            self.retain(proc);
        }
    }

    fn retain_constructor(&mut self, path: &TreePath) {
        if let Some(ty) = self.tree.id_of(path) {
            self.retain_inherited(ty, &Identifier::from("New"));
        }
    }

    fn retain_proc_path(&mut self, path: &TreePath) {
        if let Some(proc) = self.proc_for_path(path) {
            self.retain(proc);
        }
    }

    fn proc_for_path(&self, path: &TreePath) -> Option<ProcId> {
        let name = path.name()?;
        let owner = TreePath::new(path.declaration_owner().to_vec(), true);
        self.tree
            .id_of(&owner)
            .and_then(|ty| self.tree.proc_inherited(ty, name))
            .and_then(|procedure| procedure.body)
    }

    fn retain_super(&mut self, current: ProcId) {
        let Some(procedure) = self.module.proc(current) else {
            return;
        };
        if let Some(previous) = procedure.previous {
            self.retain(previous);
            return;
        }

        let parent = self
            .tree
            .id_of(&procedure.owner)
            .and_then(|owner| self.tree.get(owner))
            .and_then(|declaration| declaration.parent);
        if let Some(parent) = parent {
            let name = procedure.name.clone();
            self.retain_inherited(parent, &name);
        }
    }
}
