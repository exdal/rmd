use core::types::{IrNodeId, ProcId, Value};
use std::{collections::HashSet, fmt::Write};

use crate::{AccessKind, Argument, IrNode, Module, OutputTarget};

pub fn dump(module: &Module) -> String { dump_with(module, false) }

pub fn dump_with(module: &Module, syntax_highlighting: bool) -> String { dump_impl(module, None, syntax_highlighting) }

pub fn dump_selected_with(module: &Module, procedures: &HashSet<ProcId>, syntax_highlighting: bool) -> String {
    dump_impl(module, Some(procedures), syntax_highlighting)
}

fn dump_impl(module: &Module, procedures: Option<&HashSet<ProcId>>, syntax_highlighting: bool) -> String {
    let referenced = procedures.map(|procedures| selected_nodes(module, procedures));
    let included = |id: IrNodeId| referenced.as_ref().is_none_or(|nodes| nodes.contains(&id));
    let displayed_blocks = module
        .procs
        .iter()
        .enumerate()
        .map(|(index, _)| {
            procedures
                .is_none_or(|procedures| procedures.contains(&ProcId(index as u32)))
                .then(|| procedure_blocks(module, index))
        })
        .collect::<Vec<_>>();
    let function_count = displayed_blocks.iter().flatten().count();
    let constant_count = module.constants.iter().filter(|id| included(**id)).count();
    let external_count = module.external_functions.iter().filter(|id| included(**id)).count();
    let blocks = displayed_blocks
        .iter()
        .flatten()
        .map(|(reachable, unreachable)| reachable.len() + unreachable.len())
        .sum::<usize>();
    let node_count = referenced.as_ref().map_or(module.nodes.len(), HashSet::len);
    let width = IrNodeId(module.nodes.len().saturating_sub(1) as u32).to_string().len();

    let mut out = String::new();
    let mut printed = vec![false; module.nodes.len()];
    let mut printed_missing = HashSet::new();
    let _ = writeln!(
        out,
        "; ir module: {} nodes, {} constants, {} external functions, {} functions, {blocks} blocks",
        node_count, constant_count, external_count, function_count
    );

    if constant_count != 0 {
        blank_line(&mut out);
        let _ = writeln!(out, "; constants");
        for id in module.constants.iter().filter(|id| included(**id)) {
            let Some(IrNode::Constant(value)) = module.node(*id) else {
                continue;
            };

            line(&mut out, width, *id, 0, &format!("constant {}", constant(value)));
            mark_printed(&mut printed, *id);
        }
    }

    if external_count != 0 {
        blank_line(&mut out);
        let _ = writeln!(out, "; external functions");
        for id in module.external_functions.iter().filter(|id| included(**id)) {
            let Some(IrNode::ExternalFunction(name)) = module.node(*id) else {
                continue;
            };

            line(&mut out, width, *id, 0, &format!("external_function {name}"));
            mark_printed(&mut printed, *id);
        }
    }

    for (index, proc) in module.procs.iter().enumerate() {
        let Some((reachable, unreachable)) = displayed_blocks[index].as_ref() else {
            continue;
        };
        blank_line(&mut out);
        let mut declaration = format!("function {}::{} vars={}", proc.owner, proc.name, proc.vars.len());

        if let Some(previous) = proc.previous.and_then(|id| module.proc(id)) {
            let _ = write!(declaration, " overrides={}", previous.function);
        }

        if let Some(intrinsic) = proc.intrinsic {
            let _ = write!(declaration, " intrinsic={intrinsic}");
        }

        line(&mut out, width, proc.function, 0, &declaration);
        mark_printed(&mut printed, proc.function);

        for parameter in &proc.parameters {
            let Some(node @ IrNode::FunctionParameter(_)) = module.node(*parameter) else {
                continue;
            };

            line(&mut out, width, *parameter, 1, &format_node(node));
            mark_printed(&mut printed, *parameter);
        }

        for block in reachable.iter().copied() {
            dump_block(module, block, width, &mut out, &mut printed, &mut printed_missing);
        }

        if !unreachable.is_empty() {
            blank_line(&mut out);
            let _ = writeln!(out, "; unreachable blocks in {}::{}", proc.owner, proc.name);
        }
        for block in unreachable.iter().copied() {
            dump_block(module, block, width, &mut out, &mut printed, &mut printed_missing);
        }
    }

    dump_referenced_nodes(module, procedures, width, &printed, &printed_missing, &mut out);

    match syntax_highlighting {
        true => highlight(&out),
        false => out,
    }
}

