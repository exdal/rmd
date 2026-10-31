use std::{
    collections::HashSet,
    env, fs,
    path::{Path, PathBuf},
};

#[derive(Debug)]
struct Declaration {
    name: String,
    file: PathBuf,
    line: usize,
}

fn main() {
    println!("cargo:rerun-if-changed=core.dm");
    println!("cargo:rerun-if-changed=stddef_ext.dm");
    println!("cargo:rerun-if-changed=demir.dm");
    println!("cargo:rerun-if-changed=imgui.dm");
    println!("cargo:rerun-if-changed=BYOND_VERSION");

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    fs::write(out_dir.join("version.dm"), version_defines(Path::new("BYOND_VERSION")))
        .expect("write generated version defines");

    let sources = [
        Path::new("core.dm"),
        Path::new("stddef_ext.dm"),
        Path::new("demir.dm"),
        Path::new("imgui.dm"),
    ];
    let mut declarations = Vec::new();
    for source in sources {
        declarations.extend(read_declarations(source));
    }

    validate(&declarations);

    let mut generated = String::from("#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]\npub enum Intrinsic {\n");
    for declaration in &declarations {
        generated.push_str(&format!("    {},\n", declaration.name));
    }
    generated.push_str(
        "}\n\nimpl std::fmt::Display for Intrinsic {\n\tfn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> \
         std::fmt::Result {\n\t\twrite!(formatter, \"{self:?}\")\n\t}\n}\n",
    );

    let mut from_str = String::from(
        "impl std::str::FromStr for Intrinsic {\n    type Err = String;\n\n    fn from_str(text: &str) -> \
         Result<Self, Self::Err> {\n        match text {\n",
    );
    for declaration in &declarations {
        from_str.push_str(&format!(
            "            {:?} => Ok(Self::{}),\n",
            declaration.name, declaration.name
        ));
    }
    from_str.push_str("            unknown => Err(unknown.to_owned()),\n        }\n    }\n}\n");
    generated.push_str(&from_str);

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

        if let Some(name) = intrinsic_name(trimmed, path, line_number) {
            let Some((_, _, proc_line)) = pending_proc.take() else {
                panic!(
                    "{}:{line_number}: intrinsic marker is not attached to a procedure",
                    path.display()
                );
            };
            declarations.push(Declaration {
                name,
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

fn intrinsic_name(line: &str, path: &Path, line_number: usize) -> Option<String> {
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
    let quoted = value.trim();
    let Some(name) = quoted.strip_prefix('"').and_then(|name| name.strip_suffix('"')) else {
        panic!(
            "{}:{line_number}: intrinsic name must be a text literal",
            path.display()
        );
    };

    if !is_identifier(name) {
        panic!(
            "{}:{line_number}: intrinsic name '{name}' is not a valid Rust identifier",
            path.display()
        );
    }

    Some(name.to_owned())
}

fn is_identifier(name: &str) -> bool {
    let mut characters = name.chars();
    match characters.next() {
        Some(first) if first.is_ascii_alphabetic() => characters.all(|character| character.is_ascii_alphanumeric()),
        _ => false,
    }
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

fn validate(declarations: &[Declaration]) {
    let mut names = HashSet::new();
    for declaration in declarations {
        if !names.insert(declaration.name.clone()) {
            panic!(
                "{}:{} declares intrinsic variant {} more than once",
                declaration.file.display(),
                declaration.line,
                declaration.name
            );
        }
    }
}
