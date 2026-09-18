//! Frontend driver. The editor is a library; this is how the compiler gets exercised without one.
//!
//! ```text
//! rmdc tokens <file.dm>     dump the token stream, layout tokens included
//! rmdc pp     <file.dme>    preprocess and print the flattened source back out
//! rmdc tree   <file.dme>    preprocess, parse and print the object tree
//! rmdc ir     <file.dme>    preprocess, parse and print the IR module
//! rmdc bytecode <file.dme>  compile and print stack bytecode
//! rmdc eval   <file.dm>     compile and execute /proc/main or /world/New
//! rmdc bake   <file.dme> <file.dmm>  bake map appearances through DM
//! rmdc map    <file.dmm>    parse a map and summarise it
//! rmdc roundtrip <file.dmm> parse a map, write it back out and diff the bytes
//! rmdc icon   <file.dmi>    decode an icon and list its states
//! ```

use core::{
    arena::StrArena,
    location::{FileId, Location},
    source::SourceMap,
};
use std::{
    collections::BTreeSet,
    fmt::Write,
    path::{Path, PathBuf},
    process::ExitCode,
};

use dmi::{IconFile, metadata::IconState};
use objtree::{ObjectTree, ProcDecl, TypeId, VarDecl};

fn usage() -> ExitCode {
    eprintln!("usage: rmdc <tokens|pp|tree|ir|bytecode|eval|bake|map|roundtrip|icon> <file>");

    ExitCode::FAILURE
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let (Some(command), Some(path)) = (args.next(), args.next().map(PathBuf::from)) else {
        return usage();
    };

    let result = match command.as_str() {
        "tokens" => dump_tokens(&path),
        "pp" => dump_preprocessed(&path),
        "tree" => dump_tree(&path),
        "ir" => dump_ir(&path),
        "bytecode" => dump_bytecode(&path),
        "eval" => eval(&path),
        "bake" => match args.next() {
            Some(map) => {
                let flags = args.collect::<Vec<_>>();
                if flags.iter().any(|flag| flag != "--summary" && flag != "--check-edit") {
                    return usage();
                }
                bake_map(
                    &path,
                    Path::new(&map),
                    flags.iter().any(|flag| flag == "--summary"),
                    flags.iter().any(|flag| flag == "--check-edit"),
                )
            },
            None => return usage(),
        },
        "map" => dump_map(&path),
        "roundtrip" => roundtrip_map(&path),
        "icon" => dump_icon(&path),
        _ => return usage(),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        },
    }
}

fn dump_tokens(path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let source = std::fs::read_to_string(path)?;
    let (tokens, errors) = lexer::tokenize(&source);

    for (token, location) in &tokens {
        println!("{:>4}:{:<4} {token}", location.begin.line, location.begin.col);
    }

    println!("=== {} tokens, {} errors ===", tokens.len(), errors.len());
    for error in &errors {
        eprintln!("{}: {error}", path.display());
    }

    Ok(())
}

fn dump_preprocessed(path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let arena = StrArena::new();
    let preprocessed = preprocessor::preprocess(&arena, path)?;

    println!("{}", preprocessor::render(&preprocessed.tokens));
    println!(
        "=== {} tokens, {} resources, {} diagnostics ===",
        preprocessed.tokens.len(),
        preprocessed.resources.len(),
        preprocessed.errors.len()
    );

    let source_root = source_root(&preprocessed.sources, preprocessed.entry, path);
    for error in &preprocessed.errors {
        eprintln!(
            "{}",
            error.display(relative_path(&preprocessed.sources, source_root, error.location.file))
        );
    }

    Ok(())
}

fn dump_tree(path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let arena = StrArena::new();

    println!("=== PREPROCESS ===");
    let preprocessed = preprocessor::preprocess(&arena, path)?;
    println!(
        "{} tokens, {} resources, {} diagnostics",
        preprocessed.tokens.len(),
        preprocessed.resources.len(),
        preprocessed.errors.len()
    );

    let source_root = source_root(&preprocessed.sources, preprocessed.entry, path);
    for error in &preprocessed.errors {
        eprintln!(
            "{}",
            error.display(relative_path(&preprocessed.sources, source_root, error.location.file))
        );
    }

    println!("=== PARSE ===");
    let ast = ast::parse(&preprocessed.tokens).map_err(|error| {
        std::io::Error::other(format_parse_error(
            &error,
            &preprocessed.sources,
            preprocessed.entry,
            path,
        ))
    })?;
    println!("{} top level declarations", ast.declarations.len());

    println!("=== TREE ===");
    let (tree, _module, errors) = sema::analyze(&ast);
    print!("{}", render_tree(&tree, &preprocessed.sources, source_root));

    for error in &errors {
        eprintln!(
            "{}",
            error.display(relative_path(&preprocessed.sources, source_root, error.location.file))
        );
    }

    Ok(())
}

