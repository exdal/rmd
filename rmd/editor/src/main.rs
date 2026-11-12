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
mod backup;
mod bake;
mod baker;
mod camera;
mod clipboard;
mod command;
mod document;
mod environment;
mod focus;
mod frame;
mod frontend;
mod git;
mod gizmo;
mod grid;
mod issues;
mod maploader;
mod node;
mod patch;
mod search;
mod session;
mod settings;
mod state;
mod tool;
mod transform;
mod visual;

use std::process::ExitCode;

fn main() -> ExitCode {
    app::run()
}
