mod args;
mod events;
pub(crate) mod external_editor;
mod logging;
pub(crate) mod pacing;
pub(crate) mod process;
mod redraw;
mod screenshot;
#[cfg(test)]
mod tests;
pub(crate) mod update;

use std::{
    fs, mem,
    path::{Path, PathBuf},
    process::ExitCode,
    sync::Arc,
    time::Instant,
};

use dear_imgui_rs::{BackendFlags, ConfigFlags, Context, Style, render::SynchronousRendererConsumer};
use dear_imgui_winit::{HiDpiMode, WinitPlatform};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use render::{Device, Renderer};
use ui::{fonts::UiFont, theme::Themes};
use winit::{
    dpi::LogicalSize,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::Window,
};

use crate::{
    document::{DocumentId, Selection},
    environment::BakeOptions,
    frontend::{LoadNotice, OpenRequest, ProfileReload, ScreenshotArea, ScreenshotRequest, UiState},
    loader::{Job, Loader, Outcome},
    session::{LoadReport, Session},
    settings::{Settings, backup_dir, imgui_ini_path, profiles_dir, themes_dir},
    viewports,
};

use args::{parse_arguments, usage};
use external_editor::SourceLocation;
use pacing::{FrameDemand, FramePacer, NextFrame, Status};
use screenshot::{PendingScreenshot, ScreenshotTarget, SystemClipboard};

fn style_base(themes: &Themes, settings: &Settings) -> Style {
    let mut style = themes.active_style().clone();
    style.set_font_size_base(settings.font.size as f32);

    style
}

pub(crate) fn run() -> ExitCode {
    logging::init();

    let arguments = std::env::args().skip(1).map(PathBuf::from).collect::<Vec<_>>();
    let arguments = match parse_arguments(&arguments) {
        Ok(arguments) => arguments,
        Err(error) => {
            log::error!("{error}");

            return usage();
        },
    };

    let loaded_settings = Settings::load();
    let settings_ready = !loaded_settings.needs_keybind_preset;
    let settings = loaded_settings.settings;
    let profiles = install_profiles();
    let mut session = Session::new();
    session.sync_git_enabled(settings.git_enabled);
    settings.apply_to(&mut session.options);

    let (startup_job, pending_map) = match arguments.environment {
        Some(entry) => {
            let bake = bake_options(&settings, Some(&entry));

            (
                Some(Job::Codebase { path: entry, bake }),
                Some(PendingMap {
                    path: arguments.map,
                    z: arguments.z,
                }),
            )
        },
        None => (
            arguments.map.map(|path| Job::Map {
                path,
                z: arguments.z,
                git_enabled: settings.git_enabled,
            }),
            None,
        ),
    };

    let mut loader = match backup_dir() {
        Ok(root) => Loader::with_backups(root),
        Err(error) => {
            log::warn!("map backups are off: {error}");
            Loader::new()
        },
    };
    let deferred_job = if settings_ready {
        if let Some(job) = startup_job {
            loader.start(job);
        }
        None
    } else {
        startup_job
    };

    let mut ui = match UiState::new(!settings_ready) {
        Ok(ui) => ui,
        Err(e) => {
            log::error!("{e}");

            return ExitCode::FAILURE;
        },
    };
    ui.set_profiles(profiles);
    let event_loop = match EventLoop::new() {
        Ok(event_loop) => event_loop,
        Err(e) => {
            log::error!("{e}");

            return ExitCode::FAILURE;
        },
    };
    event_loop.set_control_flow(ControlFlow::Wait);

    let title = window_title(&session);
    let mut app = App {
        session,
        settings,
        ui,
        loader,
        pending_map,
        pending_reload: None,
        pending_share: None,
        retry_coop_after_load: false,
        deferred_job,
        settings_ready,
        uploaded_texture_revision: None,
        pending_screenshot: None,
        screenshot_dir: None,
        clipboard: None,
        title,
        consumer: None,
        renderer: None,
        platform: None,
        imgui: None,
        scaled_style: None,
        themes: None,
        ui_font: None,
        window: None,
        pacer: FramePacer::default(),
        is_occluded: false,
    };

    let result = event_loop.run_app(&mut app);
    if app.settings_ready {
        app.capture_window_settings();
        app.settings.capture_from(&app.session.options);
        app.settings.save();
    }

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            log::error!("{e}");

            ExitCode::FAILURE
        },
    }
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