fn dump_ir(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let arena = StrArena::new();
    let preprocessed = preprocessor::preprocess(&arena, path)?;
    if !preprocessed.is_ok() {
        return Err("preprocessing failed".into());
    }

    let ast = ast::parse(&preprocessed.tokens)?;
    let (_, module, _) = sema::analyze(&ast);
    print!("{}", ir::disasm::dump_with(&module, false));

    Ok(())
}

fn dump_bytecode(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let arena = StrArena::new();
    let preprocessed = preprocessor::preprocess(&arena, path)?;
    if !preprocessed.is_ok() {
        return Err("preprocessing failed".into());
    }

    let ast = ast::parse(&preprocessed.tokens)?;
    let (_, module, _) = sema::analyze(&ast);
    let module = codegen::generate(&module)?;
    print!("{}", codegen::disasm::dump(&module)?);

    Ok(())
}

fn bake_map(entry: &Path, map_path: &Path, summary: bool, check_edit: bool) -> Result<(), Box<dyn std::error::Error>> {
    let compile_started = std::time::Instant::now();
    let arena = StrArena::new();
    let preprocessed = preprocessor::Preprocessor::new(&arena).with_baking(true).run(entry)?;
    let source_root = source_root(&preprocessed.sources, preprocessed.entry, entry);
    for error in &preprocessed.errors {
        eprintln!(
            "{}",
            error.display(relative_path(&preprocessed.sources, source_root, error.location.file))
        );
    }
    if !preprocessed.is_ok() {
        return Err("preprocessing failed".into());
    }
    let ast = ast::parse(&preprocessed.tokens).map_err(|error| {
        std::io::Error::other(format_parse_error(
            &error,
            &preprocessed.sources,
            preprocessed.entry,
            entry,
        ))
    })?;
    let (tree, ir_module, errors) = sema::analyze(&ast);
    for error in &errors {
        eprintln!(
            "{}",
            error.display(relative_path(&preprocessed.sources, source_root, error.location.file))
        );
    }
    if !errors.is_empty() {
        return Err("semantic analysis failed".into());
    }

    if !vm::bake::has_profile(&tree) {
        return Err(
            "the codebase defines no demir_bake, demir_initialize, demir_prepare or demir_light under #ifdef \
             __DEMIR_BAKE__"
                .into(),
        );
    }

    let module = codegen::generate(&ir_module)?;
    drop(ast);
    eprintln!("compiled in {:.2}s", compile_started.elapsed().as_secs_f32());
    let roots = entry
        .parent()
        .map(Path::to_path_buf)
        .into_iter()
        .chain(preprocessed.resource_dirs.iter().cloned())
        .collect::<Vec<_>>();
    let icons = icon_states(&tree, &module, &roots);

    let source = std::fs::read_to_string(map_path)?;
    let (map, errors) = dmm::parser::parse(&source);
    if !errors.is_empty() {
        return Err(format!("{} map parse errors", errors.len()).into());
    }

    let mut atoms = Vec::new();
    let mut instance = 0u64;
    for z in 1..=map.size.z {
        for y in 1..=map.size.y {
            for x in 1..=map.size.x {
                if let Some(tile) = map.tile_at(dmm::Coord::new(x, y, z)) {
                    for prefab in tile {
                        instance += 1;
                        if let Some(ty) = tree.id_of(&prefab.path) {
                            atoms.push(vm::bake::Atom {
                                instance,
                                ty,
                                position: vm::world::Position::new(x as i32, y as i32, z as i32),
                                vars: prefab
                                    .vars
                                    .iter()
                                    .map(|(name, value)| (name.clone(), value.value.clone()))
                                    .collect::<Vec<_>>(),
                            });
                        }
                    }
                }
            }
        }
    }

    let baseline = atoms
        .iter()
        .map(|atom| {
            let state = atom
                .vars
                .iter()
                .find(|(name, _)| name.as_str() == "icon_state")
                .map(|(_, value)| value.clone())
                .or_else(|| {
                    tree.var_inherited(atom.ty, &"icon_state".into())
                        .map(|variable| variable.value.clone())
                });
            (atom.instance, atom.position, state)
        })
        .collect::<Vec<_>>();
    let edit_atom = check_edit
        .then(|| {
            atoms
                .iter()
                .find(|atom| {
                    tree.get(atom.ty)
                        .is_some_and(|ty| ty.path.to_string().contains("/wall"))
                })
                .cloned()
        })
        .flatten();

    let bake_started = std::time::Instant::now();
    let mut bake = vm::bake::Bake::new(
        &tree,
        &module,
        atoms,
        [map.size.x as i32, map.size.y as i32, map.size.z as i32],
        vm::Limits::default(),
        icons,
    );
    let bake_seconds = bake_started.elapsed().as_secs_f64();
    report_bake_output(&mut bake);

    let mut changed = 0;
    let mut connected = 0;
    let mut links = 0;
    for (id, position, fallback) in baseline {
        let incident = bake.connections(id).len();
        if incident > 0 {
            connected += 1;
            links += incident;
        }

        let baked = bake.appearances.get(&id).and_then(|delta| {
            delta
                .vars
                .iter()
                .find(|(name, _)| name.as_str() == "icon_state")
                .map(|(_, value)| value.clone())
        });
        if baked.is_some()
            && baked.as_ref().unwrap_or(&core::types::Value::Null)
                != fallback.as_ref().unwrap_or(&core::types::Value::Null)
        {
            changed += 1;
        }
        if !summary {
            let state = baked
                .or(fallback)
                .map(|value| value.to_string())
                .unwrap_or_else(|| "null".into());
            println!("{},{},{} #{id} {state}", position.x, position.y, position.z);
        }
    }

    eprintln!(
        "baked {}/{} attempted, {} placed atoms, {} faults in {:.3}s",
        bake.succeeded,
        bake.attempted,
        instance,
        bake.diagnostics.count(),
        bake_seconds
    );
    eprintln!("{changed} icon states changed, {} cache hits", bake.cache_hits);
    eprintln!("{connected} connected placements, {links} incident connections");
    if let Some(lighting) = &bake.lighting {
        let lit = lighting
            .tiles
            .iter()
            .filter(|tile| tile.corners.iter().flatten().any(|channel| *channel > 0.0))
            .count();
        let mut roles = [0usize; 3];
        for appearance in bake.appearances.values() {
            count_lighting_roles(appearance, &mut roles);
        }
        eprintln!(
            "{lit}/{} tiles lit, {} overlay lights, {} emissive and {} blocker sprites",
            lighting.tiles.len(),
            roles[0],
            roles[1],
            roles[2]
        );
    }
    for diagnostic in bake
        .diagnostics
        .entries
        .iter()
        .filter(|diagnostic| diagnostic.count > 0)
    {
        let fault = &diagnostic.fault;
        let name = fault
            .proc
            .and_then(|proc| module.function_for_proc(proc))
            .and_then(|function| module.strings.get(function.name.0 as usize))
            .map(String::as_str)
            .unwrap_or_default();
        eprintln!(
            "{}x {name} {:?} at {}",
            diagnostic.count,
            fault.kind,
            fault.location.display(preprocessed.sources.path(fault.location.file))
        );
        if let Some(offset) = fault.offset {
            eprintln!("  bytecode {offset}");
        }
    }

    if let Some(atom) = edit_atom {
        let before = bake.appearances.clone();
        let lighting_before = bake.lighting.clone();
        let started = std::time::Instant::now();
        let affected = bake.update(&tree, &module, Vec::new(), &[atom.instance]);
        report_bake_output(&mut bake);
        let remove_us = started.elapsed().as_micros();
        let started = std::time::Instant::now();
        bake.update(&tree, &module, vec![atom], &[]);
        report_bake_output(&mut bake);
        let restore_us = started.elapsed().as_micros();
        if bake.appearances != before {
            return Err("remove/restore changed the derived appearance layer".into());
        }
        if bake.lighting != lighting_before {
            return Err("remove/restore changed the derived lighting layer".into());
        }
        eprintln!(
            "edit check: {} affected instances, remove {remove_us} us, restore {restore_us} us, appearances restored",
            affected.appearances.len()
        );
    }

    Ok(())
}

