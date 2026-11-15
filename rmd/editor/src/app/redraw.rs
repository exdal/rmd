use std::time::Instant;

use dear_imgui_rs::Context;
use render::{MapViewFrame, PickRequest, PickResult, Renderer};
use ui::{fonts::UiFont, theme::Themes, viewports};
use winit::{event_loop::ActiveEventLoop, window::Window};

use super::screenshot::take_screenshot;
use super::{
    App, Redraw,
    pacing::{FrameDemand, FramePacer},
    style_base, window_title,
};
use crate::{
    document::DocumentId,
    environment::worker::{Job, Loader},
    frontend::{UiOutput, UiState},
    session::Session,
    settings::{Settings, keybind::KeybindPreset},
};

impl App {
    pub(super) fn redraw(&mut self, event_loop: &ActiveEventLoop) -> Result<Redraw, Box<dyn std::error::Error>> {
        let Self {
            session,
            settings,
            ui,
            loader,
            deferred_job,
            settings_ready,
            uploaded_texture_revision,
            pending_screenshot,
            clipboard,
            title,
            consumer,
            renderer,
            platform,
            imgui,
            scaled_style,
            themes,
            ui_font,
            window,
            pacer,
            ..
        } = self;
        let (Some(consumer), Some(renderer), Some(platform), Some(imgui), Some(themes), Some(ui_font), Some(window)) = (
            consumer.as_ref(),
            renderer.as_mut(),
            platform.as_mut(),
            imgui.as_mut(),
            themes.as_mut(),
            ui_font.as_mut(),
            window.as_ref(),
        ) else {
            return Ok(Redraw::default());
        };

        sync_title_and_textures(session, ui, window, title, renderer, uploaded_texture_revision);

        sync_fonts_and_style(settings, imgui, ui_font, scaled_style, themes, window);

        platform.prepare_frame(imgui, window)?;
        let frame = imgui.try_begin_frame()?;
        let load = loader.view().or_else(|| session.bake_view());
        let output = ui.draw(frame.ui(), session, settings, themes, load.as_ref())?;
        if let Some(preset) = output.keybind_preset {
            apply_keybind_preset(settings, settings_ready, deferred_job, loader, preset);
        }
        platform.prepare_render(frame.ui(), window)?;
        let (drawn, map_views, picking) = collect_map_views(session, &output);
        let mut demand = output.demand;
        if map_views.iter().any(|view| view.is_animated()) {
            demand.raise(FrameDemand::Throttled);
        }

        let scene = session.frame(&map_views, picking);
        let pending = frame.try_render(consumer)?;
        window.pre_present_notify();
        let picked = renderer.draw_imgui(&scene, pending, |reconciled| {
            if !platform.viewports_enabled() {
                return Vec::new();
            }

            let (collected, faults) = platform
                .with_event_loop(event_loop, |_| viewports::collect(reconciled))
                .into_parts();
            for fault in faults {
                log::error!("{fault}");
            }

            collected.unwrap_or_default()
        })?;
        settle_appearing_windows(imgui, pacer, Instant::now());
        if let Some(pending) = pending_screenshot.take() {
            take_screenshot(renderer, session, &scene, &drawn, pending, clipboard);
        }
        if let Some((index, request, pick)) = picked {
            apply_pick(ui, session, &drawn, index, request, pick);
        }

        Ok(Redraw {
            exit: output.exit,
            open: output.open,
            open_source: output.open_source,
            pick_new_map_path: output.pick_new_map_path,
            screenshot: output.screenshot,
            cancel_load: output.cancel_load,
            copy_to_clipboard: output.copy_to_clipboard,
            reload_profile: output.reload_profile,
            load_conflicts: output.load_conflicts,
            demand,
        })
    }
}

fn sync_title_and_textures(
    session: &Session, ui: &mut UiState, window: &Window, title: &mut String, renderer: &mut Renderer,
    uploaded_texture_revision: &mut Option<u64>,
) {
    let current = window_title(session);
    if *title != current {
        window.set_title(&current);
        *title = current;
    }

    if *uploaded_texture_revision != Some(session.texture_revision()) {
        *uploaded_texture_revision = Some(session.texture_revision());
        if let Err(e) = renderer.set_textures(&session.textures) {
            log::error!("{e}");
            ui.set_open_error(Some(e.to_string()));
        }
    }
}

