//! ```sh
//! rmde <file.dme> [file.dmm] [z]
//! rmde <file.dme> [z]
//! rmde <file.dmm> [z]
//! ```

mod camera;
mod gizmo;
mod inspector;
mod session;
mod settings;
mod transform;
mod ui;

use std::{path::PathBuf, process::ExitCode, sync::Arc};

use dear_imgui_rs::{
    BackendFlags,
    ConfigFlags,
    Context,
    FontSource,
    StbTrueTypeFontData,
    render::SynchronousRendererConsumer,
};
use dear_imgui_winit::{HiDpiMode, WinitPlatform};
use editor::tool::Tool;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use render::{Device, PickResult, Renderer};
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{Window, WindowId},
};

use crate::{camera::Controller, session::Session, settings::Settings, ui::UiState};

const FONT_DATA: &[u8] = include_bytes!("../assets/FiraMono-Regular.ttf");
const MDI_FONT_DATA: &[u8] = include_bytes!("../assets/materialdesignicons-webfont.ttf");

fn usage() -> ExitCode {
    eprintln!("usage: rmde <file.dme> [file.dmm] [z]");
    eprintln!("       rmde <file.dme> [z]");
    eprintln!("       rmde <file.dmm> [z]");

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

    let settings = Settings::load();
    let mut session = Session::new();
    settings.apply_to(&mut session.options);
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

    let ui = match UiState::new() {
        Ok(ui) => ui,
        Err(e) => {
            eprintln!("error: {e}");

            return ExitCode::FAILURE;
        },
    };
    let event_loop = match EventLoop::new() {
        Ok(event_loop) => event_loop,
        Err(e) => {
            eprintln!("error: {e}");

            return ExitCode::FAILURE;
        },
    };
    event_loop.set_control_flow(ControlFlow::Poll);

    let title = window_title(&session);
    let mut app = App {
        session,
        settings,
        camera: Controller::new(),
        ui,
        uploaded_texture_revision: None,
        title,
        consumer: None,
        renderer: None,
        platform: None,
        imgui: None,
        window: None,
    };

    let result = event_loop.run_app(&mut app);
    app.settings.capture_from(&app.session.options);
    app.settings.save();

    match result {
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

fn window_title(session: &Session) -> String {
    let map = session
        .map_path()
        .and_then(|path| path.file_name())
        .map(|name| name.to_string_lossy().into_owned());

    match (session.codebase_name(), map) {
        (Some(codebase), Some(map)) => format!("Rapid Map Editor: {codebase} - {map}"),
        (Some(codebase), None) => format!("Rapid Map Editor: {codebase}"),
        (None, Some(map)) => format!("Rapid Map Editor: {map}"),
        (None, None) => "Rapid Map Editor".to_string(),
    }
}

struct App {
    session: Session,
    settings: Settings,
    camera: Controller,
    ui: UiState,
    uploaded_texture_revision: Option<u64>,
    title: String,
    consumer: Option<SynchronousRendererConsumer>,
    renderer: Option<Renderer>,
    platform: Option<WinitPlatform>,
    imgui: Option<Context>,
    window: Option<Arc<Window>>,
}

impl App {
    fn start(&mut self, event_loop: &ActiveEventLoop) -> Result<(), Box<dyn std::error::Error>> {
        let attributes = Window::default_attributes()
            .with_title(&self.title)
            .with_inner_size(LogicalSize::new(1280, 720));
        let window = Arc::new(event_loop.create_window(attributes)?);
        let size = window.inner_size();
        let device = Device::new(window.window_handle()?.as_raw(), window.display_handle()?.as_raw())?;

        let mut imgui = Context::create();
        let text_font = StbTrueTypeFontData::from_slice(FONT_DATA)?;
        let mdi_font = StbTrueTypeFontData::from_slice(MDI_FONT_DATA)?;
        imgui.font_atlas().add_font(&[
            FontSource::stb_truetype_with_size(text_font, 16.0),
            FontSource::stb_truetype_with_size(mdi_font, 16.0),
        ]);

        imgui.set_ini_filename(Some("imgui.ini"))?;
        imgui.set_renderer_name(Some("rmd vir"))?;
        let config = imgui.io().config_flags() | ConfigFlags::DOCKING_ENABLE;
        imgui.io_mut().set_config_flags(config);

        let mut platform = WinitPlatform::new(&mut imgui)?;
        platform.attach_window(Arc::clone(&window), HiDpiMode::Default, &mut imgui)?;

        let backend =
            imgui.io().backend_flags() | BackendFlags::RENDERER_HAS_TEXTURES | BackendFlags::RENDERER_HAS_VTX_OFFSET;
        imgui.io_mut().set_backend_flags(backend);
        let consumer = imgui.create_synchronous_renderer_consumer()?;

        let mut renderer = Renderer::new(device, size.width, size.height, self.session.textures.len())?;
        renderer.upload_textures(&self.session.textures)?;

        self.uploaded_texture_revision = Some(self.session.texture_revision());
        self.consumer = Some(consumer);
        self.renderer = Some(renderer);
        self.platform = Some(platform);
        self.imgui = Some(imgui);
        self.window = Some(window);

        Ok(())
    }

    fn redraw(&mut self) -> Result<bool, Box<dyn std::error::Error>> {
        let Self {
            session,
            settings,
            camera,
            ui,
            uploaded_texture_revision,
            title,
            consumer,
            renderer,
            platform,
            imgui,
            window,
            ..
        } = self;
        let (Some(consumer), Some(renderer), Some(platform), Some(imgui), Some(window)) = (
            consumer.as_ref(),
            renderer.as_mut(),
            platform.as_mut(),
            imgui.as_mut(),
            window.as_ref(),
        ) else {
            return Ok(false);
        };

        let current = window_title(session);
        if *title != current {
            window.set_title(&current);
            *title = current;
        }

        if *uploaded_texture_revision != Some(session.texture_revision()) {
            renderer.upload_textures(&session.textures)?;
            *uploaded_texture_revision = Some(session.texture_revision());
        }

        platform.prepare_frame(imgui, window)?;
        let frame = imgui.try_begin_frame()?;
        let output = ui.draw(frame.ui(), session, settings, camera)?;
        platform.prepare_render(frame.ui(), window)?;
        let scene = session.frame(camera.camera);
        let pending = frame.try_render(consumer)?;
        window.pre_present_notify();
        let picked = renderer.draw_imgui(&scene, output.viewport, pending, output.interaction)?;
        if let Some(pick) = picked {
            match session.tool() {
                Tool::Select => match pick {
                    PickResult::Hit(owner) => session.select_instance(Some(owner)),
                    PickResult::Miss => session.select_instance(None),
                },
                Tool::Delete => {
                    if let PickResult::Hit(owner) = pick {
                        session.delete_instance(owner);
                    }
                },
                Tool::Place | Tool::BlockSelect | Tool::Fill => {},
            }
        }

        Ok(output.exit)
    }

    fn shutdown(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if let (Some(context), Some(consumer), Some(renderer)) =
            (self.imgui.as_mut(), self.consumer.as_ref(), self.renderer.as_mut())
        {
            let reset = context.prepare_renderer_texture_reset(consumer)?;
            renderer.reset_imgui_textures()?;
            reset.commit();
        }

        drop(self.consumer.take());

        if let (Some(platform), Some(context)) = (self.platform.as_mut(), self.imgui.as_mut()) {
            let _ = platform.detach_window(context)?;
        }

        drop(self.renderer.take());
        drop(self.platform.take());
        drop(self.imgui.take());
        drop(self.window.take());

        Ok(())
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

            return;
        }

        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let is_main_window = self.window.as_ref().is_some_and(|window| window.id() == id);
        if !is_main_window {
            return;
        }

        if let (Some(platform), Some(imgui), Some(window)) =
            (self.platform.as_mut(), self.imgui.as_mut(), self.window.as_ref())
            && let Err(e) = platform.handle_window_event(imgui, window, &event)
        {
            eprintln!("error: {e}");
        }

        match event {
            WindowEvent::CloseRequested => {
                if let Err(e) = self.shutdown() {
                    eprintln!("error: {e}");
                }
                event_loop.exit();
            },

            WindowEvent::Resized(size) => {
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.resize(size.width, size.height);
                }
            },

            WindowEvent::RedrawRequested => {
                let exit = match self.redraw() {
                    Ok(exit) => exit,
                    Err(e) => {
                        eprintln!("error: {e}");

                        false
                    },
                };

                if exit {
                    if let Err(e) = self.shutdown() {
                        eprintln!("error: {e}");
                    }
                    event_loop.exit();
                } else if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            },

            _ => {},
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        if let Err(e) = self.shutdown() {
            eprintln!("error while shutting down: {e}");
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
