use std::{path::PathBuf, process::ExitCode};

pub(super) fn usage() -> ExitCode {
    log::error!(
        "usage: rmde\n       rmde <file.dme> [file.dmm] [z]\n       rmde <file.dme> [z]\n       rmde <file.dmm> [z]"
    );

    ExitCode::FAILURE
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Arguments {
    pub(super) environment: Option<PathBuf>,
    pub(super) map: Option<PathBuf>,
    pub(super) z: u32,
}

pub(super) fn parse_arguments(arguments: &[PathBuf]) -> Result<Arguments, String> {
    let (environment, map, z) = match arguments {
        [] => (None, None, 1),
        [entry] if is_map(entry) => (None, Some(entry.clone()), 1),
        [entry] => (Some(entry.clone()), None, 1),
        [entry, z] if is_map(entry) => (None, Some(entry.clone()), parse_z(z)?),
        [entry, map] if is_map(map) => (Some(entry.clone()), Some(map.clone()), 1),
        [entry, z] => (Some(entry.clone()), None, parse_z(z)?),
        [entry, map, z] if !is_map(entry) && is_map(map) => (Some(entry.clone()), Some(map.clone()), parse_z(z)?),
        _ => return Err("invalid arguments".to_string()),
    };

    Ok(Arguments { environment, map, z })
}

fn parse_z(argument: &std::path::Path) -> Result<u32, String> {
    argument
        .to_str()
        .and_then(|argument| argument.parse::<u32>().ok())
        .filter(|z| *z > 0)
        .ok_or_else(|| format!("invalid z level '{}': expected a positive integer", argument.display()))
}

fn is_map(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("dmm"))
}