#[derive(Default)]
struct Redraw {
    exit: bool,
    open: Option<OpenRequest>,
    open_source: Option<SourceLocation>,
    pick_new_map_path: bool,
    screenshot: Option<ScreenshotRequest>,
    cancel_load: bool,
    copy_to_clipboard: Option<String>,
    reload_profile: Option<ProfileReload>,
    load_conflicts: Option<DocumentId>,
    demand: FrameDemand,
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
    pending_reload: Option<DocumentId>,
    pending_share: Option<PathBuf>,
    retry_coop_after_load: bool,
    deferred_job: Option<Job>,
    settings_ready: bool,
    uploaded_texture_revision: Option<u64>,
    pending_screenshot: Option<PendingScreenshot>,
    screenshot_dir: Option<PathBuf>,
    clipboard: Option<arboard::Clipboard>,
    title: String,
    consumer: Option<SynchronousRendererConsumer>,
    renderer: Option<Renderer>,
    platform: Option<WinitPlatform>,
    imgui: Option<Context>,
    scaled_style: Option<viewports::ScaledStyle>,
    themes: Option<Themes>,
    ui_font: Option<UiFont>,
    window: Option<Arc<Window>>,
    pacer: FramePacer,
    is_occluded: bool,
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
        match arboard::Clipboard::new() {
            Ok(clipboard) => imgui.set_clipboard_backend(SystemClipboard(clipboard)),
            Err(e) => log::error!("could not open the clipboard: {e}"),
        }

        let themes_dir = themes_dir()
            .inspect_err(|e| log::error!("could not find the themes folder: {e}"))
            .ok();
        let themes = Themes::load(themes_dir, imgui.style(), &self.settings.theme);
        let ui_font = UiFont::new(imgui.font_atlas(), &mut self.settings.font)?;

        let ini_filename = imgui_ini_path()?;
        if let Some(parent) = ini_filename.parent() {
            fs::create_dir_all(parent)?;
        }
        imgui.set_ini_filename(Some(ini_filename))?;
        imgui.set_renderer_name(Some("rmd vir"))?;
        let config = imgui.io().config_flags() | ConfigFlags::DOCKING_ENABLE;
        imgui.io_mut().set_config_flags(config);
        imgui.io_mut().set_config_input_text_cursor_blink(false);

        let mut platform = WinitPlatform::new(&mut imgui)?;
        platform.attach_window(Arc::clone(&window), HiDpiMode::Default, &mut imgui)?;

        let backend =
            imgui.io().backend_flags() | BackendFlags::RENDERER_HAS_TEXTURES | BackendFlags::RENDERER_HAS_VTX_OFFSET;
        imgui.io_mut().set_backend_flags(backend);
        let consumer = imgui.create_synchronous_renderer_consumer()?;

        let viewports_enabled = match platform.enable_viewports(&mut imgui) {
            Ok(()) => {
                viewports::install(&mut imgui);
                imgui.enable_multi_viewport();
                true
            },
            Err(e) => {
                log::warn!("imgui windows stay inside the main window: {e}");
                false
            },
        };
        let mut scaled_style = viewports::ScaledStyle::new(&mut imgui, viewports_enabled && !cfg!(target_os = "macos"));
        scaled_style.apply(
            &mut imgui,
            &style_base(&themes, &self.settings),
            window.scale_factor() as f32,
            self.settings.ui_scale_override_percent,
        );

        let mut renderer = Renderer::new(device, size.width, size.height, self.session.textures.len())?;
        renderer.set_textures(&self.session.textures)?;

        self.uploaded_texture_revision = Some(self.session.texture_revision());
        self.consumer = Some(consumer);
        self.renderer = Some(renderer);
        self.platform = Some(platform);
        self.imgui = Some(imgui);
        self.scaled_style = Some(scaled_style);
        self.themes = Some(themes);
        self.ui_font = Some(ui_font);
        self.window = Some(window);

