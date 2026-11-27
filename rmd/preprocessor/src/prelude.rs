use std::path::PathBuf;

/// `DM_STDDEF`
pub const STDDEF_ENV: &str = "DM_STDDEF";

pub const DEMIR_ENV: &str = "DM_DEMIR";

pub use prelude::{CORE_SOURCE, DEMIR_SOURCE, IMGUI_SOURCE, STDDEF_EXT_SOURCE, STDDEF_SOURCE, VERSION_SOURCE};

pub enum PreludeFile {
    Embedded(&'static str, &'static str),
    Disk(PathBuf),
}

pub fn core_files() -> [PreludeFile; 2] {
    [
        PreludeFile::Embedded("<version.dm>", VERSION_SOURCE),
        PreludeFile::Embedded("<core.dm>", CORE_SOURCE),
    ]
}

pub fn prelude_files() -> Vec<PreludeFile> {
    vec![
        env_override(STDDEF_ENV, PreludeFile::Embedded("<stddef.dm>", STDDEF_SOURCE)),
        PreludeFile::Embedded("<stddef_ext.dm>", STDDEF_EXT_SOURCE),
        env_override(DEMIR_ENV, PreludeFile::Embedded("<demir.dm>", DEMIR_SOURCE)),
        PreludeFile::Embedded("<imgui.dm>", IMGUI_SOURCE),
    ]
}

fn env_override(variable: &str, embedded: PreludeFile) -> PreludeFile {
    match std::env::var_os(variable) {
        Some(path) => PreludeFile::Disk(PathBuf::from(path)),
        None => embedded,
    }
}
