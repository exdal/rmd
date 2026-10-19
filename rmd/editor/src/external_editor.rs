use std::{
    io,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceLocation {
    pub path: PathBuf,
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EditorCommand {
    program: String,
    arguments: Vec<String>,
}

pub(crate) fn open(template: &str, source: &SourceLocation, root: Option<&Path>) -> Result<(), String> {
    let command = editor_command(template, source, root)?;

    match spawn(&command.program, &command.arguments) {
        Ok(()) => Ok(()),
        #[cfg(target_os = "windows")]
        Err(error) if error.kind() == io::ErrorKind::NotFound && Path::new(&command.program).extension().is_none() => {
            let program = format!("{}.cmd", command.program);
            spawn(&program, &command.arguments)
                .map_err(|fallback| format!("could not launch '{}' or '{}': {fallback}", command.program, program))
        },
        Err(error) => Err(format!("could not launch '{}': {error}", command.program)),
    }
}

fn spawn(program: &str, arguments: &[String]) -> io::Result<()> {
    editor::process::command(program).args(arguments).spawn().map(|_| ())
}

fn editor_command(template: &str, source: &SourceLocation, root: Option<&Path>) -> Result<EditorCommand, String> {
    let Some(parts) = shlex::split(template) else {
        return Err(String::from("preferred editor command has invalid quoting"));
    };
    if parts.is_empty() {
        return Err(String::from("preferred editor command is empty"));
    }
    if !parts.iter().any(|part| part.contains("{file}")) {
        return Err(String::from("preferred editor command must contain {file}"));
    }
    if root.is_none() && parts.iter().any(|part| part.contains("{root}")) {
        return Err(String::from(
            "preferred editor command uses {root}, but no environment is loaded",
        ));
    }

    let root = root.map(Path::to_string_lossy).unwrap_or_default();
    let mut expanded = parts.into_iter().map(|part| expand(&part, source, &root));
    let program = expanded.next().expect("non-empty command checked above");
    let arguments = expanded.collect::<Vec<_>>();

    Ok(EditorCommand { program, arguments })
}

fn expand(part: &str, source: &SourceLocation, root: &str) -> String {
    part.replace("{line}", &source.line.to_string())
        .replace("{column}", &source.column.to_string())
        .replace("{root}", root)
        .replace("{file}", &source.path.to_string_lossy())
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{EditorCommand, SourceLocation, editor_command};

    fn source() -> SourceLocation {
        SourceLocation {
            path: PathBuf::from(r"C:\code bases\station\items.dm"),
            line: 42,
            column: 7,
        }
    }

    #[test]
    fn vscode_command_opens_the_source_at_its_declaration() {
        assert_eq!(
            editor_command("code --goto {file}:{line}:{column}", &source(), None).unwrap(),
            EditorCommand {
                program: String::from("code"),
                arguments: vec![
                    String::from("--goto"),
                    String::from(r"C:\code bases\station\items.dm:42:7")
                ],
            }
        );
    }

    #[test]
    fn root_placeholder_opens_the_workspace_directory_alongside_the_source() {
        let root = Path::new(r"C:\code bases\station");
        for template in [
            "code {root} --goto {file}:{line}:{column}",
            r#"code "{root}" --goto {file}:{line}:{column}"#,
        ] {
            assert_eq!(
                editor_command(template, &source(), Some(root)).unwrap(),
                EditorCommand {
                    program: String::from("code"),
                    arguments: vec![
                        String::from(r"C:\code bases\station"),
                        String::from("--goto"),
                        String::from(r"C:\code bases\station\items.dm:42:7")
                    ],
                }
            );
        }
    }

    #[test]
    fn root_placeholder_requires_a_loaded_environment() {
        assert_eq!(
            editor_command("code {root} --goto {file}:{line}:{column}", &source(), None).unwrap_err(),
            "preferred editor command uses {root}, but no environment is loaded"
        );
    }

    #[test]
    fn quoted_programs_and_fixed_arguments_are_preserved() {
        assert_eq!(
            editor_command(
                r#""C:\Program Files\Editor\editor.exe" --line {line} {file}"#,
                &source(),
                None
            )
            .unwrap(),
            EditorCommand {
                program: String::from(r"C:\Program Files\Editor\editor.exe"),
                arguments: vec![
                    String::from("--line"),
                    String::from("42"),
                    String::from(r"C:\code bases\station\items.dm")
                ],
            }
        );
    }

    #[test]
    fn editor_commands_must_be_valid_and_include_the_file() {
        assert_eq!(
            editor_command("", &source(), None).unwrap_err(),
            "preferred editor command is empty"
        );
        assert_eq!(
            editor_command("code \"{file}", &source(), None).unwrap_err(),
            "preferred editor command has invalid quoting"
        );
        assert_eq!(
            editor_command("code --goto nowhere", &source(), None).unwrap_err(),
            "preferred editor command must contain {file}"
        );
    }
}
