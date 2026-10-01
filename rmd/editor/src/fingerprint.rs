use core::{
    location::{FileId, Location},
    types::Value,
};

use net::CodebaseHash;
use objtree::ObjectTree;
use ring::digest::{Context, SHA256};

pub(crate) fn object_tree(tree: &ObjectTree, builtin_files: &[FileId]) -> CodebaseHash {
    let is_builtin = |location: &Location| builtin_files.contains(&location.file);
    let mut types = tree
        .iter()
        .filter_map(|decl| {
            let mut vars = decl
                .vars
                .values()
                .filter(|var| !is_builtin(&var.location))
                .collect::<Vec<_>>();
            if is_builtin(&decl.location) && vars.is_empty() {
                return None;
            }

            vars.sort_unstable_by(|a, b| a.name.as_str().cmp(b.name.as_str()));

            Some((decl.path.to_string(), decl, vars))
        })
        .collect::<Vec<_>>();
    types.sort_unstable_by(|a, b| a.0.cmp(&b.0));

    let mut context = Context::new(&SHA256);
    context.update(b"rmd object tree v3\0");
    write_len(&mut context, types.len());
    for (path, decl, vars) in types {
        write_str(&mut context, &path);
        match &decl.parent_type {
            Some(parent) => {
                context.update(&[1]);
                write_str(&mut context, &parent.to_string());
            },
            None => context.update(&[0]),
        }

        write_len(&mut context, vars.len());
        for var in vars {
            write_str(&mut context, var.name.as_str());
            write_value(&mut context, &var.value);
        }
    }

    let mut hash = [0; 32];
    hash.copy_from_slice(context.finish().as_ref());
    CodebaseHash(hash)
}

fn write_value(context: &mut Context, value: &Value) {
    match value {
        Value::Null => context.update(&[0]),
        Value::Num(num) => {
            context.update(&[1]);
            context.update(&num.to_bits().to_le_bytes());
        },
        Value::Text(text) => {
            context.update(&[2]);
            write_str(context, text);
        },
        Value::Resource(path) => {
            context.update(&[3]);
            write_str(context, path);
        },
        Value::Path(path) => {
            context.update(&[4]);
            write_str(context, &path.to_string());
        },
        Value::List(entries) => {
            context.update(&[5]);
            write_len(context, entries.len());
            for entry in entries {
                write_value(context, &entry.key);
                match &entry.value {
                    Some(value) => {
                        context.update(&[1]);
                        write_value(context, value);
                    },
                    None => context.update(&[0]),
                }
            }
        },
        Value::Unevaluated => context.update(&[6]),
    }
}

fn write_str(context: &mut Context, text: &str) {
    write_len(context, text.len());
    context.update(text.as_bytes());
}

fn write_len(context: &mut Context, len: usize) { context.update(&(len as u64).to_le_bytes()); }