/// `'icons/obj/doors.dmi'` from any var initializer or proc constant, with its state names read off disk.
fn icon_states(tree: &ObjectTree, module: &codegen::Module, roots: &[PathBuf]) -> vm::IconStates {
    let names = tree
        .iter()
        .flat_map(|decl| decl.vars.values())
        .map(|var| &var.value)
        .chain(&module.constants)
        .filter_map(|value| match value {
            core::types::Value::Resource(path) if path.to_ascii_lowercase().ends_with(".dmi") => Some(path.as_str()),
            _ => None,
        })
        .collect::<BTreeSet<_>>();

    vm::IconStates::new(names.into_iter().filter_map(|name| {
        let relative = name.replace('\\', "/");
        let path = roots
            .iter()
            .map(|root| root.join(&relative))
            .find(|path| path.is_file())?;
        let metadata = IconFile::load_metadata(path).ok()?;

        Some((
            name.to_owned(),
            metadata.states.into_iter().map(|state| state.name).collect(),
        ))
    }))
}

fn count_lighting_roles(appearance: &vm::AppearanceDelta, roles: &mut [usize; 3]) {
    match appearance.lighting {
        vm::AppearanceLighting::OverlayLight | vm::AppearanceLighting::OverlayLightSubtract => roles[0] += 1,
        vm::AppearanceLighting::Emissive => roles[1] += 1,
        vm::AppearanceLighting::Blocker => roles[2] += 1,
        vm::AppearanceLighting::Normal => {},
    }
    for layer in appearance.overlays.iter().chain(&appearance.underlays) {
        count_lighting_roles(layer, roles);
    }
}

