use std::{
    collections::{HashMap, HashSet},
    env,
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug)]
struct Declaration {
    variant: String,
    id: u16,
    file: PathBuf,
    line: usize,
}

fn main() {
    println!("cargo:rerun-if-changed=core.dm");
    println!("cargo:rerun-if-changed=demir.dm");
    println!("cargo:rerun-if-changed=BYOND_VERSION");

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    fs::write(out_dir.join("version.dm"), version_defines(Path::new("BYOND_VERSION")))
        .expect("write generated version defines");

    let sources = [Path::new("core.dm"), Path::new("demir.dm")];
    let mut declarations = Vec::new();
    for source in sources {
        declarations.extend(read_declarations(source));
    }

    validate(&declarations);

    let mut generated = String::from(
        "#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, num_enum::TryFromPrimitive)]\n#[repr(u16)]\npub enum \
         Intrinsic {\n",
    );
    for declaration in &declarations {
        generated.push_str(&format!("    {} = {},\n", declaration.variant, declaration.id));
    }
    generated.push_str(
        "}\n\nimpl std::fmt::Display for Intrinsic {\n\tfn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> \
         std::fmt::Result {\n\t\twrite!(formatter, \"{self:?}\")\n\t}\n}\n",
    );

    fs::write(out_dir.join("intrinsic.rs"), generated).expect("write generated intrinsic enum");
}

fn version_defines(path: &Path) -> String {
    let source = fs::read_to_string(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    let Some((version, build)) = source
        .trim()
        .split_once('.')
        .filter(|(version, build)| digits(version) && digits(build))
    else {
        panic!(
            "{}: expected `version.build`, found {:?}",
            path.display(),
            source.trim()
        );
    };

    format!("#define DM_VERSION {version}\n#define DM_BUILD {build}\n")
}

fn read_declarations(path: &Path) -> Vec<Declaration> {
    let source = fs::read_to_string(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let mut owner = String::new();
    let mut pending_proc: Option<(String, String, usize)> = None;
    let mut declarations = Vec::new();

    for (index, raw) in source.lines().enumerate() {
        let line_number = index + 1;
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }

        let indent = raw.chars().take_while(|character| *character == '\t').count();
        if indent == 0 && trimmed.starts_with('/') && !trimmed.starts_with("/proc/") && !trimmed.contains('(') {
            owner = trimmed.trim_start_matches('/').to_owned();
            pending_proc = None;
            continue;
        }

        if let Some(id) = intrinsic_id(trimmed, path, line_number) {
            let Some((proc_owner, proc_name, proc_line)) = pending_proc.take() else {
                panic!(
                    "{}:{line_number}: intrinsic marker is not attached to a procedure",
                    path.display()
                );
            };
            let variant = variant_name(&proc_owner, &proc_name);
            declarations.push(Declaration {
                variant,
                id,
                file: path.to_path_buf(),
                line: proc_line,
            });
            continue;
        }

        pending_proc =
            procedure_name(trimmed, indent, &owner).map(|(proc_owner, name)| (proc_owner, name, line_number));
    }

    declarations
}

fn intrinsic_id(line: &str, path: &Path, line_number: usize) -> Option<u16> {
    let after_set = line.strip_prefix("set")?;
    if !after_set.starts_with(char::is_whitespace) {
        return None;
    }

    let after_name = after_set.trim_start().strip_prefix("__demir_intrin")?;
    if after_name.starts_with(|character: char| character.is_ascii_alphanumeric() || character == '_') {
        return None;
    }

    let value = after_name
        .trim_start()
        .strip_prefix('=')
        .unwrap_or_else(|| panic!("{}:{line_number}: malformed intrinsic marker", path.display()));
    value
        .trim()
        .parse::<u16>()
        .map(Some)
        .unwrap_or_else(|_| panic!("{}:{line_number}: intrinsic id must be a u16 integer", path.display()))
}

fn procedure_name(line: &str, indent: usize, owner: &str) -> Option<(String, String)> {
    let signature = line.split_once('(')?.0.trim();
    if let Some(name) = signature.strip_prefix("/proc/") {
        return Some((String::new(), name.trim_matches('/').to_owned()));
    }
    if let Some(name) = signature.strip_prefix("proc/") {
        return Some((owner.to_owned(), name.trim_matches('/').to_owned()));
    }
    if indent == 1 && !owner.is_empty() && !signature.contains(char::is_whitespace) {
        return Some((owner.to_owned(), signature.to_owned()));
    }

    None
}

fn variant_name(owner: &str, proc_name: &str) -> String {
    let owner = owner.rsplit('/').next().unwrap_or_default();
    let mut variant = pascal_case(owner);
    variant.push_str(&pascal_case(proc_name));
    if variant.is_empty() || variant.starts_with(|character: char| character.is_ascii_digit()) {
        panic!("cannot derive a Rust intrinsic variant from {owner}/{proc_name}");
    }
    match variant.as_str() {
        "WorldFile2list" => String::from("WorldFile2List"),
        "DmIconGetpixel" => String::from("DmIconGetPixel"),
        _ => variant,
    }
}

fn pascal_case(value: &str) -> String {
    let mut output = String::new();
    for piece in value.split(|character: char| !character.is_ascii_alphanumeric()) {
        if piece.is_empty() {
            continue;
        }
        push_word(&mut output, piece);
    }
    output
}

fn push_word(output: &mut String, word: &str) {
    let normalized = if word.chars().all(|character| !character.is_ascii_lowercase()) {
        word.to_ascii_lowercase()
    } else {
        word.to_owned()
    };
    let mut characters = normalized.chars();
    if let Some(first) = characters.next() {
        output.extend(first.to_uppercase());
        output.extend(characters);
    }
}

fn validate(declarations: &[Declaration]) {
    let mut ids = HashMap::new();
    let mut variants = HashSet::new();
    for declaration in declarations {
        if let Some(previous) = ids.insert(declaration.id, declaration) {
            panic!(
                "{}:{} and {}:{} both declare intrinsic {}",
                previous.file.display(),
                previous.line,
                declaration.file.display(),
                declaration.line,
                declaration.id
            );
        }
        if !variants.insert(declaration.variant.clone()) {
            panic!(
                "{}:{} generates duplicate intrinsic variant {}",
                declaration.file.display(),
                declaration.line,
                declaration.variant
            );
        }
    }
}
