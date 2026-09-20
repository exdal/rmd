use core::{
    location::Location,
    path::TreePath,
    types::{Identifier, ListEntry, Value, VarModifiers, VarSpec},
};
use std::collections::{HashMap, HashSet};

use ast::{AST, Expression, ExpressionId, ForLoop, Literal, Statement};

use crate::*;

#[derive(Debug, Clone)]
pub struct UnresolvedNew {
    pub owner: TreePath,
    pub name: Identifier,
    pub node: IrNodeId,
}

pub struct IrModuleBuilder<'a> {
    ast: &'a AST,
    pub module: Module,
    unresolved_new: Vec<UnresolvedNew>,
    constants: HashMap<ConstantKey, IrNodeId>,
    external_functions: HashMap<Identifier, IrNodeId>,
    named_variables: HashMap<Identifier, IrNodeId>,
    scopes: Vec<HashMap<Identifier, BindingId>>,
    vars: Vec<VarSpec<IrNodeId>>,
    parameters: Vec<BindingId>,
    current_def: HashMap<(BindingId, IrNodeId), IrNodeId>,
    stored_bindings: HashMap<BindingId, IrNodeId>,
    incomplete_phis: HashMap<IrNodeId, HashMap<BindingId, IrNodeId>>,
    phi_block: HashMap<IrNodeId, IrNodeId>,
    preds: HashMap<IrNodeId, Vec<IrNodeId>>,
    sealed: HashSet<IrNodeId>,
    blocks: Vec<IrNodeId>,
    named_blocks: HashMap<Identifier, IrNodeId>,
    defined_named_blocks: HashSet<IrNodeId>,
    controls: Vec<ControlTarget>,
    current_block: Option<IrNodeId>,
    entry: IrNodeId,
    current_owner: TreePath,
    locals_in_memory: bool,
    depth: usize,
}

#[derive(Clone, PartialEq, Eq, Hash)]
enum ConstantKey {
    Null,
    Num(u32),
    Text(String),
    Resource(String),
    Path(TreePath),
    List(Vec<(ConstantKey, Option<ConstantKey>)>),
    Unevaluated,
}

impl From<&Value> for ConstantKey {
    fn from(value: &Value) -> Self {
        match value {
            Value::Null => Self::Null,
            Value::Num(value) => Self::Num(value.to_bits()),
            Value::Text(value) => Self::Text(value.clone()),
            Value::Resource(value) => Self::Resource(value.clone()),
            Value::Path(value) => Self::Path(value.clone()),
            Value::List(entries) => Self::List(entries.iter().map(constant_list_entry).collect()),
            Value::Unevaluated => Self::Unevaluated,
        }
    }
}

fn constant_list_entry(entry: &ListEntry) -> (ConstantKey, Option<ConstantKey>) {
    (
        ConstantKey::from(&entry.key),
        entry.value.as_ref().map(ConstantKey::from),
    )
}

impl<'a> IrModuleBuilder<'a> {
    pub fn new(ast: &'a AST) -> Self {
        Self {
            ast,
            module: Module::default(),
            unresolved_new: Vec::new(),
            constants: HashMap::new(),
            external_functions: HashMap::new(),
            named_variables: HashMap::new(),
            scopes: Vec::new(),
            vars: Vec::new(),
            parameters: Vec::new(),
            current_def: HashMap::new(),
            stored_bindings: HashMap::new(),
            incomplete_phis: HashMap::new(),
            phi_block: HashMap::new(),
            preds: HashMap::new(),
            sealed: HashSet::new(),
            blocks: Vec::new(),
            named_blocks: HashMap::new(),
            defined_named_blocks: HashSet::new(),
            controls: Vec::new(),
            current_block: None,
            entry: IrNodeId(0),
            current_owner: TreePath::default(),
            locals_in_memory: false,
            depth: 0,
        }
    }

    pub fn lower_initializer(
        &mut self, owner: TreePath, name: Identifier, ty: Option<TreePath>, expression: ExpressionId,
        location: Location,
    ) -> ProcId {
        let id = self.lower_proc(
            owner,
            name,
            &[],
            false,
            &[Statement::Return(Some(expression))],
            location,
        );

        if let Some(ty) = ty
            && let Some(body) = self.module.proc(id).map(|proc| proc.body)
            && let Some(instructions) = self.module.block(body).map(<[IrNodeId]>::to_vec)
            && let Some(IrNode::Return(Some(value))) = instructions.last().and_then(|id| self.module.node(*id))
        {
            let value = *value;

            if matches!(self.module.node(value), Some(IrNode::New { ty: None, .. })) {
                let ty_node = self.intern_constant(Value::Path(ty));

                if let Some(IrNode::New { ty, .. }) = self.module.nodes.get_mut(value.0 as usize) {
                    *ty = Some(ty_node);
                }
            }
        }

        id
    }

    pub fn finish(mut self) -> Module {
        self.link_global_function_calls();
        // remove instructions after any branch nodes, and repair phi predecessor
        // lists after the removed control-flow edges disappear.
        crate::opt::canonicalize_terminators(&mut self.module);

        crate::opt::simplify_phis(&mut self.module);

        // better constant propagation
        // this:
        //  %1 = 2 + 3
        //  %2 = %1 * 4
        //  return %2
        // becomes:
        //  return 20
        crate::opt::sparse_constant_propagation(&mut self.module);

        // constant branches can leave blocks with no executable path from a
        // procedure entry, remove those blocks and simplify the phis they fed
        crate::opt::eliminate_unreachable_blocks(&mut self.module);
        crate::opt::simplify_phis(&mut self.module);

        // remove unused computations whose evaluation has no observable effect
        crate::opt::eliminate_dead_code(&mut self.module);

        // combine straight-line blocks and remove their intermediate jumps
        crate::opt::merge_linear_blocks(&mut self.module);

        // redirect edges around blocks that only forward to another block
        crate::opt::eliminate_forwarding_blocks(&mut self.module);

        #[cfg(debug_assertions)]
        if let Err(error) = crate::verify(&self.module) {
            panic!("lowering produced invalid IR: {error}");
        }

        self.module
    }

    pub fn unresolved_new(&self) -> &[UnresolvedNew] { &self.unresolved_new }

    pub fn resolve_new(&mut self, node: IrNodeId, ty: TreePath) { self.set_new_type(node, Some(ty)); }