fn selected_nodes(module: &Module, procedures: &HashSet<ProcId>) -> HashSet<IrNodeId> {
    let mut selected = HashSet::new();
    let mut pending = Vec::new();

    for proc_id in procedures {
        let Some(procedure) = module.proc(*proc_id) else {
            continue;
        };
        mark(procedure.function, &mut selected, &mut pending);
        for parameter in &procedure.parameters {
            mark(*parameter, &mut selected, &mut pending);
        }
        for parameter in &procedure.params {
            for node in parameter
                .default
                .into_iter()
                .chain(parameter.in_list)
                .chain(parameter.spec.dimensions.iter().flatten().copied())
            {
                mark(node, &mut selected, &mut pending);
            }
        }
        for variable in &procedure.vars {
            for dimension in variable.dimensions.iter().flatten() {
                mark(*dimension, &mut selected, &mut pending);
            }
        }
        let (reachable, unreachable) = procedure_blocks(module, proc_id.0 as usize);
        for block in reachable.into_iter().chain(unreachable) {
            mark(block, &mut selected, &mut pending);
            let Some(IrNode::Label(instructions)) = module.node(block) else {
                continue;
            };
            for instruction in instructions {
                mark(*instruction, &mut selected, &mut pending);
            }
        }
    }

    while let Some(id) = pending.pop() {
        if let Some(node) = module.node(id) {
            node.for_each_operand(|operand| mark(operand, &mut selected, &mut pending));
        }
    }

    selected
}

fn procedure_blocks(module: &Module, index: usize) -> (Vec<IrNodeId>, Vec<IrNodeId>) {
    let Some(procedure) = module.procs.get(index) else {
        return (Vec::new(), Vec::new());
    };
    let start = procedure.function.0 as usize + 1;
    let end = module
        .procs
        .get(index + 1)
        .map_or(module.nodes.len(), |next| next.function.0 as usize)
        .min(module.nodes.len());
    let owned = module
        .nodes
        .get(start..end)
        .unwrap_or_default()
        .iter()
        .enumerate()
        .filter_map(|(offset, node)| matches!(node, IrNode::Label(_)).then_some(IrNodeId((start + offset) as u32)))
        .collect::<Vec<_>>();
    let owned_set = owned.iter().copied().collect::<HashSet<_>>();
    let reachable = reachable(module, procedure.body)
        .into_iter()
        .filter(|block| owned_set.contains(block))
        .collect::<Vec<_>>();
    let reachable_set = reachable.iter().copied().collect::<HashSet<_>>();
    let unreachable = owned
        .into_iter()
        .filter(|block| !reachable_set.contains(block))
        .collect::<Vec<_>>();

    (reachable, unreachable)
}

fn dump_block(
    module: &Module, block: IrNodeId, width: usize, out: &mut String, printed: &mut [bool],
    printed_missing: &mut HashSet<IrNodeId>,
) {
    let Some(IrNode::Label(instructions)) = module.node(block) else {
        return;
    };

    blank_line(out);
    line(out, width, block, 1, "label");
    mark_printed(printed, block);

    for instruction in instructions {
        match module.node(*instruction) {
            Some(node) => {
                line(out, width, *instruction, 2, &format_node(node));
                mark_printed(printed, *instruction);
            },
            None => {
                line(out, width, *instruction, 2, "<missing>");
                printed_missing.insert(*instruction);
            },
        }
    }
}

fn mark_printed(printed: &mut [bool], id: IrNodeId) {
    if let Some(slot) = printed.get_mut(id.0 as usize) {
        *slot = true;
    }
}

fn mark(id: IrNodeId, selected: &mut HashSet<IrNodeId>, pending: &mut Vec<IrNodeId>) {
    if selected.insert(id) {
        pending.push(id);
    }
}