        Ok(())
    }

    fn apply_open(&mut self, request: OpenRequest) {
        if self.loader.is_busy() {
            return;
        }

        self.pending_share = None;
        let resolved = match request {
            OpenRequest::PickCodebase => self.pick_file("BYOND environment", "dme").map(Opened::Codebase),
            OpenRequest::PickMap => self.pick_file("BYOND map", "dmm").map(Opened::Map),
            OpenRequest::Codebase(path) => Some(Opened::Codebase(path)),
            OpenRequest::Map(path) => Some(Opened::Map(path)),
            OpenRequest::ReloadCoop => {
                let Some(path) = self.session.environment_path().map(PathBuf::from) else {
                    return;
                };

                self.retry_coop_after_load = true;
                Some(Opened::Codebase(path))
            },
            OpenRequest::ShareMap(path) => {
                if self.session.open_coop_file(&path) || self.session.share_coop_file(&path) {
                    return;
                }

                self.pending_share = Some(path.clone());
                Some(Opened::Map(path))
            },
        };
        let Some(resolved) = resolved else {
            return;
        };

        if let Opened::Map(path) = &resolved
            && self.session.open_coop_file(path)
        {
            return;
        }

        if let Opened::Map(path) = &resolved
            && let Some(id) = self.session.state.document_for_path(path)
        {
            self.pending_share = None;
            self.ui.focus_map(&mut self.session, id);
            return;
        }

        self.ui.set_open_error(None);
        self.ui.set_load_notice(None);
        self.pending_map = None;
        self.loader.start(match resolved {
            Opened::Codebase(path) => {
                self.session.begin_coop_reload();
                let bake = bake_options(&self.settings, Some(&path));

                Job::Codebase { path, bake }
            },
            Opened::Map(path) => Job::Map {
                path,
                z: 1,
                git_enabled: self.settings.git_enabled,
            },
        });
    }

    fn reload_profile(&mut self, request: ProfileReload) {
        if self.loader.is_busy() {
            return;
        }

        let Some(path) = self.session.environment_path().map(PathBuf::from) else {
            return;
        };
        let mut bake = bake_options(&self.settings, Some(&path));
        match request {
            ProfileReload::Select(profile) => {
                bake.forced_profile = None;
                bake.profile = Some(base::path::TreePath::parse(&profile));
            },
            ProfileReload::Force(profile) => bake.forced_profile = profile,
            ProfileReload::Current => {},
        }
        self.ui.set_load_notice(None);
        self.session.begin_coop_reload();
        self.loader.start(Job::Codebase { path, bake });
    }

    fn open_source(&mut self, source: SourceLocation) {
        if let Err(error) = external_editor::open(&self.settings.preferred_editor, &source, self.session.codebase_dir())
        {
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

    fn diagnostic_path(&self, path: &Path) -> String {
        self.session
            .codebase_dir()
            .and_then(|base| path.strip_prefix(base).ok())
            .unwrap_or(path)
            .display()
            .to_string()
    }

    fn apply_outcome(&mut self, outcome: Outcome) {
        match outcome {
            Outcome::Codebase { path, loaded } => {
                let forced_profile = loaded.environment.bake_options.forced_profile.as_deref();
                self.settings.set_forced_profile_for(&path, forced_profile);
                if forced_profile.is_none()
                    && let Some(profiles) = loaded.environment.profiles.as_ref()
                {
                    let selected = (profiles.active != profiles.default).then_some(profiles.active.as_str());
                    self.settings.set_profile_for(&path, selected);
                }
                let report = self.session.apply_codebase(*loaded);
                self.settings.record_codebase(&path);
                self.ui.set_open_error(None);
                self.ui.set_load_notice(None);
                self.ui.set_codebase_report(report);
                if mem::take(&mut self.retry_coop_after_load) && self.session.coop().is_some() {
                    match self.session.retry_coop() {
                        Ok(()) => self.ui.reset_coop_notice(),
                        Err(error) => self.ui.set_open_error(Some(error)),
                    }
                }
                self.start_pending_map();
            },

            Outcome::Map(loaded) => {
                let path = loaded.path.clone();
                let report = LoadReport::map(
                    &self.diagnostic_path(&path),
                    &loaded.map,
                    self.session.tree(),
                    &loaded.errors,
                );
                if let Some(id) = self.pending_reload.take() {
                    self.session.reload_map(id, *loaded);
                } else {
                    self.session.apply_map(*loaded);
                    self.ui.request_refit(self.session.state.active());
                }

                if self.pending_share.take_if(|pending| *pending == path).is_some() {
                    self.session.share_coop_file(&path);
                }
                self.ui.set_open_error(None);
                self.ui.set_load_notice(None);
                self.ui
                    .set_map_report(fs::canonicalize(&path).unwrap_or_else(|_| path.clone()), report);
                self.settings.record_recent(self.session.environment_path(), &path);
            },

            Outcome::Failed { job, error } => {
                self.retry_coop_after_load = false;
                self.session.finish_coop_reload();
                log::error!("{error}");
                self.pending_map = None;
                self.pending_reload = None;
                self.pending_share = None;
                self.ui.set_open_error(Some(error.clone()));
                self.ui.set_load_notice(None);
                match job {
                    Job::Codebase { path, .. } => {
                        let report = LoadReport::failure(&path.display().to_string(), &path, &error);
                        self.ui.set_failed_codebase_report(report);
                    },
                    Job::Map { path, .. } => {
                        let report = LoadReport::failure(&self.diagnostic_path(&path), &path, &error);
                        self.ui.set_map_report(fs::canonicalize(&path).unwrap_or(path), report);
                    },
                }
            },

            Outcome::Cancelled => {
                self.retry_coop_after_load = false;
                self.session.finish_coop_reload();
                self.pending_map = None;
                self.pending_reload = None;
                self.pending_share = None;
                self.ui.set_load_notice(None);
            },
        }
        self.pacer.activity(Instant::now());
    }

    fn start_pending_map(&mut self) {
        let Some(pending) = self.pending_map.take() else {
            return;
        };
        let Some(path) = pending.path.or_else(|| self.session.first_map()) else {
            return;
        };

        self.loader.start(Job::Map {
            path,
            z: pending.z,
            git_enabled: self.settings.git_enabled,
        });
    }

    fn load_conflicts(&mut self, id: DocumentId) {
        if self.loader.is_busy() {
            return;
        }
        let Some(document) = self.session.state.document(id) else {
            return;
        };
        let Some(path) = document.path.clone() else { return };
        self.pending_reload = Some(id);
        self.loader.start(Job::Map {
            path,
            z: document.z,
            git_enabled: self.settings.git_enabled,
        });
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

    fn request_screenshot(&mut self, request: ScreenshotRequest) {
        let Some(document) = self.session.state.active_document() else {
            return;
        };
        let selection = match request.area {
            ScreenshotArea::Map => None,
            ScreenshotArea::Selection => match document.selection() {
                Some(selection) => Some(*selection),
                None => return,
            },
        };
        let target = if request.copy {
            ScreenshotTarget::Clipboard
        } else {
            match self.pick_screenshot_path(selection) {
                Some(path) => ScreenshotTarget::File(path),
                None => return,
            }
        };

        self.pending_screenshot = Some(PendingScreenshot { target, selection });
    }

    fn pick_screenshot_path(&mut self, selection: Option<Selection>) -> Option<PathBuf> {
        let document = self.session.state.active_document()?;
        let map_path = document.path.as_deref();
        let stem = map_path
            .and_then(Path::file_stem)
            .map_or_else(|| "map".into(), |stem| stem.to_string_lossy());
        let directory = self
            .screenshot_dir
            .clone()
            .or_else(|| map_path.and_then(Path::parent).map(Path::to_path_buf));
        let dialog = rfd::FileDialog::new()
            .add_filter("PNG image", &["png"])
            .set_title("Save screenshot")
            .set_file_name(match selection {
                Some(selection) => format!(
                    "{stem}_z{}_{}-{}_{}-{}.png",
                    document.z, selection.min.x, selection.min.y, selection.max.x, selection.max.y
                ),
                None => format!("{stem}_z{}.png", document.z),
            });
        let dialog = match directory {
            Some(directory) => dialog.set_directory(directory),
            None => dialog,
        };
        let dialog = match self.window.as_ref() {
            Some(window) => dialog.set_parent(window),
            None => dialog,
        };

        let mut path = dialog.save_file()?;
        if path.extension().is_none() {
            path.set_extension("png");
        }
        self.screenshot_dir = path.parent().map(Path::to_path_buf);

        Some(path)
    }

    fn shutdown(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.capture_window_settings();

        if let Some(renderer) = self.renderer.as_mut() {
            renderer.release_viewports()?;
        }
        if let (Some(platform), Some(context)) = (self.platform.as_mut(), self.imgui.as_mut())
            && platform.viewports_enabled()
        {
            viewports::uninstall(context);
            platform.disable_viewports(context)?;
        }

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

    fn is_busy(&self) -> bool {
        self.loader.is_busy() || self.session.has_background_work() || self.ui.is_checking_for_updates()
    }

    fn schedule_frame(&self, event_loop: &ActiveEventLoop, frame_start: Instant, demand: FrameDemand, status: Status) {
        let Some(window) = self.window.as_ref() else {
            return;
        };

        match self.pacer.next(Instant::now(), frame_start, demand, status) {
            NextFrame::Now => {
                event_loop.set_control_flow(ControlFlow::Wait);
                window.request_redraw();
            },
            NextFrame::At(at) => event_loop.set_control_flow(ControlFlow::WaitUntil(at)),
            NextFrame::OnEvent => event_loop.set_control_flow(ControlFlow::Wait),
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

fn install_profiles() -> Vec<String> {
    let dir = match profiles_dir() {
        Ok(dir) => dir,
        Err(error) => {
            log::warn!("profiles fall back to the embedded copies: {error}");

            return crate::environment::BundledProfile::ALL
                .map(|profile| profile.stem().to_owned())
                .to_vec();
        },
    };
    if let Err(error) = crate::environment::write_missing_profiles(&dir) {
        log::warn!("writing profiles to {}: {error}", dir.display());
    }

    crate::environment::scan_profiles(&dir)
}

fn bake_options(settings: &Settings, environment: Option<&std::path::Path>) -> BakeOptions {
    BakeOptions {
        enabled: crate::environment::baking_enabled(settings.bake_enabled),
        optimizations_enabled: settings.optimizations_enabled,
        profile: environment
            .and_then(|path| settings.profile_for(path))
            .map(base::path::TreePath::parse),
        forced_profile: environment
            .and_then(|path| settings.forced_profile_for(path))
            .map(str::to_owned),
        profile_dir: profiles_dir().ok(),
        ..Default::default()
    }
}
