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
mod blame;
mod camera;
mod clipboard;
mod command;
mod conflict;
mod diff;
mod document;
mod environment;
mod error;
mod fingerprint;
mod focus;
mod fonts;
mod frame;
mod frontend;
mod git;
mod git_worker;
mod gizmo;
mod grid;
mod issues;
mod loader;
mod maploader;
mod node;
mod patch;
mod progress;
mod search;
mod session;
mod settings;
mod state;
mod theme;
mod tool;
mod transform;
mod viewports;
mod visual;
mod world;

use std::process::ExitCode;

fn main() -> ExitCode {
    app::run()
}