fn dump_referenced_nodes(
    module: &Module, procedures: Option<&HashSet<ProcId>>, width: usize, printed: &[bool],
    printed_missing: &HashSet<IrNodeId>, out: &mut String,
) {
    let mut referenced = vec![false; module.nodes.len()];
    let mut missing = HashSet::new();
    let mut pending = Vec::new();

    for (index, was_printed) in printed.iter().copied().enumerate() {
        if !was_printed {
            continue;
        }
        if let Some(node) = module.nodes.get(index) {
            node.for_each_operand(|operand| pending.push(operand));
        }
    }

    for (index, procedure) in module.procs.iter().enumerate() {
        if procedures.is_some_and(|procedures| !procedures.contains(&ProcId(index as u32))) {
            continue;
        }
        for parameter in &procedure.params {
            pending.extend(parameter.default);
            pending.extend(parameter.in_list);
            pending.extend(parameter.spec.dimensions.iter().flatten().copied());
        }
        for variable in &procedure.vars {
            pending.extend(variable.dimensions.iter().flatten().copied());
        }
    }

    while let Some(id) = pending.pop() {
        let index = id.0 as usize;
        let Some(seen) = referenced.get_mut(index) else {
            missing.insert(id);
            continue;
        };
        if *seen {
            continue;
        }
        *seen = true;
        if let Some(node) = module.node(id) {
            node.for_each_operand(|operand| pending.push(operand));
        }
    }

    let detached = referenced
        .iter()
        .copied()
        .enumerate()
        .filter(|(index, referenced)| *referenced && !printed[*index])
        .filter_map(|(index, _)| {
            let id = IrNodeId(index as u32);
            (!matches!(module.node(id), Some(IrNode::Function(_)))).then_some(id)
        })
        .collect::<Vec<_>>();
    let mut missing = missing
        .into_iter()
        .filter(|id| !printed_missing.contains(id))
        .collect::<Vec<_>>();
    missing.sort_by_key(|id| id.0);

    if detached.is_empty() && missing.is_empty() {
        return;
    }

    blank_line(out);
    let _ = writeln!(out, "; referenced nodes outside displayed blocks");
    for id in detached {
        let Some(node) = module.node(id) else {
            continue;
        };
        line(out, width, id, 0, &format_node(node));
    }
    for id in missing {
        line(out, width, id, 0, "<missing>");
    }
}

fn reachable(module: &Module, entry: IrNodeId) -> Vec<IrNodeId> {
    let mut order = Vec::new();
    let mut seen = HashSet::new();
    let mut queue = vec![entry];

    while let Some(block) = queue.pop() {
        if !seen.insert(block) {
            continue;
        }

        let Some(IrNode::Label(instructions)) = module.node(block) else {
            continue;
        };

        order.push(block);
        let mut branches = Vec::new();
        let mut merges = Vec::new();
        for instruction in instructions {
            match module.node(*instruction) {
                Some(IrNode::Branch(target)) => branches.push(*target),
                Some(IrNode::ConditionalBranch {
                    true_block,
                    false_block,
                    ..
                }) => branches.extend([*true_block, *false_block]),
                Some(IrNode::TryCatch { body, catch, .. }) => branches.extend([*body, *catch]),
                Some(IrNode::LoopMerge {
                    merge_block,
                    continue_block,
                }) => merges.extend([*continue_block, *merge_block]),
                Some(IrNode::SelectionMerge { merge_block }) => merges.push(*merge_block),
                _ => {},
            }
        }

        let mut next = [branches, merges].concat();
        next.reverse();
        queue.extend(next);
    }

    order
}

fn line(out: &mut String, width: usize, id: IrNodeId, depth: usize, body: &str) {
    let _ = writeln!(out, "{:>width$} = {}{body}", id.to_string(), "  ".repeat(depth));
}

fn blank_line(out: &mut String) {
    if !out.is_empty() && !out.ends_with("\n\n") {
        out.push('\n');
    }
}

fn constant(value: &Value) -> String {
    match value {
        Value::Text(text) => format!("{text:?}"),
        Value::Resource(text) => format!("'{text}'"),
        other => other.to_string(),
    }
}

