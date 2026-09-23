//! ```sh
//! rmdv <file.dme> [file.dmm] [z]
//! rmdv <file.dme> [z]
//! rmdv <file.dmm> [z]
//! ```
//!
//! Drag to pan, scroll to zoom, PageUp/PageDown to change z level, A to toggle areas, O to toggle area outlines.

mod camera;
mod session;

use std::{path::PathBuf, process::ExitCode};

use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use render::{Device, Renderer};
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{Key, NamedKey},
    window::{Window, WindowId},
};

use crate::{camera::Controller, session::Session};

fn usage() -> ExitCode {
    eprintln!("usage: rmdv <file.dme> [file.dmm] [z]");
    eprintln!("       rmdv <file.dme> [z]");
    eprintln!("       rmdv <file.dmm> [z]");

    ExitCode::FAILURE
}

fn main() -> ExitCode {
    let arguments = std::env::args().skip(1).map(PathBuf::from).collect::<Vec<_>>();

    let arguments = match parse_arguments(&arguments) {
        Ok(arguments) => arguments,
        Err(error) => {
            eprintln!("error: {error}");

            return usage();
        },
    };

    let mut session = Session::new();

    if let Some(entry) = arguments.environment.as_ref()
        && let Err(e) = session.load_environment(entry)
    {
        eprintln!("error: {e}");

        return ExitCode::FAILURE;
    }

    let map = arguments.map.or_else(|| session.first_map());
    let Some(map) = map else {
        eprintln!("error: the environment includes no map, and none was named");

        return ExitCode::FAILURE;
    };

    if let Err(e) = session.open_map(&map, arguments.z) {
        eprintln!("error: {}: {e}", map.display());

        return ExitCode::FAILURE;
    }

    println!(
        "{} sprite instances across {} z level(s)",
        session.sprite_count(),
        session.map().map_or(0, |map| map.size.z)
    );
    println!(
        "{} cells mapped across {} DMI sheets ({:.1} MiB decoded during upload)",
        session.texture_cell_count(),
        session.texture_count(),
        session.texture_bytes() as f64 / (1024.0 * 1024.0)
    );

    let event_loop = match EventLoop::new() {
        Ok(event_loop) => event_loop,
        Err(e) => {
            eprintln!("error: {e}");

            return ExitCode::FAILURE;
        },
    };
    event_loop.set_control_flow(ControlFlow::Poll);

    let mut app = App {
        session,
        camera: Controller::new(),
        window: None,
        renderer: None,
        title: map.file_name().map(|n| n.to_string_lossy().into_owned()),
    };

    match event_loop.run_app(&mut app) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");

            ExitCode::FAILURE
        },
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Arguments {
    environment: Option<PathBuf>,
    map: Option<PathBuf>,
    z: u32,
}

fn parse_arguments(arguments: &[PathBuf]) -> Result<Arguments, String> {
    let (environment, map, z) = match arguments {
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
    let value = argument
        .to_str()
        .and_then(|argument| argument.parse::<u32>().ok())
        .filter(|z| *z > 0)
        .ok_or_else(|| format!("invalid z level '{}': expected a positive integer", argument.display()))?;

    Ok(value)
}

fn is_map(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("dmm"))
}

struct App {
    session: Session,
    camera: Controller,
    window: Option<Window>,
    renderer: Option<Renderer>,
    title: Option<String>,
}

impl App {
    fn start(&mut self, event_loop: &ActiveEventLoop) -> Result<(), Box<dyn std::error::Error>> {
        let title = self.title.as_deref().unwrap_or("rmd");
        let attributes = Window::default_attributes()
            .with_title(format!("rmd - {title}"))
            .with_inner_size(LogicalSize::new(1280, 720));

        let window = event_loop.create_window(attributes)?;
        let size = window.inner_size();

        let device = Device::new(window.window_handle()?.as_raw(), window.display_handle()?.as_raw())?;

        let mut renderer = Renderer::new(device, size.width, size.height, self.session.textures.len())?;
        renderer.upload_textures(&self.session.textures)?;

        self.camera.resize(size.width, size.height);
        let (width, height) = self.session.extent_px();
        self.camera.frame_map(width, height);

        self.window = Some(window);
        self.renderer = Some(renderer);

        Ok(())
    }

