#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]

//! ```sh
//! rmde
//! rmde <file.dme> [file.dmm] [z]
//! rmde <file.dme> [z]
//! rmde <file.dmm> [z]
//! ```
//!
//! With no arguments the editor opens on its welcome page, which can open a codebase or a map.

mod app;
mod bake;
mod camera;
mod clipboard;
mod dm_ui;
mod document;
mod environment;
mod focus;
mod frame;
mod frontend;
mod git;
mod gizmo;
mod issues;
mod maploader;
mod node;
mod region;
mod search;
mod session;
mod settings;
mod tool;
mod transform;

use std::process::ExitCode;

fn main() -> ExitCode {
    app::run()
}