fn list(ids: &[IrNodeId]) -> String { ids.iter().map(IrNodeId::to_string).collect::<Vec<_>>().join(" ") }

fn arguments(args: &[Argument]) -> String {
    args.iter()
        .map(|arg| match (arg.key, arg.value) {
            (Some(key), Some(value)) => format!("{key}={value}"),
            (None, Some(value)) => value.to_string(),
            (Some(key), None) => format!("{key}=_"),
            (None, None) => String::from("_"),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn format_node(node: &IrNode) -> String {
    match node {
        IrNode::Function(id) => format!("function proc={id}"),
        IrNode::ExternalFunction(name) => format!("external_function {name}"),
        IrNode::Label(_) => String::from("label"),
        IrNode::Constant(value) => format!("constant {}", constant(value)),
        IrNode::FunctionParameter(index) => format!("function_parameter #{index}"),
        IrNode::Phi { operands } => format!(
            "phi {}",
            operands
                .iter()
                .map(|operand| format!("[{} from block {}]", operand.value, operand.block))
                .collect::<Vec<_>>()
                .join(" ")
        ),
        IrNode::Variable(name) => format!("variable {name}"),
        IrNode::Load { pointer } => format!("load {pointer}"),
        IrNode::Builtin(builtin) => format!("builtin {}", snake_case(builtin)),
        IrNode::Interpolate { chunks, values } => {
            format!("interpolate chunks={} values={}", chunks.len(), list(values))
        },
        IrNode::Unary { op, operand } => format!("unary {} {operand}", snake_case(op)),
        IrNode::Binary { op, lhs, rhs } => format!("binary {} {lhs} {rhs}", snake_case(op)),
        IrNode::CompoundBinary { op, lhs, rhs } => {
            format!("compound_binary {} {lhs} {rhs}", snake_case(op))
        },
        IrNode::SetField {
            object,
            name,
            access,
            value,
        } => format!("set_field {object} {}{name} {value}", access_operator(*access)),
        IrNode::SetIndex {
            object,
            index,
            value,
            conditional,
        } => {
            let conditional = if *conditional { " conditional" } else { "" };
            format!("set_index {object} {index} {value}{conditional}")
        },
        IrNode::Store { pointer, value } => format!("store {pointer} {value}"),
        IrNode::Initialize { pointer, value } => format!("initialize {pointer} {value}"),
        IrNode::StoreBuiltin { builtin, value } => {
            format!("store_builtin {} {value}", snake_case(builtin))
        },
        IrNode::CatchValue => String::from("catch_value"),
        IrNode::AccessField { object, name, access } => {
            format!("access_field {object} {}{name}", access_operator(*access))
        },
        IrNode::Initial { object, name } => match object {
            Some(object) => format!("initial {object}.{name}"),
            None => format!("initial {name}"),
        },
        IrNode::Index {
            object,
            index,
            conditional,
        } => match conditional {
            true => format!("index {object} {index} conditional"),
            false => format!("index {object} {index}"),
        },
        IrNode::Call { callee, args } => format!("call {callee} {}", arguments(args)),
        IrNode::FunctionCall { function, args } => format!("function_call {function} {}", arguments(args)),
        IrNode::Super {
            args,
            forwards_extra_args,
        } => {
            let extra = if *forwards_extra_args { " extra_args" } else { "" };
            format!("super {}{extra}", arguments(args))
        },
        IrNode::New { ty, args } => match ty {
            Some(ty) => format!("new ty={ty} {}", arguments(args)),
            None => format!("new {}", arguments(args)),
        },
        IrNode::ModifiedType { path, overrides } => format!(
            "modified_type {path} {}",
            overrides
                .iter()
                .map(|(name, id)| format!("{name}={id}"))
                .collect::<Vec<_>>()
                .join(" ")
        ),
        IrNode::List(args) => format!("list {}", arguments(args)),
        IrNode::Pick(choices) => format!(
            "pick {}",
            choices
                .iter()
                .map(|(weight, value)| match weight {
                    Some(weight) => format!("{weight}:{value}"),
                    None => value.to_string(),
                })
                .collect::<Vec<_>>()
                .join(" ")
        ),
        IrNode::InRange {
            value,
            start,
            end,
            step,
        } => match step {
            Some(step) => format!("in_range {value} {start} {end} step={step}"),
            None => format!("in_range {value} {start} {end}"),
        },
        IrNode::Range { start, end, step } => match step {
            Some(step) => format!("range {start} {end} step={step}"),
            None => format!("range {start} {end}"),
        },
        IrNode::SelectionMerge { merge_block } => format!("selection_merge merge={merge_block}"),
        IrNode::LoopMerge {
            merge_block,
            continue_block,
        } => format!("loop_merge merge={merge_block} continue={continue_block}"),
        IrNode::Branch(target) => format!("branch {target}"),
        IrNode::ConditionalBranch {
            condition,
            true_block,
            false_block,
        } => format!("conditional_branch {condition} {true_block} {false_block}"),
        IrNode::Return(value) => match value {
            Some(value) => format!("return {value}"),
            None => String::from("return"),
        },
        IrNode::IterInit {
            list,
            ty,
            value_is_associated,
        } => {
            let association = if *value_is_associated { " values=associated" } else { "" };
            match ty {
                Some(ty) => format!("iter_init {list} ty={ty}{association}"),
                None => format!("iter_init {list}{association}"),
            }
        },
        IrNode::IterNext(id) => format!("iter_next {id}"),
        IrNode::IterValue(id) => format!("iter_value {id}"),
        IrNode::IterKey(id) => format!("iter_key {id}"),
        IrNode::RangeTest { current, end, step } => format!("range_test {current} end={end} step={step}"),
        IrNode::TryCatch { body, catch, merge } => {
            format!("try_catch body={body} catch={catch} merge={merge}")
        },
        IrNode::Del(id) => format!("del {id}"),
        IrNode::Throw(id) => format!("throw {id}"),
        IrNode::Output { target, value } => match target {
            OutputTarget::Value(target) => format!("output {value} to {target}"),
            OutputTarget::Field { object, name, access } => {
                format!("output {value} to {object}{}{name}", access_operator(*access))
            },
            OutputTarget::Index {
                object,
                index,
                conditional,
            } => {
                let conditional = if *conditional { " conditional" } else { "" };
                format!("output {value} to {object}[{index}]{conditional}")
            },
        },
        IrNode::Noop => String::from("noop"),
        IrNode::Blocked(what) => format!("blocked {what:?}"),
        IrNode::Trap { reason } => format!("trap {reason:?}"),
    }
}

fn snake_case(value: &impl std::fmt::Debug) -> String {
    let name = format!("{value:?}");
    let mut out = String::with_capacity(name.len());
    let mut previous_was_lowercase = false;

    for character in name.chars() {
        if character.is_ascii_uppercase() {
            if previous_was_lowercase {
                out.push('_');
            }
            out.push(character.to_ascii_lowercase());
            previous_was_lowercase = false;
        } else {
            out.push(character);
            previous_was_lowercase = character.is_ascii_lowercase() || character.is_ascii_digit();
        }
    }

    out
}

fn access_operator(access: AccessKind) -> &'static str {
    match access {
        AccessKind::Dot => ".",
        AccessKind::Colon => ":",
        AccessKind::SafeDot => "?.",
        AccessKind::SafeColon => "?:",
        AccessKind::Scope => "::",
    }
}

mod color {
    pub const ID: &str = "\x1b[36m";
    pub const KEY: &str = "\x1b[37m";
    pub const NUMBER: &str = "\x1b[33m";
    pub const OPCODE: &str = "\x1b[1;34m";
    pub const RESET: &str = "\x1b[0m";
    pub const STRING: &str = "\x1b[32m";
    pub const COMMENT: &str = "\x1b[90m";
    pub const VARIABLE: &str = "\x1b[35m";
}

fn highlight(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 2);

    for line in text.lines() {
        highlight_line(&mut out, line);
        out.push('\n');
    }

    out
}

fn highlight_line(out: &mut String, line: &str) {
    if line.trim_start().starts_with(';') {
        out.push_str(color::COMMENT);
        highlight_operands(out, line, color::COMMENT);
        out.push_str(color::RESET);

        return;
    }

    let Some((id, body)) = line.split_once(" = ") else {
        highlight_operands(out, line, "");

        return;
    };

    let padding = id.len() - id.trim_start().len();
    out.push_str(&id[..padding]);
    token(out, color::ID, id.trim_start(), "");
    out.push_str(" = ");

    let indent = body.len() - body.trim_start().len();
    let opcode = body[indent..]
        .find(|c: char| !c.is_alphanumeric() && c != '_')
        .map_or(body.len(), |end| indent + end);
    out.push_str(&body[..indent]);
    token(out, color::OPCODE, &body[indent..opcode], "");
    highlight_operands(out, &body[opcode..], "");
}

fn highlight_operands(out: &mut String, text: &str, base: &str) {
    let key = match base {
        color::COMMENT => color::COMMENT,
        _ => color::KEY,
    };
    let mut rest = text;

    while let Some(head) = rest.chars().next() {
        let taken = match head {
            '"' | '\'' => {
                let end = rest[1..].find(head).map_or(rest.len(), |end| end + 2);
                token(out, color::STRING, &rest[..end], base);

                end
            },
            '%' | '$' => {
                let end = word_end(&rest[1..]) + 1;
                let color = match head {
                    '$' => color::VARIABLE,
                    _ => color::ID,
                };
                token(out, color, &rest[..end], base);

                end
            },
            _ if head.is_ascii_digit() => {
                let end = number_end(rest);
                token(out, color::NUMBER, &rest[..end], base);

                end
            },
            _ if head.is_alphabetic() || head == '_' => {
                let end = word_end(rest);
                match rest[end..].starts_with('=') {
                    true => token(out, key, &rest[..end], base),
                    false => out.push_str(&rest[..end]),
                }

                end
            },
            _ => {
                out.push(head);

                head.len_utf8()
            },
        };
        rest = &rest[taken..];
    }
}

fn word_end(text: &str) -> usize {
    text.find(|c: char| !c.is_alphanumeric() && c != '_')
        .unwrap_or(text.len())
}

fn number_end(text: &str) -> usize {
    let mut end = 0;

    for (index, c) in text.char_indices() {
        if !(c.is_ascii_digit() || c == '.' || c == '-') {
            break;
        }
        end = index + c.len_utf8();
    }

    end
}

fn token(out: &mut String, color: &str, text: &str, base: &str) {
    if color == base {
        out.push_str(text);

        return;
    }

    out.push_str(color);
    out.push_str(text);
    out.push_str(color::RESET);
    out.push_str(base);
}

#[cfg(test)]
mod tests {
    macro_rules! fixture {
        ($path:literal) => {
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/", $path))
        };
    }

    use core::{
        location::Location,
        path::TreePath,
        types::{IrNodeId, ProcId},
    };

    use super::*;
    use crate::{IrModuleBuilder, Procedure};

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

    /// Expression trees are printed above the instruction that reads them, one level deeper.
    #[test]
    fn disassembly_is_stable_and_names_variables() {
        assert_eq!(
            dump(&lower(fixture!(
                "programs/disassembly_is_stable_and_names_variables.dm"
            ))),
            concat!(
                "; ir module: 5 nodes, 1 constants, 0 external functions, 1 functions, 1 blocks\n",
                "\n",
                "; constants\n",
                "%3 = constant 2\n",
                "\n",
                "%0 = function /::test vars=1\n",
                "%1 =   function_parameter #0\n",
                "\n",
                "%2 =   label\n",
                "%4 =     return %1\n",
            )
        );
    }

    /// A loop declares its merge and continue targets immediately before the branch they describe.
    #[test]
    fn a_loop_disassembles_as_blocks_and_branches() {
        assert_eq!(
            dump(&lower(fixture!(
                "programs/a_loop_disassembles_as_blocks_and_branches.dm"
            ))),
            concat!(
                "; ir module: 13 nodes, 1 constants, 0 external functions, 1 functions, 3 blocks\n",
                "\n",
                "; constants\n",
                "%10 = constant 0\n",
                "\n",
                " %0 = function /::t vars=1\n",
                " %1 =   function_parameter #0\n",
                "\n",
                " %2 =   label\n",
                " %6 =     branch %3\n",
                "\n",
                " %3 =   label\n",
                " %7 =     phi [%1 from block %2] [%10 from block %3]\n",
                " %8 =     loop_merge merge=%5 continue=%3\n",
                " %9 =     conditional_branch %7 %3 %5\n",
                "\n",
                " %5 =   label\n",
                "%12 =     return\n",
            )
        );
    }

    #[test]
    fn opcodes_and_enum_operands_are_lower_snake_case() {
        let output = dump(&lower(fixture!(
            "programs/opcodes_and_enum_operands_are_lower_snake_case.dm"
        )));

        assert!(output.contains("builtin src"), "{output}");
        assert!(output.contains("unary neg"), "{output}");
        assert!(output.contains("binary add"), "{output}");
        assert!(
            output
                .lines()
                .any(|line| line.contains("set_field") && line.contains(".value")),
            "{output}"
        );
        assert!(
            output
                .lines()
                .any(|line| line.contains("access_field") && line.contains(".value")),
            "{output}"
        );
    }

    /// Highlighting only wraps the plain dump, so the text is identical once the escapes are gone.
    #[test]
    fn highlighting_changes_nothing_but_colour() {
        let module = lower(fixture!("programs/highlighting_changes_nothing_but_colour.dm"));
        let plain = dump(&module);
        let coloured = dump_with(&module, true);

        assert!(coloured.contains("\x1b["));
        assert_eq!(strip(&coloured), plain);
    }

    #[test]
    fn selected_disassembly_omits_unreachable_procedures_and_constants() {
        let module = lower(fixture!(
            "programs/selected_disassembly_omits_unreachable_procedures_and_constants.dm"
        ));
        let output = dump_selected_with(&module, &HashSet::from([ProcId(0)]), false);

        assert!(output.contains("1 functions"), "{output}");
        assert!(output.contains("::live"), "{output}");
        assert!(output.contains("constant 1"), "{output}");
        assert!(!output.contains("::dead"), "{output}");
        assert!(!output.contains("constant 2"), "{output}");
    }

    #[test]
    fn disassembly_includes_unreachable_blocks_and_their_invalid_references() {
        let mut module = Module {
            nodes: vec![
                IrNode::Function(ProcId(0)),
                IrNode::Label(vec![IrNodeId(2)]),
                IrNode::Return(None),
                IrNode::Label(vec![IrNodeId(4)]),
                IrNode::Return(Some(IrNodeId(5))),
                IrNode::Noop,
            ],
            procs: vec![Procedure {
                function: IrNodeId(0),
                parameters: Vec::new(),
                previous: None,
                owner: TreePath::default(),
                name: "test".into(),
                params: Vec::new(),
                variadic: false,
                vars: Vec::new(),
                body: IrNodeId(1),
                intrinsic: None,
                location: Location::default(),
            }],
            ..Module::default()
        };

        let output = dump(&module);
        assert!(output.contains("2 blocks"), "{output}");
        assert!(output.contains("; unreachable blocks in /::test"), "{output}");
        assert!(output.contains("%4 =     return %5"), "{output}");
        assert!(output.contains("%5 = noop"), "{output}");

        module.nodes[4] = IrNode::Return(Some(IrNodeId(99)));
        let output = dump(&module);
        assert!(output.contains("%99 = <missing>"), "{output}");
    }

    fn strip(text: &str) -> String {
        let mut out = String::new();
        let mut rest = text;

        while let Some(start) = rest.find('\x1b') {
            out.push_str(&rest[..start]);
            let end = rest[start..].find('m').map_or(rest.len(), |end| start + end + 1);
            rest = &rest[end..];
        }
        out.push_str(rest);

        out
    }
    #[test]
    fn probe_dump_file() {
        let Ok(path) = std::env::var("DM_DUMP") else {
            return;
        };
        let source = std::fs::read_to_string(&path).expect("read");
        eprintln!("{}", dump_with(&lower(&source), std::env::var("DM_COLOR").is_ok()));
    }
}
