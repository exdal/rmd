use std::{
    env, io,
    path::{Path, PathBuf},
};

use crate::build_info::VERSION;

pub(super) fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

pub(super) fn settings_path() -> io::Result<PathBuf> {
    #[cfg(target_os = "windows")]
    let variable = "LOCALAPPDATA";
    #[cfg(not(target_os = "windows"))]
    let variable = "HOME";

    let root = env::var_os(variable)
        .filter(|root| !root.is_empty())
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("{variable} is not set")))?;

    Ok(settings_path_from(Path::new(&root), cfg!(target_os = "windows")))
}

pub(crate) fn imgui_ini_path() -> io::Result<PathBuf> {
    Ok(settings_path()?.with_file_name("imgui.ini"))
}

pub(crate) fn backup_dir() -> io::Result<PathBuf> {
    Ok(settings_path()?.with_file_name("backup"))
}

pub(crate) fn crash_dir() -> io::Result<PathBuf> {
    Ok(settings_path()?.with_file_name("crashes"))
}

pub(crate) fn recovery_dir() -> io::Result<PathBuf> {
    Ok(settings_path()?.with_file_name("recovery"))
}

pub(crate) fn themes_dir() -> io::Result<PathBuf> {
    Ok(settings_path()?.with_file_name("themes"))
}

pub(crate) fn profiles_dir() -> io::Result<PathBuf> {
    Ok(settings_path()?.with_file_name("profiles").join(VERSION))
}

#[cfg(target_os = "windows")]
pub(crate) fn log_path() -> io::Result<PathBuf> {
    Ok(settings_path()?.with_file_name("latest.log"))
}

pub(super) fn settings_path_from(root: &Path, windows: bool) -> PathBuf {
    if windows {
        root.join("rmd/settings.toml")
    } else {
        root.join(".config/rmd/settings.toml")
    }
}
