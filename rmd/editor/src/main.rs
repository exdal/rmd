#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]

//! ```sh
//! rmde
//! rmde <file.dme> [file.dmm] [z]
//! rmde <file.dme> [z]
//! rmde <file.dmm> [z]
//! ```
//!
//! With no arguments the editor opens on its welcome page, which can open a codebase or a map.

mod camera;
mod external_editor;
mod gizmo;
mod loader;
mod logging;
mod session;
mod settings;
mod transform;
mod ui;

use std::{fs, path::PathBuf, process::ExitCode, sync::Arc};

use dear_imgui_rs::{
    BackendFlags,
    ConfigFlags,
    Context,
    FontSource,
    StbTrueTypeFontData,
    StyleColor,
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

use crate::{
    external_editor::SourceLocation,
    loader::{Job, Loader, Outcome},
    session::Session,
    settings::{Settings, imgui_ini_path},
    ui::{LoadNotice, OpenRequest, UiState},
};

const FONT_DATA: &[u8] = include_bytes!("../assets/FiraMono-Regular.ttf");
const MDI_FONT_DATA: &[u8] = include_bytes!("../assets/materialdesignicons-webfont.ttf");
const TABLE_ROW_ALT_ALPHA_SCALE: f32 = 0.4;

fn usage() -> ExitCode {
    log::error!(
        "usage: rmde\n       rmde <file.dme> [file.dmm] [z]\n       rmde <file.dme> [z]\n       rmde <file.dmm> [z]"
    );

    ExitCode::FAILURE
}

fn main() -> ExitCode {
    logging::init();

    let arguments = std::env::args().skip(1).map(PathBuf::from).collect::<Vec<_>>();
    let arguments = match parse_arguments(&arguments) {
        Ok(arguments) => arguments,
        Err(error) => {
            log::error!("{error}");

            return usage();
        },
    };

    let settings = Settings::load();
    let mut session = Session::new();
    settings.apply_to(&mut session.options);

    // The window comes up first and the arguments load behind the progress popup, so a big
    // codebase no longer looks like a hang before anything is on screen.
    let mut loader = Loader::new();
    let mut pending_map = None;
    match arguments.environment {
        Some(entry) => {
            loader.start(Job::Codebase(entry));
            pending_map = Some(PendingMap {
                path: arguments.map,
                z: arguments.z,
            });
        },
        None => {
            if let Some(path) = arguments.map {
                loader.start(Job::Map { path, z: arguments.z });
            }
        },
    }

    let ui = match UiState::new() {
        Ok(ui) => ui,
        Err(e) => {
            log::error!("{e}");

            return ExitCode::FAILURE;
        },
    };
    let event_loop = match EventLoop::new() {
        Ok(event_loop) => event_loop,
        Err(e) => {
            log::error!("{e}");

            return ExitCode::FAILURE;
        },
    };
    event_loop.set_control_flow(ControlFlow::Poll);

    let title = window_title(&session);
    let mut app = App {
        session,
        settings,
        ui,
        loader,
        pending_map,
        uploaded_texture_revision: None,
        title,
        consumer: None,
        renderer: None,
        platform: None,
        imgui: None,
        window: None,
    };

    let result = event_loop.run_app(&mut app);
    app.capture_window_settings();
    app.settings.capture_from(&app.session.options);
    app.settings.save();

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            log::error!("{e}");

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

fn window_title(session: &Session) -> String {
    let map = session
        .map_path()
        .and_then(|path| path.file_name())
        .map(|name| name.to_string_lossy().into_owned());

    match (session.codebase_name(), map) {
        (Some(codebase), Some(map)) => format!("Rapid Mapping Device: {codebase} - {map}"),
        (Some(codebase), None) => format!("Rapid Mapping Device: {codebase}"),
        (None, Some(map)) => format!("Rapid Mapping Device: {map}"),
        (None, None) => "Rapid Mapping Device".to_string(),
    }
}

struct Redraw {
    exit: bool,
    open: Option<OpenRequest>,
    open_source: Option<SourceLocation>,
    pick_new_map_path: bool,
    cancel_load: bool,
    copy_to_clipboard: Option<String>,
}

enum Opened {
    Codebase(PathBuf),
    Map(PathBuf),
}

struct PendingMap {
    path: Option<PathBuf>,
    z: u32,
}

struct App {
    session: Session,
    settings: Settings,
    ui: UiState,
    loader: Loader,
    pending_map: Option<PendingMap>,
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
            .with_maximized(self.settings.maximized)
            .with_inner_size(LogicalSize::new(1280, 720));
        let window = Arc::new(event_loop.create_window(attributes)?);
        let size = window.inner_size();
        let device = Device::new(window.window_handle()?.as_raw(), window.display_handle()?.as_raw())?;

        let mut imgui = Context::create();
        let mut alternate_row = imgui.style().color(StyleColor::TableRowBgAlt);
        alternate_row[3] *= TABLE_ROW_ALT_ALPHA_SCALE;
        imgui.style_mut().set_color(StyleColor::TableRowBgAlt, alternate_row);
        let text_font = StbTrueTypeFontData::from_slice(FONT_DATA)?;
        let mdi_font = StbTrueTypeFontData::from_slice(MDI_FONT_DATA)?;
        imgui.font_atlas().add_font(&[
            FontSource::stb_truetype_with_size(text_font, 16.0),
            FontSource::stb_truetype_with_size(mdi_font, 16.0),
        ]);

        let ini_filename = imgui_ini_path()?;
        if let Some(parent) = ini_filename.parent() {
            fs::create_dir_all(parent)?;
        }
        imgui.set_ini_filename(Some(ini_filename))?;
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

    fn redraw(&mut self) -> Result<Redraw, Box<dyn std::error::Error>> {
        let Self {
            session,
            settings,
            ui,
            loader,
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
            return Ok(Redraw {
                exit: false,
                open: None,
                open_source: None,
                pick_new_map_path: false,
                cancel_load: false,
                copy_to_clipboard: None,
            });
        };

        let current = window_title(session);
        if *title != current {
            window.set_title(&current);
            *title = current;
        }

        if *uploaded_texture_revision != Some(session.texture_revision()) {
            *uploaded_texture_revision = Some(session.texture_revision());
            if let Err(e) = renderer.upload_textures(&session.textures) {
                log::error!("{e}");
                ui.set_open_error(Some(e.to_string()));
            }
        }

        platform.prepare_frame(imgui, window)?;
        let frame = imgui.try_begin_frame()?;
        let load = loader.view();
        let output = ui.draw(frame.ui(), session, settings, load.as_ref())?;
        platform.prepare_render(frame.ui(), window)?;
        let mut drawn = Vec::with_capacity(output.map_views.len());
        let mut map_views = Vec::with_capacity(output.map_views.len());
        let mut picking = None;
        for (index, view) in output.map_views.iter().enumerate() {
            let Some(frame) = session.map_view_frame(view.document, view.rect, view.camera, view.interaction) else {
                continue;
            };
            if output.picking == Some(index) {
                picking = Some(map_views.len());
            }
            drawn.push(view.document);
            map_views.push(frame);
        }
        let scene = session.frame(&map_views, picking);
        let pending = frame.try_render(consumer)?;
        window.pre_present_notify();
        let picked = renderer.draw_imgui(&scene, pending)?;
        if let Some((index, pick)) = picked {
            if let Some(document) = drawn.get(index).copied() {
                session.state.set_active(document);
            }
            match session.tool() {
                Tool::Select => match pick {
                    PickResult::Hit(owner) => {
                        session.select_instance(Some(owner));
                        ui.reveal_selected_instance(session);
                    },
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

        Ok(Redraw {
            exit: output.exit,
            open: output.open,
            open_source: output.open_source,
            pick_new_map_path: output.pick_new_map_path,
            cancel_load: output.cancel_load,
            copy_to_clipboard: output.copy_to_clipboard,
        })
    }

    fn apply_open(&mut self, request: OpenRequest) {
        let resolved = match request {
            OpenRequest::PickCodebase => self.pick_file("BYOND environment", "dme").map(Opened::Codebase),
            OpenRequest::PickMap => self.pick_file("BYOND map", "dmm").map(Opened::Map),
            OpenRequest::Codebase(path) => Some(Opened::Codebase(path)),
            OpenRequest::Map(path) => Some(Opened::Map(path)),
        };
        let Some(resolved) = resolved else {
            return;
        };

        self.ui.set_open_error(None);
        self.ui.set_load_notice(None);
        self.pending_map = None;
        self.loader.start(match resolved {
            Opened::Codebase(path) => Job::Codebase(path),
            Opened::Map(path) => Job::Map { path, z: 1 },
        });
    }

    fn open_source(&mut self, source: SourceLocation) {
        if let Err(error) = external_editor::open(&self.settings.preferred_editor, &source) {
            log::error!(
                "opening {}:{}:{} in the preferred editor: {error}",
                source.path.display(),
                source.line,
                source.column
            );
            self.ui
                .set_load_notice(Some(LoadNotice::failed("Opening source", &source.path, error)));
        }
    }

    fn apply_outcome(&mut self, outcome: Outcome) {
        match outcome {
            Outcome::Codebase { path, loaded } => {
                let report = self.session.apply_codebase(*loaded);
                self.settings.record_codebase(&path);
                self.ui.set_open_error(None);
                self.ui.set_load_notice(
                    (!report.is_empty()).then(|| LoadNotice::diagnostics(&path, report.summary(), report.lines)),
                );
                self.start_pending_map();
            },

            Outcome::Map(loaded) => {
                let path = loaded.path.clone();
                self.session.apply_map(*loaded);
                self.ui.request_refit(self.session.state.active());
                self.ui.set_open_error(None);
                self.ui.set_load_notice(None);
                self.settings.record_recent(self.session.environment_path(), &path);
            },

            Outcome::Failed { job, error } => {
                log::error!("{error}");
                self.pending_map = None;
                self.ui.set_open_error(Some(error.clone()));
                self.ui
                    .set_load_notice(Some(LoadNotice::failed(job.title(), job.path(), error)));
            },

            Outcome::Cancelled => {
                self.pending_map = None;
                self.ui.set_load_notice(None);
            },
        }
    }

    fn start_pending_map(&mut self) {
        let Some(pending) = self.pending_map.take() else {
            return;
        };
        let Some(path) = pending.path.or_else(|| self.session.first_map()) else {
            return;
        };

        self.loader.start(Job::Map { path, z: pending.z });
    }

    fn pick_file(&self, label: &str, extension: &str) -> Option<PathBuf> {
        let dialog = rfd::FileDialog::new().add_filter(label, &[extension]);
        let dialog = match self.window.as_ref() {
            Some(window) => dialog.set_parent(window),
            None => dialog,
        };

        dialog.pick_file()
    }

    fn pick_new_map_path(&mut self) {
        let Some(codebase_dir) = self.session.codebase_dir().map(ToOwned::to_owned) else {
            return;
        };
        let dialog = rfd::FileDialog::new()
            .add_filter("BYOND map", &["dmm"])
            .set_title("Choose new map path")
            .set_directory(&codebase_dir);
        let dialog = match self.window.as_ref() {
            Some(window) => dialog.set_parent(window),
            None => dialog,
        };

        if let Some(path) = dialog.save_file() {
            self.ui.set_new_map_path(path, &codebase_dir);
        }
    }

    fn shutdown(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.capture_window_settings();

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

    fn capture_window_settings(&mut self) {
        if let Some(window) = self.window.as_ref() {
            self.settings.maximized = window.is_maximized();
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        if let Err(e) = self.start(event_loop) {
            log::error!("{e}");
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
            log::error!("{e}");
        }

        match event {
            WindowEvent::CloseRequested => {
                self.ui.request_exit();
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            },

            WindowEvent::Resized(size) => {
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.resize(size.width, size.height);
                }
            },

            WindowEvent::RedrawRequested => {
                let redraw = match self.redraw() {
                    Ok(redraw) => redraw,
                    Err(e) => {
                        log::error!("{e}");

                        Redraw {
                            exit: false,
                            open: None,
                            open_source: None,
                            pick_new_map_path: false,
                            cancel_load: false,
                            copy_to_clipboard: None,
                        }
                    },
                };

                if let (Some(text), Some(imgui)) = (redraw.copy_to_clipboard, self.imgui.as_ref()) {
                    imgui.set_clipboard_text(text);
                }
                if redraw.cancel_load {
                    self.loader.cancel();
                }
                if redraw.pick_new_map_path {
                    self.pick_new_map_path();
                }
                if let Some(request) = redraw.open {
                    self.apply_open(request);
                }

                if let Some(source) = redraw.open_source {
                    self.open_source(source);
                }

                if let Some(outcome) = self.loader.poll() {
                    self.apply_outcome(outcome);
                }

                if redraw.exit {
                    if let Err(e) = self.shutdown() {
                        log::error!("{e}");
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
            log::error!("error while shutting down: {e}");
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
    fn no_arguments_open_nothing() {
        assert_eq!(
            parse(&[]),
            Ok(Arguments {
                environment: None,
                map: None,
                z: 1,
            })
        );
    }

    #[test]
    fn rejects_invalid_arguments_and_levels() {
        assert!(parse(&["map.dmm", "0"]).is_err());
        assert!(parse(&["map.dmm", "two"]).is_err());
        assert!(parse(&["environment.dme", "not-a-map", "2"]).is_err());
        assert!(parse(&["environment.dme", "map.dmm", "2", "extra"]).is_err());
    }
}