fn report_bake_output(bake: &mut vm::bake::Bake) {
    for line in bake.take_output() {
        eprintln!("DM: {line}");
    }
}

fn eval(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    for line in evaluate_file(path)? {
        println!("{line}");
    }

    Ok(())
}

fn evaluate_file(path: &Path) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let arena = StrArena::new();
    let preprocessed = preprocessor::preprocess(&arena, path)?;
    let source_root = source_root(&preprocessed.sources, preprocessed.entry, path);
    for error in &preprocessed.errors {
        eprintln!(
            "{}",
            error.display(relative_path(&preprocessed.sources, source_root, error.location.file))
        );
    }
    if !preprocessed.is_ok() {
        return Err("preprocessing failed".into());
    }

    let ast = ast::parse(&preprocessed.tokens).map_err(|error| {
        std::io::Error::other(format_parse_error(
            &error,
            &preprocessed.sources,
            preprocessed.entry,
            path,
        ))
    })?;
    let (tree, module, errors) = sema::analyze(&ast);
    for error in &errors {
        eprintln!(
            "{}",
            error.display(relative_path(&preprocessed.sources, source_root, error.location.file))
        );
    }
    if !errors.is_empty() {
        return Err("semantic analysis failed".into());
    }

    let main = tree
        .proc_inherited(TypeId::ROOT, &"main".into())
        .and_then(|proc| proc.body);
    let world_new = tree
        .id_of(&core::path::TreePath::parse("/world"))
        .and_then(|world| tree.proc_inherited(world, &"New".into()))
        .and_then(|proc| proc.body);
    let module = codegen::generate(&module)?;
    let mut runtime = vm::Runtime::default();
    let result = if let Some(main) = main {
        runtime.run(&tree, &module, main, None, Vec::new(), vm::Limits::default())
    } else if let Some(world_new) = world_new {
        runtime.run_world(&tree, &module, world_new, Vec::new(), vm::Limits::default())
    } else {
        return Err(std::io::Error::other("missing /proc/main and /world/New").into());
    };
    result.map_err(|fault| {
        std::io::Error::other(format!(
            "runtime fault at {}: {:?}",
            format_location(&preprocessed.sources, source_root, fault.location),
            fault.kind
        ))
    })?;

    Ok(runtime.take_output())
}

fn render_tree(tree: &ObjectTree, sources: &SourceMap<'_>, source_root: &Path) -> String {
    let mut output = String::new();
    render_type(&mut output, tree, sources, source_root, TypeId::ROOT, 0);

    output
}

fn source_root<'a>(sources: &'a SourceMap<'_>, entry: Option<FileId>, path: &'a Path) -> &'a Path {
    // The preprocessor normalizes loaded paths to absolute paths. Derive the root from its entry
    // source so a relative CLI argument still produces entry-relative locations.
    entry
        .and_then(|entry| sources.path(entry))
        .and_then(Path::parent)
        .or_else(|| path.parent())
        .unwrap_or_else(|| Path::new(""))
}

