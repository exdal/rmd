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
mod clipboard;
mod coop;
mod dm_ui;
mod document;
mod environment;
mod focus;
mod frame;
mod frontend;
mod git;
mod gizmo;
mod inspector;
mod issues;
mod maploader;
mod node;
mod object_tree;
mod palette;
mod region;
mod search;
mod session;
mod settings;
#[cfg(test)]
mod testing;
mod tool;
mod viewport;

use std::process::ExitCode;

fn main() -> ExitCode {
    app::run()
}