    #[allow(clippy::too_many_arguments)]
    pub fn lower_proc(
        &mut self, owner: TreePath, name: Identifier, params: &[ast::ProcParam], variadic: bool, body: &[Statement],
        location: Location,
    ) -> ProcId {
        self.lower_proc_with_intrinsic(owner, name, params, variadic, body, None, location)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn lower_proc_with_intrinsic(
        &mut self, owner: TreePath, name: Identifier, params: &[ast::ProcParam], variadic: bool, body: &[Statement],
        intrinsic: Option<Intrinsic>, location: Location,
    ) -> ProcId {
        let locals_in_memory = statements_contain_try(body);

        self.lower_function(
            owner,
            name,
            params,
            variadic,
            intrinsic,
            location,
            locals_in_memory,
            |this| this.lower_statements(body),
        )
    }

    /// `var/cache[8]` builds a fresh list for every instance even without an initializer.
    pub fn lower_sized_initializer(
        &mut self, owner: TreePath, name: Identifier, dimensions: &[Option<ExpressionId>], location: Location,
    ) -> ProcId {
        self.lower_function(owner, name, &[], false, None, location, false, |this| {
            let dimensions = dimensions
                .iter()
                .map(|dimension| dimension.map(|expression| this.lower_expr(expression)))
                .collect::<Vec<_>>();
            let list = this.declared_array(&dimensions);
            this.terminate_current_block(IrNode::Return(Some(list)));
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn lower_function(
        &mut self, owner: TreePath, name: Identifier, params: &[ast::ProcParam], variadic: bool,
        intrinsic: Option<Intrinsic>, location: Location, locals_in_memory: bool, body: impl FnOnce(&mut Self),
    ) -> ProcId {
        self.reset_proc();
        self.current_owner = owner.clone();

        let id = ProcId(self.module.procs.len() as u32);
        let function = self.make_node(IrNode::Function(id));
        let parameters = params
            .iter()
            .enumerate()
            .map(|(index, _)| self.make_node(IrNode::FunctionParameter(index as u32)))
            .collect::<Vec<_>>();
        let entry = self.make_block();
        self.entry = entry;
        self.seal_block(entry);
        self.set_current_block(entry);

        let mut lowered = Vec::new();
        for (param, value) in params.iter().zip(parameters.iter().copied()) {
            let spec = self.spec(&param.spec);
            let var = self.declare_var(spec.clone());
            self.parameters.push(var);
            self.write_variable(var, entry, value);
            lowered.push(ProcParam {
                spec,
                default: param.default.map(|id| self.lower_expr(id)),
                in_list: param.in_list.map(|id| self.lower_expr(id)),
            });
        }

        if locals_in_memory {
            let parameters = self.parameters.clone();
            for binding in parameters {
                let value = self.read_variable(binding, entry);
                let pointer = self.frame_local(binding);
                self.stored_bindings.insert(binding, pointer);
                self.emit_instr(IrNode::Store { pointer, value });
            }
            self.locals_in_memory = true;
        }

        body(self);
        self.terminate_current_block(IrNode::Return(None));

        let named_blocks = self
            .named_blocks
            .values()
            .copied()
            .chain(self.defined_named_blocks.iter().copied())
            .collect::<HashSet<_>>();
        for block in named_blocks {
            self.seal_block(block);
        }

        self.terminate_open_blocks();

        let vars = std::mem::take(&mut self.vars);
        self.module.procs.push(Procedure {
            function,
            parameters,
            previous: None,
            owner,
            name,
            params: lowered,
            variadic,
            vars,
            body: entry,
            intrinsic,
            location,
        });

        id
    }

    fn reset_proc(&mut self) {
        self.scopes.clear();
        self.scopes.push(HashMap::new());
        self.named_variables.clear();
        self.vars.clear();
        self.parameters.clear();
        self.current_def.clear();
        self.stored_bindings.clear();
        self.incomplete_phis.clear();
        self.phi_block.clear();
        self.preds.clear();
        self.sealed.clear();
        self.blocks.clear();
        self.named_blocks.clear();
        self.defined_named_blocks.clear();
        self.controls.clear();
        self.current_block = None;
        self.locals_in_memory = false;
        self.depth = 0;
    }

    fn make_node(&mut self, node: IrNode) -> IrNodeId {
        let id = IrNodeId(self.module.nodes.len() as u32);
        self.module.nodes.push(node);

        id
    }

    fn make_block(&mut self) -> IrNodeId {
        let block = self.make_node(IrNode::Label(Vec::new()));
        self.blocks.push(block);

        block
    }

    fn intern_constant(&mut self, value: Value) -> IrNodeId {
        let key = ConstantKey::from(&value);
        if let Some(id) = self.constants.get(&key) {
            return *id;
        }

        let id = self.make_node(IrNode::Constant(value));
        self.constants.insert(key, id);
        self.module.constants.push(id);

        id
    }

    fn intern_external_function(&mut self, name: Identifier) -> IrNodeId {
        if let Some(id) = self.external_functions.get(&name) {
            return *id;
        }

        let id = self.make_node(IrNode::ExternalFunction(name.clone()));
        self.external_functions.insert(name, id);
        self.module.external_functions.push(id);

        id
    }

    fn named_variable(&mut self, name: &Identifier) -> IrNodeId {
        if let Some(id) = self.named_variables.get(name) {
            return *id;
        }

        let declaration_index = self.named_variables.len();
        let id = self.make_node(IrNode::Variable(name.clone()));
        self.named_variables.insert(name.clone(), id);
        if let Some(IrNode::Label(instructions)) = self.module.nodes.get_mut(self.entry.0 as usize) {
            instructions.insert(declaration_index, id);
        }

        id
    }

    fn link_global_function_calls(&mut self) {
        let globals = self
            .module
            .procs
            .iter()
            .filter(|proc| proc.owner == TreePath::default())
            .map(|proc| (proc.name.clone(), proc.function))
            .collect::<HashMap<_, _>>();
        let shadowed = self
            .module
            .procs
            .iter()
            .filter(|proc| proc.owner != TreePath::default())
            .map(|proc| proc.name.clone())
            .collect::<HashSet<_>>();
        let mut direct_calls = Vec::new();

        for (index, proc) in self.module.procs.iter().enumerate() {
            let from_global = proc.owner == TreePath::default();
            let start = proc.function.0 as usize + 1;
            let end = self
                .module
                .procs
                .get(index + 1)
                .map_or(self.module.nodes.len(), |next| next.function.0 as usize);
            for (offset, node) in self.module.nodes.get(start..end).unwrap_or_default().iter().enumerate() {
                let IrNode::FunctionCall { function, args } = node else {
                    continue;
                };
                let Some(IrNode::ExternalFunction(name)) = self.module.node(*function) else {
                    continue;
                };
                let Some(internal) = globals.get(name) else {
                    continue;
                };
                if !from_global && shadowed.contains(name) {
                    continue;
                }

                direct_calls.push((IrNodeId((start + offset) as u32), *function, *internal, args.clone()));
            }
        }

        for (call, _, function, args) in &direct_calls {
            self.module.nodes[call.0 as usize] = IrNode::FunctionCall {
                function: *function,
                args: args.clone(),
            };
        }

        let candidates = direct_calls
            .iter()
            .map(|(_, external, ..)| *external)
            .collect::<HashSet<_>>();
        let referenced = self
            .module
            .nodes
            .iter()
            .flat_map(IrNode::operands)
            .collect::<HashSet<_>>();
        let removed = candidates
            .into_iter()
            .filter(|external| !referenced.contains(external))
            .collect::<HashSet<_>>();
        self.module
            .external_functions
            .retain(|external| !removed.contains(external));
        for external in removed {
            self.module.nodes[external.0 as usize] = IrNode::Noop;
        }
    }

    fn call_target(&mut self, expression: ExpressionId) -> Option<IrNodeId> {
        let Some(Expression::Identifier(name)) = self.ast.get_expr(expression) else {
            return None;
        };

        Some(self.intern_external_function(name.clone()))
    }

    fn set_current_block(&mut self, block: IrNodeId) { self.current_block = Some(block); }

    fn emit_instr(&mut self, node: IrNode) -> IrNodeId {
        let id = self.make_node(node);

        if let Some(IrNode::Label(instructions)) = self
            .current_block
            .and_then(|block| self.module.nodes.get_mut(block.0 as usize))
        {
            instructions.push(id);
        }

        id
    }

    fn terminate_current_block(&mut self, node: IrNode) {
        let Some(block) = self.current_block else {
            return;
        };

        for target in branch_targets(&node) {
            self.preds.entry(target).or_default().push(block);
        }

        self.emit_instr(node);
        self.current_block = None;
    }

    fn fall_through(&mut self, next: IrNodeId) {
        self.terminate_current_block(IrNode::Branch(next));
        self.set_current_block(next);
    }

    fn resume(&mut self, block: IrNodeId) {
        self.current_block = self
            .preds
            .get(&block)
            .is_some_and(|predecessors| !predecessors.is_empty())
            .then_some(block);
    }

    fn terminate_open_blocks(&mut self) {
        let open = self
            .blocks
            .iter()
            .copied()
            .filter(|block| {
                matches!(
                    self.module.node(*block),
                    Some(IrNode::Label(instructions))
                    if !instructions
                        .last()
                        .and_then(|id| self.module.node(*id))
                        .is_some_and(IrNode::is_terminator)
                )
            })
            .collect::<Vec<_>>();

        for block in open {
            let terminator = self.make_node(IrNode::Return(None));
            if let Some(IrNode::Label(instructions)) = self.module.nodes.get_mut(block.0 as usize) {
                instructions.push(terminator);
            }
        }
    }

    fn named_block(&mut self, name: &Identifier) -> IrNodeId {
        if let Some(block) = self.named_blocks.get(name) {
            return *block;
        }

        let block = self.make_block();
        self.named_blocks.insert(name.clone(), block);

        block
    }

    fn define_named_block(&mut self, name: &Identifier) -> IrNodeId {
        let block = self.named_block(name);
        let block = if self.defined_named_blocks.contains(&block) {
            let block = self.make_block();
            self.named_blocks.insert(name.clone(), block);
            block
        } else {
            block
        };
        self.defined_named_blocks.insert(block);

        block
    }

    fn break_target(&self, name: &Option<Identifier>) -> Option<IrNodeId> {
        match name {
            None => self
                .controls
                .iter()
                .rev()
                .find(|target| target.continue_block.is_some()),
            Some(name) => self
                .controls
                .iter()
                .rev()
                .find(|target| target.name.as_ref() == Some(name)),
        }
        .map(|target| target.break_block)
    }

    fn continue_target(&self, name: &Option<Identifier>) -> Option<IrNodeId> {
        self.controls
            .iter()
            .rev()
            .find(|target| {
                target.continue_block.is_some() && name.as_ref().is_none_or(|name| target.name.as_ref() == Some(name))
            })
            .and_then(|target| target.continue_block)
    }

    fn enter_loop(
        &mut self, name: Option<&Identifier>, header: IrNodeId, continue_block: IrNodeId, merge_block: IrNodeId,
    ) {
        let merge = self.make_node(IrNode::LoopMerge {
            merge_block,
            continue_block,
        });

        if let Some(IrNode::Label(instructions)) = self.module.nodes.get_mut(header.0 as usize) {
            instructions.push(merge);
        }

        self.controls.push(ControlTarget {
            name: name.cloned(),
            break_block: merge_block,
            continue_block: Some(continue_block),
        });
    }

    fn selection_merge(&mut self, merge_block: IrNodeId) { self.emit_instr(IrNode::SelectionMerge { merge_block }); }

    fn write_variable(&mut self, var: BindingId, block: IrNodeId, value: IrNodeId) {
        if let Some(pointer) = self.stored_bindings.get(&var).copied() {
            self.emit_instr(IrNode::Store { pointer, value });

            return;
        }
        self.current_def.insert((var, block), value);
    }

    fn read_variable(&mut self, var: BindingId, block: IrNodeId) -> IrNodeId {
        if let Some(pointer) = self.stored_bindings.get(&var).copied() {
            return self.emit_instr(IrNode::Load { pointer });
        }
        if let Some(value) = self.current_def.get(&(var, block)) {
            return *value;
        }

        self.read_variable_recursive(var, block)
    }

    fn read_variable_recursive(&mut self, var: BindingId, block: IrNodeId) -> IrNodeId {
        let value = if !self.sealed.contains(&block) {
            let phi = self.new_phi(block);
            self.incomplete_phis.entry(block).or_default().insert(var, phi);

            phi
        } else {
            let preds = self.preds.get(&block).cloned().unwrap_or_default();

            match preds.as_slice() {
                [] => self.undefined(),
                [single] => {
                    let single = *single;

                    self.read_variable(var, single)
                },
                _ => {
                    let phi = self.new_phi(block);
                    self.write_variable(var, block, phi);
                    self.add_phi_operands(var, phi);

                    phi
                },
            }
        };

        self.write_variable(var, block, value);

        value
    }

    fn new_phi(&mut self, block: IrNodeId) -> IrNodeId {
        let phi = self.make_node(IrNode::Phi { operands: Vec::new() });
        self.phi_block.insert(phi, block);

        if let Some(IrNode::Label(instructions)) = self.module.nodes.get_mut(block.0 as usize) {
            instructions.insert(0, phi);
        }

        phi
    }

    fn undefined(&mut self) -> IrNodeId { self.intern_constant(Value::Null) }

    fn add_phi_operands(&mut self, var: BindingId, phi: IrNodeId) {
        let Some(block) = self.phi_block.get(&phi).copied() else {
            return;
        };
        let preds = self.preds.get(&block).cloned().unwrap_or_default();

        for pred in preds {
            let value = self.read_variable(var, pred);
            self.append_phi_operand(phi, pred, value);
        }
    }

    fn append_phi_operand(&mut self, phi: IrNodeId, block: IrNodeId, value: IrNodeId) {
        if let Some(IrNode::Phi { operands, .. }) = self.module.nodes.get_mut(phi.0 as usize) {
            operands.push(PhiOperand { block, value });
        }
    }

    fn seal_block(&mut self, block: IrNodeId) {
        if self.sealed.contains(&block) {
            return;
        }

        self.sealed.insert(block);
        let pending = self.incomplete_phis.remove(&block).unwrap_or_default();

        for (var, phi) in pending {
            self.add_phi_operands(var, phi);
        }
    }

    fn declare_var(&mut self, spec: VarSpec<IrNodeId>) -> BindingId {
        let id = BindingId(self.vars.len() as u32);
        let stored = spec.modifiers.is_static || self.locals_in_memory;
        let static_local = spec.modifiers.is_static;

        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(spec.name.clone(), id);
        }
        self.vars.push(spec);
        if stored {
            let pointer = if static_local {
                let proc = self.module.procs.len();
                self.make_node(IrNode::Variable(format!("__dmed_static_local_{proc}_{}", id.0).into()))
            } else {
                self.frame_local(id)
            };
            self.stored_bindings.insert(id, pointer);
        }

        id
    }

    fn frame_local(&mut self, binding: BindingId) -> IrNodeId {
        let proc = self.module.procs.len();

        self.make_node(IrNode::Variable(
            format!("__dmed_frame_local_{proc}_{}", binding.0).into(),
        ))
    }

    fn declare_temp(&mut self, name: &str) -> BindingId {
        BindingId({
            let id = self.vars.len() as u32;
            self.vars.push(VarSpec {
                name: Identifier::from(name),
                var_type: None,
                modifiers: VarModifiers::default(),
                dimensions: Vec::new(),
                as_type: None,
            });

            id
        })
    }

    fn lookup(&self, name: &Identifier) -> Option<BindingId> {
        self.scopes.iter().rev().find_map(|scope| scope.get(name)).copied()
    }

    fn spec(&mut self, spec: &ast::VarSpec) -> VarSpec<IrNodeId> {
        VarSpec {
            name: spec.name.clone(),
            var_type: spec.var_type.clone(),
            modifiers: spec.modifiers,
            dimensions: spec.dimensions.iter().map(|e| e.map(|e| self.lower_expr(e))).collect(),
            as_type: spec.as_type.clone(),
        }
    }

    fn args(&mut self, args: &[ast::Argument]) -> Vec<Argument> {
        args.iter()
            .map(|a| Argument {
                key: a.key.map(|e| match self.ast.get_expr(e) {
                    Some(Expression::Identifier(name)) => self.intern_constant(Value::Text(name.to_string())),
                    _ => self.lower_expr(e),
                }),
                value: a.value.map(|e| self.lower_expr(e)),
            })
            .collect()
    }

    fn declared_array(&mut self, dimensions: &[Option<IrNodeId>]) -> IrNodeId {
        let null = self.undefined();
        let ty = self.intern_constant(Value::Path(TreePath::parse("/list")));
        let args = dimensions
            .iter()
            .map(|dimension| Argument {
                key: None,
                value: Some(dimension.unwrap_or(null)),
            })
            .collect::<Vec<_>>();

        self.emit_instr(IrNode::New { ty: Some(ty), args })
    }

    fn lower_expr(&mut self, id: ExpressionId) -> IrNodeId {
        if self.depth >= 256 {
            return self.emit_instr(IrNode::Trap {
                reason: "expression nesting exceeds 256".into(),
            });
        }

        self.depth += 1;
        let value = self.lower_expr_inner(id);
        self.depth -= 1;

        value
    }

    fn lower_expr_inner(&mut self, id: ExpressionId) -> IrNodeId {
        let node = match self.ast.get_expr(id) {
            None => IrNode::Trap {
                reason: "missing AST expression".into(),
            },
            Some(Expression::Literal(literal)) => {
                return self.intern_constant(match literal {
                    Literal::Null => Value::Null,
                    Literal::Num(n) => Value::Num(*n),
                    Literal::String(s) => Value::Text(core::types::decode_string(s)),
                    Literal::Resource(s) => Value::Resource(s.clone()),
                });
            },
            Some(Expression::Identifier(name)) => match (self.lookup(name), self.current_block) {
                (Some(var), Some(block)) => return self.read_variable(var, block),
                _ => {
                    let name = name.clone();
                    let pointer = self.named_variable(&name);

                    IrNode::Load { pointer }
                },
            },
            Some(Expression::Path(path)) => return self.intern_constant(Value::Path(path.clone())),
            Some(Expression::Builtin(builtin)) => IrNode::Builtin(*builtin),
            Some(Expression::Grouped(inner)) => return self.lower_expr(*inner),
            Some(Expression::InterpString { chunks, expressions }) => {
                let chunks = chunks.iter().map(|s| core::types::decode_string(s)).collect();
                let values = expressions
                    .iter()
                    .map(|e| match e {
                        Some(e) => self.lower_expr(*e),
                        None => self.undefined(),
                    })
                    .collect();

                IrNode::Interpolate { chunks, values }
            },
            Some(Expression::Unary { op, operand }) => {
                if matches!(
                    op,
                    UnaryOp::PreIncrement | UnaryOp::PreDecrement | UnaryOp::PostIncrement | UnaryOp::PostDecrement
                ) {
                    return self.increment(*op, *operand);
                }

                let operand = self.lower_expr(*operand);

                IrNode::Unary { op: *op, operand }
            },
            Some(Expression::Binary { op, lhs_expr, rhs_expr }) => match op {
                BinaryOp::LogicalAnd | BinaryOp::LogicalOr => {
                    return self.short_circuit(*op, *lhs_expr, *rhs_expr);
                },
                // `locate(/obj/item) in loc` searches `loc` rather than testing membership
                BinaryOp::In
                    if let Some(Expression::Call { callee, args }) = self.ast.get_expr(*lhs_expr)
                        && matches!(self.ast.get_expr(*callee), Some(Expression::Identifier(name)) if name.as_str() == "locate")
                        && args.len() == 1
                        && let Some(function) = self.call_target(*callee) =>
                {
                    let mut lowered = self.args(args);
                    let container = self.lower_expr(*rhs_expr);
                    lowered.push(Argument {
                        key: None,
                        value: Some(container),
                    });

                    IrNode::FunctionCall {
                        function,
                        args: lowered,
                    }
                },
                op => {
                    let lhs = self.lower_expr(*lhs_expr);
                    let rhs = self.lower_expr(*rhs_expr);

                    IrNode::Binary { op: *op, lhs, rhs }
                },
            },
            Some(Expression::Assign {
                kind,
                lhs_expr,
                rhs_expr,
            }) => return self.assign(*kind, *lhs_expr, *rhs_expr),
            Some(Expression::Ternary {
                condition,
                true_expr,
                false_expr,
            }) => return self.ternary(*condition, *true_expr, *false_expr),
            Some(Expression::Field { object, name, access }) => {
                let object = self.lower_expr(*object);

                IrNode::AccessField {
                    object,
                    name: name.clone(),
                    access: *access,
                }
            },
            Some(Expression::Index {
                object,
                index,
                conditional,
            }) => {
                let object = self.lower_expr(*object);
                let index = self.lower_expr(*index);

                IrNode::Index {
                    object,
                    index,
                    conditional: *conditional,
                }
            },
            Some(Expression::Call { callee, args }) => {
                if matches!(self.ast.get_expr(*callee), Some(Expression::Identifier(name)) if name.as_str() == "nameof")
                    && let [argument] = args.as_slice()
                    && let Some(value) = argument.value
                    && let Some(name) = self.referenced_name(value)
                {
                    return self.intern_constant(Value::Text(name));
                }

                if matches!(self.ast.get_expr(*callee), Some(Expression::Identifier(name)) if name.as_str() == "initial")
                    && let [argument] = args.as_slice()
                    && let Some(value) = argument.value
                {
                    match self.ast.get_expr(value) {
                        Some(Expression::Field { object, name, .. }) => {
                            let object = self.lower_expr(*object);

                            return self.emit_instr(IrNode::Initial {
                                object: Some(object),
                                name: name.clone(),
                            });
                        },
                        Some(Expression::Identifier(name)) if self.lookup(name).is_none() => {
                            return self.emit_instr(IrNode::Initial {
                                object: None,
                                name: name.clone(),
                            });
                        },
                        _ => {},
                    }
                }
                match matches!(
                    self.ast.get_expr(*callee),
                    Some(Expression::Builtin(Builtin::SuperProc))
                ) {
                    true => {
                        let (args, forwards_extra_args) = if args.is_empty() {
                            let bindings = self.parameters.clone();
                            let block = self.current_block.unwrap_or(self.entry);
                            (
                                bindings
                                    .into_iter()
                                    .map(|binding| Argument {
                                        key: None,
                                        value: Some(self.read_variable(binding, block)),
                                    })
                                    .collect::<Vec<_>>(),
                                true,
                            )
                        } else {
                            (self.args(args), false)
                        };
                        IrNode::Super {
                            args,
                            forwards_extra_args,
                        }
                    },
                    false if let Some(function) = self.call_target(*callee) => {
                        let mut lowered = self.args(args);
                        if matches!(self.ast.get_expr(*callee), Some(Expression::Identifier(name)) if name.as_str() == "istype")
                            && args.len() == 1
                            && let Some(ty) = args
                                .first()
                                .and_then(|argument| argument.value)
                                .and_then(|expression| self.inferred_type(expression))
                        {
                            lowered.push(Argument {
                                key: None,
                                value: Some(self.intern_constant(Value::Path(ty))),
                            });
                        }
                        IrNode::FunctionCall {
                            function,
                            args: lowered,
                        }
                    },
                    false => {
                        let callee = self.lower_expr(*callee);

                        IrNode::Call {
                            callee,
                            args: self.args(args),
                        }
                    },
                }
            },
            Some(Expression::New { type_expr, args }) => IrNode::New {
                ty: type_expr.map(|e| self.lower_expr(e)),
                args: self.args(args),
            },
            Some(Expression::ModifiedType { path, overrides }) => IrNode::ModifiedType {
                path: path.clone(),
                overrides: overrides
                    .iter()
                    .map(|(n, e)| (n.clone(), self.lower_expr(*e)))
                    .collect(),
            },
            Some(Expression::List(args)) => IrNode::List(self.args(args)),
            Some(Expression::Pick(values)) => IrNode::Pick(
                values
                    .iter()
                    .map(|v| (v.weight.map(|e| self.lower_expr(e)), self.lower_expr(v.value)))
                    .collect(),
            ),
            Some(Expression::InRange {
                value,
                start,
                end,
                step,
            }) => IrNode::InRange {
                value: self.lower_expr(*value),
                start: self.lower_expr(*start),
                end: self.lower_expr(*end),
                step: step.map(|e| self.lower_expr(e)),
            },
            Some(Expression::Range { start, end, step }) => IrNode::Range {
                start: self.lower_expr(*start),
                end: self.lower_expr(*end),
                step: step.map(|e| self.lower_expr(e)),
            },
            Some(Expression::Input { .. }) => IrNode::Blocked("input"),
        };

        self.emit_instr(node)
    }

    fn increment(&mut self, op: UnaryOp, target: ExpressionId) -> IrNodeId {
        let old = self.lower_expr(target);
        let one = self.intern_constant(Value::Num(1.0));
        let binary_op = match op {
            UnaryOp::PreIncrement | UnaryOp::PostIncrement => BinaryOp::Add,
            UnaryOp::PreDecrement | UnaryOp::PostDecrement => BinaryOp::Sub,
            _ => return old,
        };
        let new = self.emit_instr(IrNode::Binary {
            op: binary_op,
            lhs: old,
            rhs: one,
        });
        self.store(target, new);

        match op {
            UnaryOp::PostIncrement | UnaryOp::PostDecrement => old,
            _ => new,
        }
    }

    fn short_circuit(&mut self, op: BinaryOp, lhs_expr: ExpressionId, rhs_expr: ExpressionId) -> IrNodeId {
        let name = match op {
            BinaryOp::LogicalAnd => "$and",
            _ => "$or",
        };
        let temp = self.declare_temp(name);
        let lhs = self.lower_expr(lhs_expr);
        let Some(block) = self.current_block else {
            return lhs;
        };
        self.write_variable(temp, block, lhs);

        let rest = self.make_block();
        let merge = self.make_block();
        self.selection_merge(merge);
        let (true_block, false_block) = match op {
            BinaryOp::LogicalAnd => (rest, merge),
            _ => (merge, rest),
        };
        self.terminate_current_block(IrNode::ConditionalBranch {
            condition: lhs,
            true_block,
            false_block,
        });
        self.seal_block(rest);

        self.set_current_block(rest);
        let rhs = self.lower_expr(rhs_expr);
        if let Some(block) = self.current_block {
            self.write_variable(temp, block, rhs);
        }
        self.terminate_current_block(IrNode::Branch(merge));

        self.seal_block(merge);
        self.set_current_block(merge);

        self.read_variable(temp, merge)
    }

    fn ternary(&mut self, condition: ExpressionId, true_expr: ExpressionId, false_expr: ExpressionId) -> IrNodeId {
        let temp = self.declare_temp("$ternary");
        let condition = self.lower_expr(condition);
        let yes = self.make_block();
        let no = self.make_block();
        let merge = self.make_block();

        self.selection_merge(merge);
        self.terminate_current_block(IrNode::ConditionalBranch {
            condition,
            true_block: yes,
            false_block: no,
        });
        self.seal_block(yes);
        self.seal_block(no);

        for (block, expression) in [(yes, true_expr), (no, false_expr)] {
            self.set_current_block(block);
            let value = self.lower_expr(expression);
            if let Some(block) = self.current_block {
                self.write_variable(temp, block, value);
            }
            self.terminate_current_block(IrNode::Branch(merge));
        }

        self.seal_block(merge);
        self.set_current_block(merge);

        self.read_variable(temp, merge)
    }

    fn assign(&mut self, kind: ast::AssignmentKind, lhs_expr: ExpressionId, rhs_expr: ExpressionId) -> IrNodeId {
        use ast::AssignmentKind as Kind;

        // `a ||= b` is `a || (a = b)`, which keeps the short circuit
        if matches!(kind, Kind::LogicalAndSet | Kind::LogicalOrSet) {
            let op = match kind {
                Kind::LogicalAndSet => BinaryOp::LogicalAnd,
                _ => BinaryOp::LogicalOr,
            };
            let value = self.short_circuit(op, lhs_expr, rhs_expr);

            return self.store(lhs_expr, value);
        }

        let value = match compound(kind) {
            None => self.lower_expr(rhs_expr),
            Some(op) => {
                let lhs = self.lower_expr(lhs_expr);
                let rhs = self.lower_expr(rhs_expr);

                self.emit_instr(IrNode::CompoundBinary { op, lhs, rhs })
            },
        };

        let inferred = match self.ast.get_expr(lhs_expr) {
            Some(Expression::Identifier(name)) => self
                .lookup(name)
                .and_then(|binding| self.vars.get(binding.0 as usize))
                .and_then(|spec| spec.var_type.clone())
                .or_else(|| self.defer_new_type(value, name)),
            Some(Expression::Field { object, name, .. })
                if matches!(self.ast.get_expr(*object), Some(Expression::Builtin(Builtin::Src))) =>
            {
                self.defer_new_type(value, name)
            },
            _ => None,
        };
        self.set_new_type(value, inferred);

        self.store(lhs_expr, value)
    }

    fn defer_new_type(&mut self, value: IrNodeId, name: &Identifier) -> Option<TreePath> {
        if matches!(self.module.node(value), Some(IrNode::New { ty: None, .. })) {
            self.unresolved_new.push(UnresolvedNew {
                owner: self.current_owner.clone(),
                name: name.clone(),
                node: value,
            });
        }

        None
    }

    fn set_new_type(&mut self, value: IrNodeId, inferred: Option<TreePath>) {
        if !matches!(self.module.node(value), Some(IrNode::New { ty: None, .. })) {
            return;
        }
        let Some(inferred) = inferred else {
            return;
        };
        let ty = self.intern_constant(Value::Path(inferred));
        if let Some(IrNode::New { ty: target, .. }) = self.module.nodes.get_mut(value.0 as usize) {
            *target = Some(ty);
        }
    }

    /// `/datum/foo/proc/bar` and `type::bar` and `src.bar` all name `bar`.
    fn referenced_name(&self, expression: ExpressionId) -> Option<String> {
        match self.ast.get_expr(expression)? {
            Expression::Path(path) => path.name().map(Identifier::to_string),
            Expression::Field { name, .. } | Expression::Identifier(name) => Some(name.to_string()),
            _ => None,
        }
    }

    fn inferred_type(&self, expression: ExpressionId) -> Option<TreePath> {
        match self.ast.get_expr(expression) {
            Some(Expression::Identifier(name)) => self
                .lookup(name)
                .and_then(|binding| self.vars.get(binding.0 as usize))
                .and_then(|spec| spec.var_type.clone()),
            Some(Expression::Builtin(Builtin::Src)) => Some(self.current_owner.clone()),
            _ => None,
        }
    }

    fn store(&mut self, target: ExpressionId, value: IrNodeId) -> IrNodeId {
        match self.ast.get_expr(target) {
            Some(Expression::Identifier(name)) => match (self.lookup(name), self.current_block) {
                (Some(var), Some(block)) => self.write_variable(var, block, value),
                _ => {
                    let name = name.clone();
                    let pointer = self.named_variable(&name);
                    self.emit_instr(IrNode::Store { pointer, value });
                },
            },
            Some(Expression::Field { object, name, access }) => {
                let (name, access) = (name.clone(), *access);
                let object = self.lower_expr(*object);
                self.emit_instr(IrNode::SetField {
                    object,
                    name,
                    access,
                    value,
                });
            },
            Some(Expression::Index {
                object,
                index,
                conditional,
            }) => {
                let object = self.lower_expr(*object);
                let index = self.lower_expr(*index);
                self.emit_instr(IrNode::SetIndex {
                    object,
                    index,
                    value,
                    conditional: *conditional,
                });
            },
            Some(Expression::Builtin(builtin)) => {
                self.emit_instr(IrNode::StoreBuiltin {
                    builtin: *builtin,
                    value,
                });
            },
            _ => {
                self.emit_instr(IrNode::Trap {
                    reason: "assignment to a non-place".into(),
                });
            },
        }

        value
    }

    fn lower_statements(&mut self, body: &[Statement]) {
        if self.depth >= 256 {
            self.emit_instr(IrNode::Trap {
                reason: "statement nesting exceeds 256".into(),
            });

            return;
        }

        self.depth += 1;
        self.scopes.push(HashMap::new());
        for statement in body {
            if self.current_block.is_none() && !matches!(statement, Statement::Label { .. }) {
                continue;
            }

            self.lower_stmt(statement, None);
        }
        self.scopes.pop();
        self.depth -= 1;
    }

    fn lower_stmt(&mut self, stmt: &Statement, label: Option<&Identifier>) {
        match stmt {
            Statement::Empty | Statement::Setting { .. } if !is_waitfor(stmt) => {},
            Statement::Setting { .. } => {
                self.emit_instr(IrNode::Blocked("set waitfor"));
            },
            Statement::Empty => {},
            Statement::Output { target, value } => {
                let target = self.lower_output_target(*target);
                let value = self.lower_expr(*value);
                self.emit_instr(IrNode::Output { target, value });
            },
            Statement::Input { .. } => {
                self.emit_instr(IrNode::Blocked("input"));
            },
            Statement::Expression(e) => {
                self.lower_expr(*e);
            },
            Statement::Var { spec, initializer, .. } => {
                let spec = self.spec(spec);
                let inferred = spec.var_type.clone();
                let dimensions = spec.dimensions.clone();
                let var = self.declare_var(spec);
                let value = match initializer {
                    Some(e) => self.lower_expr(*e),
                    None if dimensions.is_empty() => self.undefined(),
                    None => self.declared_array(&dimensions),
                };
                self.set_new_type(value, inferred);
                if let Some(block) = self.current_block {
                    if let Some(pointer) = self.stored_bindings.get(&var).copied() {
                        self.emit_instr(IrNode::Initialize { pointer, value });
                    } else {
                        self.write_variable(var, block, value);
                    }
                }
            },
            Statement::Return(e) => {
                let value = e.map(|e| self.lower_expr(e));
                self.terminate_current_block(IrNode::Return(value));
            },
            Statement::If { branches, else_branch } => {
                let merge = self.make_block();

                for (condition, body) in branches {
                    let condition = self.lower_expr(*condition);
                    let taken = self.make_block();
                    let next = self.make_block();
                    self.selection_merge(merge);
                    self.terminate_current_block(IrNode::ConditionalBranch {
                        condition,
                        true_block: taken,
                        false_block: next,
                    });
                    self.seal_block(taken);
                    self.seal_block(next);

                    self.set_current_block(taken);
                    self.lower_statements(body);
                    self.terminate_current_block(IrNode::Branch(merge));

                    self.set_current_block(next);
                }

                if let Some(body) = else_branch {
                    self.lower_statements(body);
                }

                self.terminate_current_block(IrNode::Branch(merge));
                self.seal_block(merge);
                self.resume(merge);
            },
            Statement::While { condition, body } => {
                let header = self.make_block();
                let taken = self.make_block();
                let exit = self.make_block();
                self.terminate_current_block(IrNode::Branch(header));

                self.set_current_block(header);
                let condition = self.lower_expr(*condition);
                self.enter_loop(label, header, header, exit);
                self.terminate_current_block(IrNode::ConditionalBranch {
                    condition,
                    true_block: taken,
                    false_block: exit,
                });
                self.seal_block(taken);

                self.set_current_block(taken);
                self.lower_statements(body);
                self.terminate_current_block(IrNode::Branch(header));
                self.controls.pop();
                self.seal_block(header);

                self.seal_block(exit);
                self.resume(exit);
            },
            Statement::DoWhile { condition, body } => {
                let header = self.make_block();
                let taken = self.make_block();
                let latch = self.make_block();
                let exit = self.make_block();
                self.terminate_current_block(IrNode::Branch(header));

                self.set_current_block(header);
                self.enter_loop(label, header, latch, exit);
                self.terminate_current_block(IrNode::Branch(taken));
                self.seal_block(taken);

                self.set_current_block(taken);
                self.lower_statements(body);
                self.terminate_current_block(IrNode::Branch(latch));
                self.controls.pop();
                self.seal_block(latch);

                self.set_current_block(latch);
                let condition = self.lower_expr(*condition);
                self.terminate_current_block(IrNode::ConditionalBranch {
                    condition,
                    true_block: header,
                    false_block: exit,
                });
                self.seal_block(header);

                self.seal_block(exit);
                self.resume(exit);
            },
            Statement::For(header) => {
                self.scopes.push(HashMap::new());
                match header.as_ref() {
                    ForLoop::Standard {
                        init,
                        condition,
                        step,
                        body,
                    } => self.for_standard(label, init.as_deref(), *condition, step.as_deref(), body),
                    ForLoop::List { key, value, list, body } => self.for_list(label, key, value, *list, body),
                    ForLoop::Range {
                        variable,
                        start,
                        end,
                        step,
                        body,
                    } => self.for_range(label, variable, *start, *end, *step, body),
                }
                self.scopes.pop();
            },
            Statement::Switch { value, cases, default } => {
                let subject = self.lower_expr(*value);
                let merge = self.make_block();

                for case in cases {
                    let condition = self.case_condition(subject, &case.values);
                    let taken = self.make_block();
                    let next = self.make_block();
                    self.selection_merge(merge);
                    self.terminate_current_block(IrNode::ConditionalBranch {
                        condition,
                        true_block: taken,
                        false_block: next,
                    });
                    self.seal_block(taken);
                    self.seal_block(next);

                    self.set_current_block(taken);
                    self.lower_statements(&case.body);
                    self.terminate_current_block(IrNode::Branch(merge));

                    self.set_current_block(next);
                }

                if let Some(body) = default {
                    self.lower_statements(body);
                }

                self.terminate_current_block(IrNode::Branch(merge));
                self.seal_block(merge);
                self.resume(merge);
            },
            Statement::Spawn { .. } => {
                self.emit_instr(IrNode::Blocked("spawn"));
            },
            Statement::TryCatch {
                try_body,
                catch_param,
                catch_body,
            } => {
                let merge = self.make_block();
                let body = self.block_branching_to(try_body, merge);

                self.scopes.push(HashMap::new());
                let catch_binding = catch_param.as_ref().map(|s| {
                    let spec = self.spec(s);
                    self.declare_var(spec)
                });
                let catch = self.make_block();
                let previous = self.current_block;
                if let Some(previous) = previous {
                    self.preds.entry(catch).or_default().push(previous);
                }
                self.seal_block(catch);
                self.set_current_block(catch);
                if let Some(binding) = catch_binding {
                    let value = self.emit_instr(IrNode::CatchValue);
                    self.write_variable(binding, catch, value);
                }
                self.lower_statements(catch_body);
                self.terminate_current_block(IrNode::Branch(merge));
                self.current_block = previous;
                self.scopes.pop();

                self.emit_instr(IrNode::TryCatch { body, catch, merge });
                self.terminate_current_block(IrNode::Branch(merge));
                self.seal_block(merge);
                self.resume(merge);
            },
            Statement::Throw(e) => {
                let value = self.lower_expr(*e);
                self.emit_instr(IrNode::Throw(value));
            },
            Statement::Del(e) => {
                let value = self.lower_expr(*e);
                self.emit_instr(IrNode::Del(value));
            },
            Statement::Break(name) => match self.break_target(name) {
                Some(block) => self.terminate_current_block(IrNode::Branch(block)),
                None => {
                    self.emit_instr(IrNode::Trap {
                        reason: "break outside loop".into(),
                    });
                },
            },
            Statement::Continue(name) => match self.continue_target(name) {
                Some(block) => self.terminate_current_block(IrNode::Branch(block)),
                None => {
                    self.emit_instr(IrNode::Trap {
                        reason: "continue outside loop".into(),
                    });
                },
            },
            Statement::Goto(name) => {
                let block = self.named_block(name);
                self.terminate_current_block(IrNode::Branch(block));
            },
            Statement::Label { name, body } => {
                let block = self.define_named_block(name);
                self.fall_through(block);

                match body.split_first() {
                    Some((only @ (Statement::While { .. } | Statement::DoWhile { .. } | Statement::For(_)), [])) => {
                        self.lower_stmt(only, Some(name));
                    },
                    _ => {
                        let exit = self.make_block();
                        self.controls.push(ControlTarget {
                            name: Some(name.clone()),
                            break_block: exit,
                            continue_block: None,
                        });
                        self.lower_statements(body);
                        self.terminate_current_block(IrNode::Branch(exit));
                        self.controls.pop();
                        self.seal_block(exit);
                        self.resume(exit);
                    },
                }
            },
        }
    }

    fn lower_output_target(&mut self, id: ExpressionId) -> OutputTarget {
        match self.ast.get_expr(id) {
            Some(Expression::Grouped(inner)) => self.lower_output_target(*inner),
            Some(Expression::Field { object, name, access }) => {
                let object = self.lower_expr(*object);

                OutputTarget::Field {
                    object,
                    name: name.clone(),
                    access: *access,
                }
            },
            Some(Expression::Index {
                object,
                index,
                conditional,
            }) => {
                let object = self.lower_expr(*object);
                let index = self.lower_expr(*index);

                OutputTarget::Index {
                    object,
                    index,
                    conditional: *conditional,
                }
            },
            _ => OutputTarget::Value(self.lower_expr(id)),
        }
    }

    fn block_branching_to(&mut self, body: &[Statement], next: IrNodeId) -> IrNodeId {
        let block = self.make_block();
        let previous = self.current_block;

        if let Some(previous) = previous {
            self.preds.entry(block).or_default().push(previous);
        }
        self.seal_block(block);
        self.set_current_block(block);
        self.lower_statements(body);
        self.terminate_current_block(IrNode::Branch(next));
        self.current_block = previous;

        block
    }

    fn case_condition(&mut self, subject: IrNodeId, values: &[ast::SwitchValue]) -> IrNodeId {
        let mut condition = None;

        for value in values {
            let test = match value {
                ast::SwitchValue::Value(e) => {
                    let rhs = self.lower_expr(*e);

                    self.emit_instr(IrNode::Binary {
                        op: BinaryOp::CompEq,
                        lhs: subject,
                        rhs,
                    })
                },
                ast::SwitchValue::Range(a, b) => {
                    let start = self.lower_expr(*a);
                    let end = self.lower_expr(*b);

                    self.emit_instr(IrNode::InRange {
                        value: subject,
                        start,
                        end,
                        step: None,
                    })
                },
            };
            condition = Some(match condition {
                None => test,
                Some(lhs) => self.emit_instr(IrNode::Binary {
                    op: BinaryOp::LogicalOr,
                    lhs,
                    rhs: test,
                }),
            });
        }

        condition.unwrap_or_else(|| self.intern_constant(Value::Num(0.0)))
    }

    fn for_standard(
        &mut self, label: Option<&Identifier>, init: Option<&Statement>, condition: Option<ExpressionId>,
        step: Option<&Statement>, body: &[Statement],
    ) {
        if let Some(init) = init {
            self.lower_stmt(init, None);
        }

        let header = self.make_block();
        let taken = self.make_block();
        let stepping = self.make_block();
        let exit = self.make_block();
        self.terminate_current_block(IrNode::Branch(header));

        self.set_current_block(header);
        match condition {
            Some(condition) => {
                let condition = self.lower_expr(condition);
                self.enter_loop(label, header, stepping, exit);
                self.terminate_current_block(IrNode::ConditionalBranch {
                    condition,
                    true_block: taken,
                    false_block: exit,
                });
            },
            None => {
                self.enter_loop(label, header, stepping, exit);
                self.terminate_current_block(IrNode::Branch(taken));
            },
        }
        self.seal_block(taken);

        self.set_current_block(taken);
        self.lower_statements(body);
        self.terminate_current_block(IrNode::Branch(stepping));
        self.controls.pop();
        self.seal_block(stepping);

        self.set_current_block(stepping);
        if let Some(step) = step {
            self.lower_stmt(step, None);
        }
        self.terminate_current_block(IrNode::Branch(header));
        self.seal_block(header);

        self.seal_block(exit);
        self.resume(exit);
    }

    fn for_list(
        &mut self, label: Option<&Identifier>, key: &Option<ast::LoopBinding>, value: &ast::LoopBinding,
        list: Option<ExpressionId>, body: &[Statement],
    ) {
        let list = match list {
            Some(list) => self.lower_expr(list),
            None => self.undefined(),
        };
        let value_var = self.binding(value);
        let key_var = key.as_ref().map(|key| self.binding(key));
        let ty = value.spec.var_type.clone();
        let iterator = self.emit_instr(IrNode::IterInit {
            list,
            ty,
            value_is_associated: key.is_some(),
        });

        let header = self.make_block();
        let taken = self.make_block();
        let exit = self.make_block();
        self.terminate_current_block(IrNode::Branch(header));

        self.set_current_block(header);
        let condition = self.emit_instr(IrNode::IterNext(iterator));
        self.enter_loop(label, header, header, exit);
        self.terminate_current_block(IrNode::ConditionalBranch {
            condition,
            true_block: taken,
            false_block: exit,
        });
        self.seal_block(taken);

        self.set_current_block(taken);
        let next = self.emit_instr(IrNode::IterValue(iterator));
        self.write_variable(value_var, taken, next);
        if let Some(key_var) = key_var {
            let next = self.emit_instr(IrNode::IterKey(iterator));
            self.write_variable(key_var, taken, next);
        }
        self.lower_statements(body);
        self.terminate_current_block(IrNode::Branch(header));
        self.controls.pop();
        self.seal_block(header);

        self.seal_block(exit);
        self.resume(exit);
    }

    fn for_range(
        &mut self, label: Option<&Identifier>, variable: &ast::LoopBinding, start: ExpressionId, end: ExpressionId,
        step: Option<ExpressionId>, body: &[Statement],
    ) {
        let var = self.binding(variable);
        let start = self.lower_expr(start);
        let end = self.lower_expr(end);
        let step = match step {
            Some(step) => self.lower_expr(step),
            None => self.intern_constant(Value::Num(1.0)),
        };
        if let Some(block) = self.current_block {
            self.write_variable(var, block, start);
        }

        let header = self.make_block();
        let taken = self.make_block();
        let stepping = self.make_block();
        let exit = self.make_block();
        self.terminate_current_block(IrNode::Branch(header));

        self.set_current_block(header);
        let current = self.read_variable(var, header);
        let condition = self.emit_instr(IrNode::RangeTest { current, end, step });
        self.enter_loop(label, header, stepping, exit);
        self.terminate_current_block(IrNode::ConditionalBranch {
            condition,
            true_block: taken,
            false_block: exit,
        });
        self.seal_block(taken);

        self.set_current_block(taken);
        self.lower_statements(body);
        self.terminate_current_block(IrNode::Branch(stepping));
        self.controls.pop();
        self.seal_block(stepping);

        self.set_current_block(stepping);
        let current = self.read_variable(var, stepping);
        let next = self.emit_instr(IrNode::Binary {
            op: BinaryOp::Add,
            lhs: current,
            rhs: step,
        });
        self.write_variable(var, stepping, next);
        self.terminate_current_block(IrNode::Branch(header));
        self.seal_block(header);

        self.seal_block(exit);
        self.resume(exit);
    }

    fn binding(&mut self, binding: &ast::LoopBinding) -> BindingId {
        if binding.declares {
            let spec = self.spec(&binding.spec);

            return self.declare_var(spec);
        }

        match self.lookup(&binding.spec.name) {
            Some(var) => var,
            None => {
                let spec = self.spec(&binding.spec);

                self.declare_var(spec)
            },
        }
    }
}

struct ControlTarget {
    name: Option<Identifier>,
    break_block: IrNodeId,
    continue_block: Option<IrNodeId>,
}

fn branch_targets(node: &IrNode) -> Vec<IrNodeId> {
    match node {
        IrNode::Branch(target) => vec![*target],
        IrNode::ConditionalBranch {
            true_block,
            false_block,
            ..
        } => vec![*true_block, *false_block],
        _ => Vec::new(),
    }
}

fn compound(kind: ast::AssignmentKind) -> Option<BinaryOp> {
    use ast::AssignmentKind as Kind;

    Some(match kind {
        Kind::Assign | Kind::AssignInto | Kind::LogicalAndSet | Kind::LogicalOrSet => return None,
        Kind::CompoundAdd => BinaryOp::Add,
        Kind::CompoundSub => BinaryOp::Sub,
        Kind::CompoundMul => BinaryOp::Mul,
        Kind::CompoundDiv => BinaryOp::Div,
        Kind::CompoundMod => BinaryOp::Mod,
        Kind::CompoundFloatMod => BinaryOp::FloatMod,
        Kind::CompoundPow => BinaryOp::Pow,
        Kind::CompoundAnd => BinaryOp::BitAnd,
        Kind::CompoundOr => BinaryOp::BitOr,
        Kind::CompoundXor => BinaryOp::BitXor,
        Kind::CompoundShl => BinaryOp::ShiftLeft,
        Kind::CompoundShr => BinaryOp::ShiftRight,
    })
}

fn statements_contain_try(statements: &[Statement]) -> bool { statements.iter().any(statement_contains_try) }

fn statement_contains_try(statement: &Statement) -> bool {
    match statement {
        Statement::TryCatch { .. } => true,
        Statement::If { branches, else_branch } => {
            branches.iter().any(|(_, body)| statements_contain_try(body))
                || else_branch.as_deref().is_some_and(statements_contain_try)
        },
        Statement::While { body, .. } | Statement::DoWhile { body, .. } | Statement::Label { body, .. } => {
            statements_contain_try(body)
        },
        Statement::For(loop_) => match loop_.as_ref() {
            ForLoop::Standard { init, step, body, .. } => {
                init.as_deref().is_some_and(statement_contains_try)
                    || step.as_deref().is_some_and(statement_contains_try)
                    || statements_contain_try(body)
            },
            ForLoop::List { body, .. } | ForLoop::Range { body, .. } => statements_contain_try(body),
        },
        Statement::Switch { cases, default, .. } => {
            cases.iter().any(|case| statements_contain_try(&case.body))
                || default.as_deref().is_some_and(statements_contain_try)
        },
        _ => false,
    }
}

fn is_waitfor(stmt: &Statement) -> bool {
    matches!(stmt, Statement::Setting { name, .. } if name.as_str() == "waitfor")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHAPES: [&str; 15] = [
        "/proc/t(a)\n\treturn a\n",
        "/proc/t(a)\n\tif(a)\n\t\treturn 1\n\telse if(a)\n\t\treturn 2\n\telse\n\t\treturn 3\n",
        "/proc/t(a)\n\tvar/x = 0\n\tif(a)\n\t\tx = 1\n\treturn x\n",
        "/proc/t(a)\n\twhile(a)\n\t\ta = a - 1\n\treturn a\n",
        "/proc/t(a)\n\tdo\n\t\ta = a - 1\n\twhile(a)\n",
        "/proc/t(L)\n\tfor(var/x in L)\n\t\tcontinue\n",
        "/proc/t()\n\tfor(var/i = 1 to 10 step 2)\n\t\tbreak\n",
        "/proc/t()\n\tvar/n = 0\n\tfor(var/i = 0, i < 10, i++)\n\t\tn += i\n\treturn n\n",
        "/proc/t(a)\n\tswitch(a)\n\t\tif(1)\n\t\t\treturn 1\n\t\tif(2 to 4)\n\t\t\treturn 2\n\t\telse\n\t\t\treturn \
         3\n",
        "/proc/t(a)\n\touter:\n\t\twhile(a)\n\t\t\twhile(a)\n\t\t\t\tbreak outer\n",
        "/proc/t()\n\ttry\n\t\tthrow 1\n\tcatch(var/e)\n\t\treturn e\n",
        "/proc/t(a, b)\n\treturn a && b || a\n",
        "/proc/t(a)\n\tvar/x = 0\n\tagain:\n\t\tx += 1\n\t\tif(a)\n\t\t\ta = 0\n\t\t\tgoto again\n\t\treturn x\n",
        "/proc/t(mean, stddev)\n\tvar/cached\n\tvar/r1\n\tvar/r2\n\tvar/working\n\tif(cached != null)\n\t\tr1 = \
         cached\n\t\tcached = null\n\telse\n\t\tdo\n\t\t\tr1 = rand(-10000, 10000) / 10000\n\t\t\tr2 = rand(-10000, \
         10000) / 10000\n\t\t\tworking = r1 * r1 + r2 * r2\n\t\twhile(working >= 1 || working == 0)\n\t\tworking = \
         sqrt(-2 * log(working) / working)\n\t\tr1 *= working\n\t\tcached = r2 * working\n\treturn mean + stddev * \
         r1\n",
        "/proc/t(list/L, a, b)\n\tvar/n = 0\n\touter:\n\t\tfor(var/mob/M in L)\n\t\t\tfor(var/i = 1 to \
         10)\n\t\t\t\tif(a && b)\n\t\t\t\t\tcontinue outer\n\t\t\t\telse if(a || i > 3)\n\t\t\t\t\tbreak\n\t\t\t\tn \
         += i\n\tswitch(n)\n\t\tif(1 to 5)\n\t\t\tn = a ? 1 : 2\n\t\telse\n\t\t\tn = 0\n\treturn n\n",
    ];

    fn lower(source: &str) -> Module {
        let (tokens, _) = lexer::tokenize(source);
        let ast = ast::parse(&tokens).expect("fixture should parse");
        let mut builder = IrModuleBuilder::new(&ast);

        for declaration in &ast.declarations {
            let ast::Declaration::Proc { path, params, body, .. } = declaration else {
                continue;
            };
            let name = path.name().cloned().unwrap_or_else(|| "anonymous".into());
            builder.lower_proc(
                TreePath::default(),
                name,
                params,
                false,
                body.as_deref().unwrap_or_default(),
                Location::default(),
            );
        }

        builder.finish()
    }

    fn blocks(module: &Module) -> Vec<(IrNodeId, Vec<IrNodeId>)> {
        module
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(index, node)| match node {
                IrNode::Label(instructions) => Some((IrNodeId(index as u32), instructions.clone())),
                _ => None,
            })
            .collect()
    }

    fn predecessors(module: &Module) -> HashMap<IrNodeId, Vec<IrNodeId>> {
        let mut preds: HashMap<IrNodeId, Vec<IrNodeId>> = HashMap::new();

        for (block, instructions) in blocks(module) {
            for instruction in instructions {
                let Some(node) = module.node(instruction) else {
                    continue;
                };
                for target in branch_targets(node) {
                    preds.entry(target).or_default().push(block);
                }
            }
        }

        preds
    }

    fn nodes(module: &Module) -> Vec<String> { module.nodes.iter().map(|node| format!("{node:?}")).collect() }

    fn has(module: &Module, needle: &str) -> bool { nodes(module).iter().any(|node| node.contains(needle)) }

    /// A block that falls off its end would let the VM run into the next one by accident, and a
    /// merge declaration is only valid immediately before the branch it describes.
    #[test]
    fn every_block_ends_in_exactly_one_terminator() {
        for source in SHAPES {
            let module = lower(source);

            for (id, instructions) in blocks(&module) {
                let terminators = instructions
                    .iter()
                    .filter(|i| module.node(**i).is_some_and(IrNode::is_terminator))
                    .count();

                assert_eq!(terminators, 1, "block {id} in `{source}` has {terminators} terminators");

                for (index, instruction) in instructions.iter().enumerate() {
                    if module.node(*instruction).is_some_and(IrNode::is_merge) {
                        assert_eq!(index + 2, instructions.len(), "merge misplaced in {id} of `{source}`");
                    }
                }
            }
        }
    }

    /// The SSA invariant: one operand per predecessor, naming exactly those predecessors.
    #[test]
    fn every_phi_has_one_operand_per_predecessor() {
        for source in SHAPES {
            let module = lower(source);
            let preds = predecessors(&module);

            for (block, instructions) in blocks(&module) {
                for instruction in instructions {
                    let Some(IrNode::Phi { operands, .. }) = module.node(instruction) else {
                        continue;
                    };
                    let mut named = operands.iter().map(|operand| operand.block).collect::<Vec<_>>();
                    let mut expected = preds.get(&block).cloned().unwrap_or_default();
                    named.sort_by_key(|id| id.0);
                    expected.sort_by_key(|id| id.0);

                    assert_eq!(named, expected, "φ {instruction} in block {block} of `{source}`");
                }
            }
        }
    }

    /// A φ is a merge, so it belongs at the head of its block, before anything that could read it.
    #[test]
    fn phis_come_first_in_their_block() {
        for source in SHAPES {
            let module = lower(source);

            for (block, instructions) in blocks(&module) {
                let last_phi = instructions
                    .iter()
                    .rposition(|i| matches!(module.node(*i), Some(IrNode::Phi { .. })));
                let first_other = instructions
                    .iter()
                    .position(|i| !matches!(module.node(*i), Some(IrNode::Phi { .. })));

                if let (Some(last), Some(first)) = (last_phi, first_other) {
                    assert!(last < first, "φ after an instruction in block {block} of `{source}`");
                }
            }
        }
    }

    /// The phi simplification pass should leave nothing that merges a single value.
    #[test]
    fn no_trivial_phi_survives() {
        for source in SHAPES {
            let module = lower(source);

            for (block, instructions) in blocks(&module) {
                for instruction in instructions {
                    let Some(IrNode::Phi { operands, .. }) = module.node(instruction) else {
                        continue;
                    };
                    let distinct = operands
                        .iter()
                        .map(|operand| operand.value)
                        .filter(|value| *value != instruction)
                        .collect::<HashSet<_>>();

                    assert!(distinct.len() > 1, "trivial φ {instruction} in {block} of `{source}`");
                }
            }
        }
    }

    /// Every operand names a real node, and no instruction still reads a φ that was removed.
    #[test]
    fn every_operand_is_defined() {
        for source in SHAPES {
            let module = lower(source);

            for (block, instructions) in blocks(&module) {
                for instruction in instructions {
                    let Some(node) = module.node(instruction) else {
                        continue;
                    };

                    for operand in node.operands() {
                        let target = module.node(operand);

                        assert!(target.is_some(), "{instruction} in {block} of `{source}` reads nothing");
                        assert!(
                            !matches!(target, Some(IrNode::Noop)),
                            "{instruction} {node:?} in {block} of `{source}` reads removed {operand}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn every_lowered_shape_passes_verification() {
        for source in SHAPES {
            let module = lower(source);

            crate::verify(&module).unwrap_or_else(|error| panic!("invalid IR for `{source}`: {error}"));
        }
    }

    #[test]
    fn a_throw_removes_its_following_edge_and_phi_operand() {
        let module = lower("/proc/t(a)\n\tvar/x = 0\n\tif(a)\n\t\tthrow 1\n\t\tx = 2\n\telse\n\t\tx = 3\n\treturn x\n");
        let throwing_block = blocks(&module)
            .into_iter()
            .find(|(_, instructions)| {
                instructions
                    .iter()
                    .any(|instruction| matches!(module.node(*instruction), Some(IrNode::Throw(_))))
            })
            .expect("fixture should contain a throwing block");

        assert!(matches!(
            throwing_block
                .1
                .last()
                .and_then(|instruction| module.node(*instruction)),
            Some(IrNode::Throw(_))
        ));
        assert!(!module.nodes.iter().any(|node| matches!(node, IrNode::Phi { .. })));
        assert!(module.nodes.iter().any(|node| {
            matches!(
                node,
                IrNode::Return(Some(value))
                    if matches!(module.node(*value), Some(IrNode::Constant(Value::Num(3.0))))
            )
        }));
        crate::verify(&module).expect("canonicalized module should verify");
    }

    #[test]
    fn metadata_does_not_reference_defaults_after_a_noreturn_default() {
        let module = lower("/proc/t(a = input(), b = a + 2)\n\treturn b\n");

        assert!(matches!(
            module.procs[0].params[0]
                .default
                .and_then(|default| module.node(default)),
            Some(IrNode::Blocked("input"))
        ));
        assert_eq!(module.procs[0].params[1].default, None);
        crate::verify(&module).expect("removed defaults should not leave stale metadata");
    }

    #[test]
    fn constants_are_module_scoped_and_interned_across_procedures() {
        let module = lower("/proc/a()\n\treturn 1\n/proc/b()\n\tvar/x = 1\n\treturn x\n");

        assert_eq!(module.constants.len(), 1, "{:?}", nodes(&module));
        let constant = module.constants[0];
        assert!(matches!(module.node(constant), Some(IrNode::Constant(Value::Num(1.0)))));
        assert!(
            blocks(&module)
                .iter()
                .all(|(_, instructions)| !instructions.contains(&constant)),
            "constant {constant} belongs to a basic block"
        );

        let returns = module
            .nodes
            .iter()
            .filter_map(|node| match node {
                IrNode::Return(Some(value)) => Some(*value),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(returns, vec![constant, constant]);
    }

    #[test]
    fn direct_global_calls_target_function_nodes_including_forward_calls() {
        let module = lower("/proc/invoke()\n\treturn target(1)\n/proc/target(value)\n\treturn value\n");
        let callee = module.procs[1].function;
        let calls = module
            .nodes
            .iter()
            .filter_map(|node| match node {
                IrNode::FunctionCall { function, .. } => Some(*function),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(calls, vec![callee]);
        assert!(matches!(module.node(callee), Some(IrNode::Function(_))));
        assert!(
            !module
                .nodes
                .iter()
                .any(|node| matches!(node, IrNode::ExternalFunction(name) if name.as_str() == "target"))
        );
    }

    #[test]
    fn nonlocal_variables_use_explicit_read_and_write_nodes() {
        let module = lower("/proc/test()\n\texternal_value = external_value + 1\n");
        let variables = module
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(index, node)| match node {
                IrNode::Variable(name) if name.as_str() == "external_value" => Some(IrNodeId(index as u32)),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(variables.len(), 1);
        let variable = variables[0];
        assert!(
            module
                .nodes
                .iter()
                .any(|node| matches!(node, IrNode::Load { pointer } if *pointer == variable))
        );
        assert!(
            module
                .nodes
                .iter()
                .any(|node| matches!(node, IrNode::Store { pointer, .. } if *pointer == variable))
        );
    }

    #[test]
    fn external_functions_are_module_scoped_and_interned() {
        let module =
            lower("/proc/a(value)\n\treturn third_party(value)\n/proc/b(value)\n\treturn third_party(value)\n");

        assert_eq!(module.external_functions.len(), 1);
        let external = module.external_functions[0];
        assert!(matches!(
            module.node(external),
            Some(IrNode::ExternalFunction(name)) if name.as_str() == "third_party"
        ));
        assert!(
            blocks(&module)
                .iter()
                .all(|(_, instructions)| !instructions.contains(&external)),
            "external function {external} belongs to a basic block"
        );

        let calls = module
            .nodes
            .iter()
            .filter_map(|node| match node {
                IrNode::FunctionCall { function, .. } => Some(*function),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(calls, vec![external, external]);
    }

    /// The induction variable becomes a φ, and `end` and `step` stay plain values in the preheader.
    #[test]
    fn a_counted_loop_merges_its_induction_variable_with_a_phi() {
        let module = lower("/proc/t()\n\tfor(var/i = 1 to 10 step 2)\n\t\ti = i + 1\n");
        let phis = module
            .nodes
            .iter()
            .filter(|node| matches!(node, IrNode::Phi { .. }))
            .collect::<Vec<_>>();

        assert_eq!(phis.len(), 1, "{:?}", nodes(&module));
        assert!(has(&module, "RangeTest"), "{:?}", nodes(&module));
    }

    #[test]
    fn a_standard_loop_carries_its_incremented_induction_variable() {
        let module = lower("/proc/t(n)\n\tfor(var/i = 0, i < n, i++)\n\t\tcontinue\n\treturn i\n");
        let phis = module
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(index, node)| matches!(node, IrNode::Phi { .. }).then_some(IrNodeId(index as u32)))
            .collect::<Vec<_>>();

        assert_eq!(phis.len(), 1, "{:?}", nodes(&module));
        assert!(
            module
                .nodes
                .iter()
                .any(|node| { matches!(node, IrNode::Binary { op: BinaryOp::CompLess, lhs, .. } if *lhs == phis[0]) })
        );
        let Some(IrNode::Phi { operands }) = module.node(phis[0]) else {
            panic!("expected induction phi")
        };
        assert!(operands.iter().any(|operand| {
            matches!(
                module.node(operand.value),
                Some(IrNode::Binary { op: BinaryOp::Add, .. })
            )
        }));
    }

    /// `a && b` evaluates `b` only when it must, so it is two blocks and a φ.
    #[test]
    fn short_circuit_becomes_branches_and_a_phi() {
        let module = lower("/proc/t(a, b)\n\treturn a && b\n");

        assert!(has(&module, "Phi"), "{:?}", nodes(&module));
        assert_eq!(
            module
                .nodes
                .iter()
                .filter(|node| matches!(node, IrNode::ConditionalBranch { .. }))
                .count(),
            1
        );
    }

    /// A write to a local is SSA bookkeeping; a write to a field is a real memory instruction.
    #[test]
    fn field_and_index_writes_stay_instructions() {
        let module = lower("/proc/t(o, L)\n\to.name = 1\n\tL[1] = 2\n");

        assert!(has(&module, "SetField"), "{:?}", nodes(&module));
        assert!(has(&module, "SetIndex"), "{:?}", nodes(&module));
    }

    #[test]
    fn statements_after_a_return_are_dropped() {
        let module = lower("/proc/t()\n\treturn 1\n\treturn 2\n");
        let returns = module
            .nodes
            .iter()
            .filter(|node| matches!(node, IrNode::Return(_)))
            .count();

        assert_eq!(returns, 1, "{:?}", nodes(&module));
    }

    #[test]
    fn control_flow_after_a_return_does_not_restore_reachability() {
        let module = lower("/proc/t(a)\n\treturn 1\n\tif(a)\n\t\treturn 2\n");

        assert_eq!(blocks(&module).len(), 1, "{:?}", nodes(&module));
        assert!(!has(&module, "ConditionalBranch"), "{:?}", nodes(&module));
    }

    #[test]
    fn builder_resets_control_flow_state_between_procedures() {
        let module = lower("/proc/a()\n\tagain:\n\t\treturn 1\n/proc/b()\n\tagain:\n\t\treturn 2\n");
        let first_entry = module.procs[0].body;
        let second_entry = module.procs[1].body;
        let second_return = module
            .block(second_entry)
            .and_then(|instructions| {
                instructions.iter().find_map(|id| match module.node(*id) {
                    Some(IrNode::Return(Some(value))) => Some(*value),
                    _ => None,
                })
            })
            .and_then(|value| module.node(value));

        assert_ne!(first_entry, second_entry);
        assert!(matches!(second_return, Some(IrNode::Constant(Value::Num(2.0)))));
    }

    #[test]
    fn finishing_a_procedure_does_not_revisit_older_blocks() {
        let (tokens, _) = lexer::tokenize("");
        let ast = ast::parse(&tokens).expect("empty fixture should parse");
        let mut builder = IrModuleBuilder::new(&ast);
        let first = builder.lower_proc(
            TreePath::default(),
            "first".into(),
            &[],
            false,
            &[],
            Location::default(),
        );
        let first_entry = builder.module.proc(first).expect("first procedure").body;
        let Some(IrNode::Label(instructions)) = builder.module.nodes.get_mut(first_entry.0 as usize) else {
            panic!("first entry should be a block");
        };
        instructions.pop();

        builder.lower_proc(
            TreePath::default(),
            "second".into(),
            &[],
            false,
            &[],
            Location::default(),
        );

        assert!(builder.module.block(first_entry).is_some_and(<[IrNodeId]>::is_empty));
    }

    #[test]
    fn an_endless_loop_does_not_fall_through_without_a_break() {
        let module = lower("/proc/t()\n\tfor()\n\t\tcontinue\n\treturn 1\n");

        assert!(!module.nodes.iter().any(|node| {
            matches!(node, IrNode::Return(Some(value)) if matches!(module.node(*value), Some(IrNode::Constant(Value::Num(1.0)))))
        }));
    }

    #[test]
    fn a_backward_goto_contributes_to_the_labels_phi() {
        let module = lower(
            "/proc/t(a)\n\tvar/x = 0\n\tagain:\n\t\tx += 1\n\t\tif(a)\n\t\t\ta = 0\n\t\t\tgoto again\n\t\treturn x\n",
        );

        assert!(
            module
                .nodes
                .iter()
                .any(|node| { matches!(node, IrNode::Phi { operands, .. } if operands.len() == 2) }),
            "{:?}",
            nodes(&module)
        );
    }

    #[test]
    fn a_range_bound_reads_the_value_from_before_loop_initialization() {
        let module = lower("/proc/t(i)\n\tfor(i = 1 to i)\n\t\tbreak\n");
        let parameter = module
            .nodes
            .iter()
            .position(|node| matches!(node, IrNode::FunctionParameter(0)))
            .map(|index| IrNodeId(index as u32));
        let end = module.nodes.iter().find_map(|node| match node {
            IrNode::RangeTest { end, .. } => Some(*end),
            _ => None,
        });

        assert_eq!(end, parameter, "{:?}", nodes(&module));
    }

    /// `break outer` leaves the outer loop, so it branches where the inner loop never does.
    #[test]
    fn a_labelled_break_targets_the_outer_loop_exit() {
        let module = lower("/proc/t(a)\n\touter:\n\t\twhile(a)\n\t\t\twhile(a)\n\t\t\t\tbreak outer\n");
        let merges = module
            .nodes
            .iter()
            .filter_map(|node| match node {
                IrNode::LoopMerge { merge_block, .. } => Some(*merge_block),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(merges.len(), 2);
        assert_ne!(merges[0], merges[1]);

        let outer_exit = merges[0];
        assert!(
            blocks(&module)
                .iter()
                .any(|(_, instructions)| instructions.iter().any(|i| matches!(
                    module.node(*i),
                    Some(IrNode::Branch(target)) if *target == outer_exit
                ) || matches!(
                    module.node(*i),
                    Some(IrNode::ConditionalBranch { true_block, false_block, .. })
                        if *true_block == outer_exit || *false_block == outer_exit
                ))),
            "nothing branches to the outer exit"
        );
    }

    /// Built directly rather than parsed: `ast::parse` overflows its own stack long before 256, so
    /// the guard here is only reachable from an AST that did not come through the parser.
    #[test]
    fn nesting_past_the_limit_lowers_to_trap() {
        let mut expressions = vec![Expression::Literal(Literal::Num(1.0))];
        for index in 0..300 {
            expressions.push(Expression::Grouped(ExpressionId::new(index).expect("expression id")));
        }
        let outermost = ExpressionId::new(expressions.len() - 1).expect("expression id");
        let ast = ast::AST::new(Vec::new(), expressions);

        let mut builder = IrModuleBuilder::new(&ast);
        builder.lower_initializer(TreePath::default(), "deep".into(), None, outermost, Location::default());

        let module = builder.finish();
        assert!(has(&module, "Trap"));
        assert!(has(&module, "nesting exceeds 256"));
    }
}