fn format_parse_error(
    error: &ast::error::ParseError, sources: &SourceMap<'_>, entry: Option<FileId>, path: &Path,
) -> String {
    let location = format_location(sources, source_root(sources, entry, path), error.location);

    format!("Parser error at {location}: {}", error.kind)
}

fn render_type(
    output: &mut String, tree: &ObjectTree, sources: &SourceMap<'_>, source_root: &Path, id: TypeId, depth: usize,
) {
    if let Some(decl) = tree.get(id) {
        let indent = "  ".repeat(depth);
        let name = decl.path.name().map(|n| n.as_str()).unwrap_or("/");
        let location = if id == TypeId::ROOT {
            "<built-in>".to_string()
        } else {
            format_location(sources, source_root, decl.location)
        };
        let _ = writeln!(
            output,
            "{indent}{name} ({} vars, {} procs) @ {location}",
            decl.vars.len(),
            decl.procs.len()
        );

        if !decl.vars.is_empty() {
            let _ = writeln!(output, "{indent}  variables:");

            let mut vars = decl.vars.values().collect::<Vec<_>>();
            vars.sort_by(|left, right| left.name.as_str().cmp(right.name.as_str()));
            for var in vars {
                let location = format_location(sources, source_root, var.location);
                let _ = writeln!(output, "{indent}    {} @ {location}", format_var(var));
            }
        }

        if !decl.procs.is_empty() {
            let _ = writeln!(output, "{indent}  procedures:");

            let mut procs = decl.procs.values().collect::<Vec<_>>();
            procs.sort_by(|left, right| left.name.as_str().cmp(right.name.as_str()));
            for proc in procs {
                let location = format_location(sources, source_root, proc.location);
                let _ = writeln!(output, "{indent}    {} @ {location}", format_proc(proc));
            }
        }

        let mut children = decl.children.clone();
        children.sort_by(|left, right| {
            let left = tree
                .get(*left)
                .and_then(|child| child.path.name())
                .map(|name| name.as_str());
            let right = tree
                .get(*right)
                .and_then(|child| child.path.name())
                .map(|name| name.as_str());
            left.cmp(&right)
        });
        for child in children {
            render_type(output, tree, sources, source_root, child, depth + 1);
        }
    }
}

fn format_var(var: &VarDecl) -> String {
    let mut output = String::from("var");
    for (enabled, modifier) in [
        (var.modifiers.is_global, "global"),
        (var.modifiers.is_static, "static"),
        (var.modifiers.is_const, "const"),
        (var.modifiers.is_final, "final"),
        (var.modifiers.is_tmp, "tmp"),
    ] {
        if enabled {
            let _ = write!(output, "/{modifier}");
        }
    }
    if let Some(declared_type) = &var.declared_type {
        let _ = write!(output, "{declared_type}");
    }
    let _ = write!(output, "/{} = {}", var.name, var.value);

    output
}

fn format_proc(proc: &ProcDecl) -> String {
    let mut output = if proc.kind == core::types::ProcKind::Verb {
        format!("verb/{}(", proc.name)
    } else {
        format!("proc/{}(", proc.name)
    };
    for (index, param) in proc.params.iter().enumerate() {
        if index != 0 {
            output.push_str(", ");
        }
        output.push_str(param.spec.name.as_str());
    }
    output.push(')');

    output
}

fn format_location(sources: &SourceMap<'_>, source_root: &Path, location: Location) -> String {
    location
        .display(relative_path(sources, source_root, location.file))
        .to_string()
}

fn relative_path<'a>(sources: &'a SourceMap<'_>, source_root: &Path, file: FileId) -> Option<&'a Path> {
    let path = sources.path(file)?;

    Some(path.strip_prefix(source_root).unwrap_or(path))
}

fn dump_map(path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let source = std::fs::read_to_string(path)?;
    let (map, errors) = dmm::parser::parse(&source);

    for error in &errors {
        eprintln!("{}: {error}", path.display());
    }

    println!(
        "{}x{}x{}, {} distinct tiles, key length {}, format {:?}",
        map.size.x,
        map.size.y,
        map.size.z,
        map.dictionary.len(),
        map.key_length,
        map.format
    );

    if errors.is_empty() {
        return Ok(());
    }

    Err(format!("{} error(s)", errors.len()).into())
}

