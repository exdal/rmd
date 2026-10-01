use core::{
    location::{FileId, Location},
    types::Value,
};

use net::CodebaseHash;
use objtree::ObjectTree;
use ring::digest::{Context, SHA256};

#[repr(u8)]
enum ValueTag {
    Null,
    Num,
    Text,
    Resource,
    Path,
    List,
    Unevaluated,
}

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
    write_len(&mut context, types.len());
    for (path, decl, vars) in types {
        write_str(&mut context, &path);
        write_option(&mut context, decl.parent_type.as_ref(), |context, parent| {
            write_str(context, &parent.to_string());
        });

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
        Value::Null => write_tag(context, ValueTag::Null),
        Value::Num(num) => {
            write_tag(context, ValueTag::Num);
            context.update(&num.to_bits().to_le_bytes());
        },
        Value::Text(text) => {
            write_tag(context, ValueTag::Text);
            write_str(context, text);
        },
        Value::Resource(path) => {
            write_tag(context, ValueTag::Resource);
            write_str(context, path);
        },
        Value::Path(path) => {
            write_tag(context, ValueTag::Path);
            write_str(context, &path.to_string());
        },
        Value::List(entries) => {
            write_tag(context, ValueTag::List);
            write_len(context, entries.len());
            for entry in entries {
                write_value(context, &entry.key);
                write_option(context, entry.value.as_ref(), write_value);
            }
        },
        Value::Unevaluated => write_tag(context, ValueTag::Unevaluated),
    }
}

fn write_tag(context: &mut Context, tag: ValueTag) { context.update(&[tag as u8]); }

fn write_option<T>(context: &mut Context, value: Option<T>, write: impl FnOnce(&mut Context, T)) {
    context.update(&[u8::from(value.is_some())]);
    if let Some(value) = value {
        write(context, value);
    }
}

fn write_str(context: &mut Context, text: &str) {
    write_len(context, text.len());
    context.update(text.as_bytes());
}

fn write_len(context: &mut Context, len: usize) { context.update(&(len as u64).to_le_bytes()); }