fn sync_fonts_and_style(
    settings: &mut Settings, imgui: &mut Context, ui_font: &mut UiFont,
    scaled_style: &mut Option<viewports::ScaledStyle>, themes: &Themes, window: &Window,
) {
    ui_font.sync(imgui.font_atlas(), &mut settings.font);
    if let Some(scaled_style) = scaled_style.as_mut() {
        scaled_style.apply(
            imgui,
            &style_base(themes, settings),
            window.scale_factor() as f32,
            settings.ui_scale_override_percent,
        );
    }
}

fn apply_keybind_preset(
    settings: &mut Settings, settings_ready: &mut bool, deferred_job: &mut Option<Job>, loader: &mut Loader,
    preset: KeybindPreset,
) {
    settings.keybindings = preset.bindings();
    settings.save();
    *settings_ready = true;
    if let Some(job) = deferred_job.take() {
        loader.start(job);
    }
}

fn collect_map_views<'a>(
    session: &'a Session, output: &'a UiOutput,
) -> (Vec<DocumentId>, Vec<MapViewFrame<'a>>, Option<usize>) {
    let mut drawn = Vec::with_capacity(output.map_views.len());
    let mut map_views = Vec::with_capacity(output.map_views.len());
    let mut picking = None;
    for (index, view) in output.map_views.iter().enumerate() {
        let Some(frame) = session.map_view_frame(
            view.document,
            view.rect,
            view.camera,
            view.interaction,
            &view.guide_lines,
            &view.connected,
            &view.picked,
        ) else {
            continue;
        };
        if output.picking == Some(index) {
            picking = Some(map_views.len());
        }
        drawn.push(view.document);
        map_views.push(frame);
    }
    (drawn, map_views, picking)
}

fn apply_pick(
    ui: &mut UiState, session: &mut Session, drawn: &[DocumentId], index: usize, request: PickRequest, pick: PickResult,
) {
    if let Some(document) = drawn.get(index).copied() {
        session.set_active_document(document);
    }
    match request {
        PickRequest::Pick => match pick {
            PickResult::Hit(owner) => {
                session.pick_instance(Some(owner));
                ui.reveal_picked_instance(session);
                ui.request_mouse_popup();
            },
            PickResult::Miss => session.pick_instance(None),
        },
        PickRequest::Toggle => {
            if let PickResult::Hit(owner) = pick {
                session.toggle_pick(owner);
                ui.reveal_picked_instance(session);
            }
        },
        PickRequest::Move => match pick {
            PickResult::Hit(owner) => {
                session.pick_instance(Some(owner));
                ui.reveal_picked_instance(session);
            },
            PickResult::Miss => session.pick_instance(None),
        },
        PickRequest::Delete => {
            if let PickResult::Hit(owner) = pick {
                session.delete_instance(owner);
            }
        },
        PickRequest::Replace => {
            if let PickResult::Hit(owner) = pick {
                session.replace_instance(owner);
            }
        },
        PickRequest::NodeSelect => {
            session.select_node_from_pick(pick);
        },
        PickRequest::NodeSeed(coord) => {
            let picked = match pick {
                PickResult::Hit(owner) => Some(owner),
                PickResult::Miss => None,
            };
            if let Some(target) = session.node_candidate_from_pick(picked, coord) {
                session.begin_node_edit(target);
            }
        },
        PickRequest::NodeDelete(coord) => {
            let picked = match pick {
                PickResult::Hit(owner) => Some(owner),
                PickResult::Miss => None,
            };
            if let Some(connection) = session.node_connection_from_pick(picked, coord) {
                session.delete_node_connection(&connection);
            }
        },
    }
}

pub(super) fn settle_appearing_windows(imgui: &Context, pacer: &mut FramePacer, now: Instant) {
    let is_appearing = unsafe {
        let windows = &(*imgui.as_raw()).Windows;
        (0..windows.Size).any(|index| {
            let window = &**windows.Data.add(index as usize);
            window.Active && window.Appearing
        })
    };
    if is_appearing {
        pacer.activity(now);
    }
}