/// Byte-exact round trip is the only useful test of the writer: SS13 downstreams review map diffs by
/// hand, so a save that reformats a file nobody edited is a bug even when it parses back identically.
fn roundtrip_map(path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let source = std::fs::read_to_string(path)?;
    let (map, errors) = dmm::parser::parse(&source);

    for error in &errors {
        eprintln!("{}: {error}", path.display());
    }

    let written = dmm::writer::write(&map);
    if written == source && errors.is_empty() {
        return Ok(());
    }

    if let Some((line, (before, after))) = source
        .lines()
        .zip(written.lines())
        .enumerate()
        .find(|(_, (before, after))| before != after)
    {
        eprintln!("{}:{}: -{before}", path.display(), line + 1);
        eprintln!("{}:{}: +{after}", path.display(), line + 1);
    }

    Err(format!("{} does not round trip", path.display()).into())
}

fn dump_icon(path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let icon = IconFile::load(path)?;
    print!("{}", render_icon(&icon));

    Ok(())
}

/// Kept separate from the `IconFile::load` so the formatting is testable without a `.dmi` on disk.
fn render_icon(icon: &IconFile) -> String {
    let metadata = &icon.metadata;
    let sprites: usize = metadata.states.iter().map(IconState::sprite_count).sum();
    let cells = icon.cell_count();
    let mut out = String::new();

    let _ = writeln!(
        out,
        "version {}, {}x{}, sheet {}x{} ({cells} cells), {} states, {sprites} sprites",
        metadata.version,
        metadata.width,
        metadata.height,
        icon.sheet_width,
        icon.sheet_height,
        metadata.states.len()
    );

    if cells < sprites || metadata.width == 0 || !icon.sheet_width.is_multiple_of(metadata.width) {
        let _ = writeln!(
            out,
            "warning: sheet does not hold {sprites} {}x{} sprites",
            metadata.width, metadata.height
        );
    }

    for state in &metadata.states {
        let name = format!("\"{}\"", state.name);
        let _ = write!(out, "  {name:<28} dirs {}  frames {}", state.dirs, state.frames);

        if state.is_animated() {
            let delays = state.delays.iter().map(|d| format!("{d}")).collect::<Vec<String>>();
            let _ = write!(out, "  delay {}", delays.join(","));
            let _ = match state.loop_count {
                0 => write!(out, "  loop forever"),
                n => write!(out, "  loop {n}"),
            };
        }

        if state.rewind {
            let _ = write!(out, "  rewind");
        }

        if state.movement {
            let _ = write!(out, "  movement");
        }

        if let Some((x, y, frame)) = state.hotspot {
            let _ = write!(out, "  hotspot {x},{y},{frame}");
        }

        let _ = writeln!(out);
    }

    out
}

#[cfg(test)]
mod tests {
    use core::{
        arena::StrArena,
        location::{FileId, Location, Position},
        source::SourceMap,
    };
    use std::path::{Path, PathBuf};

    use dmi::{
        IconFile,
        metadata::{IconState, Metadata},
    };

    use super::{evaluate_file, format_location, format_parse_error, render_icon, render_tree, source_root};

    #[test]
    fn eval_executes_main_and_prints_world_log() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/hello_world.dm");

