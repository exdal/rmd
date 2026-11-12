use std::time::Instant;

use winit::{
    application::ApplicationHandler,
    event::{Event, StartCause, WindowEvent},
    event_loop::ActiveEventLoop,
    window::WindowId,
};

use crate::viewports;

use super::{App, Redraw, pacing::Status};

pub(super) fn is_input(event: &WindowEvent) -> bool {
    matches!(
        event,
        WindowEvent::CursorMoved { .. }
            | WindowEvent::CursorEntered { .. }
            | WindowEvent::CursorLeft { .. }
            | WindowEvent::MouseInput { .. }
            | WindowEvent::MouseWheel { .. }
            | WindowEvent::KeyboardInput { .. }
            | WindowEvent::ModifiersChanged(_)
            | WindowEvent::Ime(_)
            | WindowEvent::Focused(_)
            | WindowEvent::Moved(_)
            | WindowEvent::Resized(_)
            | WindowEvent::ScaleFactorChanged { .. }
            | WindowEvent::DroppedFile(_)
            | WindowEvent::HoveredFile(_)
            | WindowEvent::HoveredFileCancelled
            | WindowEvent::Occluded(false)
    )
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

    fn new_events(&mut self, _: &ActiveEventLoop, cause: StartCause) {
        if let StartCause::ResumeTimeReached { .. } = cause
            && let Some(window) = self.window.as_ref()
        {
            window.request_redraw();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if is_input(&event) {
            self.pacer.activity(Instant::now());
            if let Some(window) = self.window.as_ref() {
                window.request_redraw();
            }
        }

        let is_main_window = self.window.as_ref().is_some_and(|window| window.id() == id);
        if !is_main_window {
            if let (Some(platform), Some(imgui), Some(window)) =
                (self.platform.as_mut(), self.imgui.as_mut(), self.window.as_ref())
                && let Err(e) = platform.handle_event(imgui, window, &Event::<()>::WindowEvent { window_id: id, event })
            {
                log::error!("{e}");
            }

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

            WindowEvent::Focused(true) if self.settings.git_enabled => {
                self.session.refresh_git();
            },

            WindowEvent::Occluded(is_occluded) => {
                self.is_occluded = is_occluded;
            },

            WindowEvent::RedrawRequested => {
                let frame_start = Instant::now();
                let was_busy = self.is_busy();
                let is_hidden = self.is_occluded
                    || self
                        .window
                        .as_ref()
                        .is_some_and(|window| window.is_minimized() == Some(true));
                let redraw = if is_hidden {
                    Redraw::default()
                } else {
                    let redraw = match self.redraw(event_loop) {
                        Ok(redraw) => redraw,
                        Err(e) => {
                            log::error!("{e}");

                            Redraw::default()
                        },
                    };

                    if let (Some(platform), Some(imgui)) = (self.platform.as_ref(), self.imgui.as_mut()) {
                        viewports::finish_frame(platform, imgui, event_loop);
                    }

                    redraw
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
                if let Some(request) = redraw.screenshot {
                    self.request_screenshot(request);
                }
                if let Some(request) = redraw.open {
                    self.apply_open(request);
                }
                if let Some(profile) = redraw.reload_profile {
                    self.reload_profile(profile);
                }
                if let Some(id) = redraw.load_conflicts {
                    self.load_conflicts(id);
                }

                if let Some(source) = redraw.open_source {
                    self.open_source(source);
                }

                if let Some(outcome) = self.loader.poll() {
                    self.apply_outcome(outcome);
                }
                self.session.poll_bake();
                self.session.sync_open_modules();
                self.session.poll_git();
                self.session.poll_coop();
                let is_busy = self.is_busy();
                if was_busy && !is_busy {
                    self.pacer.activity(Instant::now());
                }

                if redraw.exit {
                    if let Err(e) = self.shutdown() {
                        log::error!("{e}");
                    }
                    event_loop.exit();
                } else {
                    // a job that finished during this frame still needs one more frame to show
                    let status = Status {
                        is_busy: was_busy || is_busy,
                        is_hidden,
                    };
                    self.schedule_frame(event_loop, frame_start, redraw.demand, status);
                }
            },

            _ => {},
        }
    }
}