    fn redraw(&mut self) {
        let (Some(renderer), Some(window)) = (self.renderer.as_mut(), self.window.as_ref()) else {
            return;
        };

        let map_views = [self.session.map_view(self.camera.camera)];
        let frame = self.session.frame(&map_views);
        window.pre_present_notify();
        if let Err(e) = renderer.draw(&frame) {
            eprintln!("error: {e}");
        }

        window.request_redraw();
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        if let Err(e) = self.start(event_loop) {
            eprintln!("error: {e}");
            event_loop.exit();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),

            WindowEvent::Resized(size) => {
                self.camera.resize(size.width, size.height);

                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.resize(size.width, size.height);
                }
            },

            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                self.camera.set_dragging(state == ElementState::Pressed);
            },

            WindowEvent::CursorMoved { position, .. } => {
                self.camera.cursor_moved(position.x as f32, position.y as f32);
            },

            WindowEvent::MouseWheel { delta, .. } => {
                let steps = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(position) => position.y as f32 / 60.0,
                };

                self.camera.zoom_by(steps);
            },

            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                match event.logical_key.as_ref() {
                    Key::Named(NamedKey::Escape) => event_loop.exit(),
                    Key::Named(NamedKey::PageUp) => self.session.change_level(1),
                    Key::Named(NamedKey::PageDown) => self.session.change_level(-1),
                    Key::Character("a" | "A") => self.session.toggle_areas(),
                    Key::Character("o" | "O") => self.session.toggle_area_outlines(),
                    Key::Character("l" | "L") => self.session.toggle_lighting(),
                    Key::Named(NamedKey::Home) => {
                        let (width, height) = self.session.extent_px();
                        self.camera.frame_map(width, height);
                    },
                    _ => {},
                }
            },

            WindowEvent::RedrawRequested => self.redraw(),

            _ => {},
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(arguments: &[&str]) -> Result<Arguments, String> {
        let arguments = arguments.iter().map(PathBuf::from).collect::<Vec<_>>();

        parse_arguments(&arguments)
    }

    #[test]
    fn defaults_to_first_level() {
        assert_eq!(
            parse(&["map.dmm"]),
            Ok(Arguments {
                environment: None,
                map: Some(PathBuf::from("map.dmm")),
                z: 1,
            })
        );
        assert_eq!(
            parse(&["environment.dme"]),
            Ok(Arguments {
                environment: Some(PathBuf::from("environment.dme")),
                map: None,
                z: 1,
            })
        );
        assert_eq!(
            parse(&["environment.dme", "map.dmm"]),
            Ok(Arguments {
                environment: Some(PathBuf::from("environment.dme")),
                map: Some(PathBuf::from("map.dmm")),
                z: 1,
            })
        );
    }

    #[test]
    fn accepts_a_trailing_level() {
        assert_eq!(
            parse(&["map.dmm", "2"]),
            Ok(Arguments {
                environment: None,
                map: Some(PathBuf::from("map.dmm")),
                z: 2,
            })
        );
        assert_eq!(
            parse(&["environment.dme", "2"]),
            Ok(Arguments {
                environment: Some(PathBuf::from("environment.dme")),
                map: None,
                z: 2,
            })
        );
        assert_eq!(
            parse(&["environment.dme", "map.dmm", "2"]),
            Ok(Arguments {
                environment: Some(PathBuf::from("environment.dme")),
                map: Some(PathBuf::from("map.dmm")),
                z: 2,
            })
        );
    }

    #[test]
    fn rejects_invalid_arguments_and_levels() {
        assert!(parse(&[]).is_err());
        assert!(parse(&["map.dmm", "0"]).is_err());
        assert!(parse(&["map.dmm", "two"]).is_err());
        assert!(parse(&["environment.dme", "not-a-map", "2"]).is_err());
        assert!(parse(&["environment.dme", "map.dmm", "2", "extra"]).is_err());
    }
}