        assert_eq!(
            evaluate_file(&path).expect("evaluate hello world"),
            vec![String::from("55")]
        );
    }

    #[test]
    fn eval_falls_back_to_world_new_with_world_as_src() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/eval_world_new.dm");

        assert_eq!(
            evaluate_file(&path).expect("evaluate world.New"),
            vec![String::from("booted")]
        );
    }

    #[test]
    fn icon_dump_lists_states_with_animation_details() {
        let metadata = Metadata {
            version: String::from("4.0"),
            width: 32,
            height: 32,
            states: vec![
                IconState {
                    name: String::from("chair"),
                    dirs: 4,
                    frames: 1,
                    offset: 0,
                    ..Default::default()
                },
                IconState {
                    name: String::from("fire"),
                    dirs: 1,
                    frames: 3,
                    delays: vec![2.0, 2.0, 1.5],
                    rewind: true,
                    hotspot: Some((1, 2, 1)),
                    offset: 4,
                    ..Default::default()
                },
            ],
        };
        let icon = IconFile {
            path: PathBuf::from("icons/obj/chairs.dmi"),
            metadata,
            sheet_width: 96,
            sheet_height: 96,
            pixels: Vec::new(),
        };

        assert_eq!(
            render_icon(&icon),
            concat!(
                "version 4.0, 32x32, sheet 96x96 (9 cells), 2 states, 7 sprites\n",
                "  \"chair\"                      dirs 4  frames 1\n",
                "  \"fire\"                       dirs 1  frames 3  delay 2,2,1.5  loop forever  rewind  hotspot \
                 1,2,1\n",
            )
        );
    }

    /// A sheet too small for the states it declares means the layout assumption is wrong.
    #[test]
    fn icon_dump_warns_when_the_sheet_is_too_small() {
        let icon = IconFile {
            path: PathBuf::from("icons/obj/chairs.dmi"),
            metadata: Metadata {
                version: String::from("4.0"),
                width: 32,
                height: 32,
                states: vec![IconState {
                    name: String::from("chair"),
                    dirs: 4,
                    frames: 1,
                    ..Default::default()
                }],
            },
            sheet_width: 32,
            sheet_height: 32,
            pixels: Vec::new(),
        };

        assert!(render_icon(&icon).contains("warning: sheet does not hold 4 32x32 sprites"));
    }

    #[test]
    fn tree_dump_lists_members_in_dm_syntax_and_stable_order() {
        let arena = StrArena::new();
        let source = concat!(
            "/obj/item\n",
            "\tvar/global/static/const/final/tmp/damage = 2\n",
            "\tvar/mob/living/owner = null\n",
            "\tname = \"item\"\n",
            "\tproc/use(target)\n",
            "/mob\n",
            "\tverb/look(user)\n",
        );
        let mut sources = SourceMap::new();
        let entry = sources.add(&arena, "/project/game/entry.dm", source.to_string());
        let (tokens, errors) = lexer::tokenize(source);
        assert!(errors.is_empty());
        let ast = ast::parse(&tokens).expect("fixture should parse");
        let (tree, _module, errors) = sema::analyze(&ast);
        assert!(errors.is_empty());

        let source_root = source_root(&sources, Some(entry), Path::new("/project/game/entry.dm"));
        let output = render_tree(&tree, &sources, source_root);
        assert_eq!(
            output,
            concat!(
                "/ (0 vars, 0 procs) @ <built-in>\n",
                "  mob (0 vars, 1 procs) @ entry.dm:6:1\n",
                "    procedures:\n",
                "      verb/look(user) @ entry.dm:7:2\n",
                "  obj (0 vars, 0 procs) @ entry.dm:1:1\n",
                "    item (3 vars, 1 procs) @ entry.dm:1:1\n",
                "      variables:\n",
                "        var/global/static/const/final/tmp/damage = 2 @ entry.dm:2:2\n",
                "        var/name = \"item\" @ entry.dm:4:2\n",
                "        var/mob/living/owner = null @ entry.dm:3:2\n",
                "      procedures:\n",
                "        proc/use(target) @ entry.dm:5:2\n",
            )
        );
    }

    #[test]
    fn locations_are_entry_relative_and_handle_unknown_files() {
        let arena = StrArena::new();
        let mut sources = SourceMap::new();
        sources.add(&arena, "/project/game/entry.dme", String::new());
        let included = sources.add(&arena, "/project/game/code/items.dm", String::new());
        let location = Location::in_file(included, Position::new(4, 7), Position::new(4, 8));

        assert_eq!(
            format_location(&sources, Path::new("/project/game"), location),
            "code/items.dm:4:7"
        );

        let missing = Location::in_file(FileId(42), Position::new(9, 3), Position::new(9, 4));
        assert_eq!(
            format_location(&sources, Path::new("/project/game"), missing),
            "<file 42>:9:3"
        );
    }

    #[test]
    fn parse_errors_include_the_source_file() {
        let arena = StrArena::new();
        let mut sources = SourceMap::new();
        let entry = sources.add(&arena, "/project/game/tgstation.dme", String::new());
        let included = sources.add(&arena, "/project/game/code/items.dm", String::new());
        let location = Location::in_file(included, Position::new(22, 1), Position::new(22, 2));
        let error = ast::error::ParseError::expected_path(location);

        assert_eq!(
            format_parse_error(&error, &sources, Some(entry), Path::new("/project/game/tgstation.dme")),
            "Parser error at code/items.dm:22:1: expected a type path"
        );
    }
}
