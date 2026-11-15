use dear_imgui_rs::{TableFlags, TableSizingPolicy, Ui, WindowFlags};
use ui::widgets::{DialogButton, begin_centered_modal, dialog_buttons};

use crate::{environment::profile::profile_label, frontend::ProfileReload, session::Session, settings::Settings};

use super::draw_section_heading;

pub(super) const PROFILE_RELOAD_POPUP: &str = "Reload codebase profile?";

pub(super) fn draw_compiler_settings(
    ui: &Ui, session: &Session, settings: &mut Settings, loading: bool, profiles: &[String],
    pending_profile: &mut Option<ProfileReload>,
) {
    draw_section_heading(ui, "Codebase");
    draw_profile_setting(ui, session, settings, loading, profiles, pending_profile);

    ui.separator();
    draw_optimization_settings(ui, session, settings);

    ui.separator();
    draw_section_heading(ui, "On next load");
    ui.checkbox("Run DM appearance baking", &mut settings.bake_enabled);

    ui.separator();
    draw_section_heading(ui, "Diagnostics");

    let retained = session.diagnostics.bake.iter().map(|entry| entry.count).sum::<usize>();
    ui.text(format!("{retained} atoms retained their static appearance"));
}

fn draw_optimization_settings(ui: &Ui, session: &Session, settings: &mut Settings) {
    draw_section_heading(ui, "Optimization");
    ui.checkbox("Enable compiler optimizations", &mut settings.optimizations_enabled);

    let Some(environment) = session.state.environment.as_deref() else {
        ui.text_disabled("Pass timings are available after loading a codebase.");

        return;
    };
    let timings = environment.optimization_timings;
    let samples = timings.samples();
    if samples == 0 {
        ui.text_disabled("No pass timing data for this codebase.");

        return;
    }

    let run_label = if samples == 1 { "run" } else { "runs" };
    ui.text_disabled(format!(
        "Last load: average pass time across {samples} compiler {run_label}"
    ));
    ui.table("compiler-optimization-timings")
        .flags(TableFlags::BORDERS_INNER_V | TableFlags::ROW_BG)
        .sizing_policy(TableSizingPolicy::StretchProp)
        .column("Pass")
        .weight(1.0)
        .done()
        .column("Average")
        .width(100.0)
        .done()
        .build(|ui| {
            for pass in ir::opt::OptimizationPass::ALL {
                if !environment.bake_options.optimizations_enabled && pass != ir::opt::OptimizationPass::SimplifyPhis {
                    continue;
                }
                ui.table_next_row();
                ui.table_next_column();
                ui.text(pass.label());
                ui.table_next_column();
                ui.text(format_pass_time(timings.average(pass)));
            }

            ui.table_next_row();
            ui.table_next_column();
            ui.text("Total");
            ui.table_next_column();
            ui.text(format_pass_time(timings.average_total()));
        });
}

fn format_pass_time(duration: std::time::Duration) -> String {
    format!("{:.3} ms", duration.as_secs_f64() * 1_000.0)
}

fn draw_profile_setting(
    ui: &Ui, session: &Session, settings: &Settings, loading: bool, profiles: &[String],
    pending_profile: &mut Option<ProfileReload>,
) {
    let Some(environment) = session.state.environment.as_deref() else {
        ui.text_disabled("No codebase loaded");

        return;
    };
    if !settings.bake_enabled || !environment.bake_options.enabled {
        ui.text_disabled("DM appearance baking is disabled");

        return;
    }

    {
        let _disabled = ui.begin_disabled_with_cond(loading);
        if ui.button("Reload profile") {
            *pending_profile = Some(ProfileReload::Current);
        }

        ui.set_item_tooltip("Recompile the codebase and its profile. Open maps and unsaved changes are kept");
    }

    let forced = environment.bake_options.forced_profile.as_deref();
    ui.text("Forced profile");
    if let Ok(dir) = crate::settings::paths::profiles_dir() {
        ui.set_item_tooltip(format!("Profiles are read from {}", dir.display()));
    }
    {
        let _disabled = ui.begin_disabled_with_cond(loading);
        let preview = forced.map(profile_label).unwrap_or("None - use codebase profile");
        ui.set_next_item_width(-1.0);
        if let Some(combo) = ui.begin_combo("##forced-profile", preview) {
            if ui
                .selectable_config("None - use codebase profile")
                .selected(forced.is_none())
                .build()
                && forced.is_some()
            {
                *pending_profile = Some(ProfileReload::Force(None));
            }
            for profile in profiles {
                let selected = forced == Some(profile.as_str());
                if ui
                    .selectable_config(format!("{}##{profile}", profile_label(profile)))
                    .selected(selected)
                    .build()
                    && !selected
                {
                    *pending_profile = Some(ProfileReload::Force(Some(profile.clone())));
                }
            }
            combo.end();
        }
    }
    if loading {
        ui.text_disabled("Available after the current load finishes");
    }

    ui.text("Profile");
    if forced.is_some() {
        ui.text_disabled("Codebase profile selection is disabled while a profile is forced");

        return;
    }
    let Some(profiles) = environment.profiles.as_ref() else {
        if session.diagnostics.profile.is_some() {
            ui.text_disabled("Profile declarations are invalid");
        } else {
            ui.text_disabled("This codebase defines no profiles");
        }

        return;
    };

    let _disabled = ui.begin_disabled_with_cond(loading);
    ui.set_next_item_width(-1.0);
    if let Some(combo) = ui.begin_combo("##codebase-profile", &profiles.active) {
        for profile in &profiles.available {
            let label = if *profile == profiles.default {
                format!("{profile} (default)")
            } else {
                profile.clone()
            };
            if ui
                .selectable_config(&label)
                .selected(*profile == profiles.active)
                .build()
                && *profile != profiles.active
            {
                *pending_profile = Some(ProfileReload::Select(profile.clone()));
            }
        }
        combo.end();
    }
    if loading {
        ui.text_disabled("Available after the current load finishes");
    }
}

pub(super) fn draw_profile_reload_dialog(
    ui: &Ui, pending_profile: &mut Option<ProfileReload>,
) -> Option<ProfileReload> {
    let message = match pending_profile.as_ref()? {
        ProfileReload::Current => return pending_profile.take(),
        ProfileReload::Select(profile) => format!("Reload the codebase with {profile}?"),
        ProfileReload::Force(Some(profile)) => {
            format!("Reload the codebase with the {} profile?", profile_label(profile))
        },
        ProfileReload::Force(None) => String::from("Stop forcing a profile and reload the codebase?"),
    };
    if !ui.is_popup_open(PROFILE_RELOAD_POPUP) {
        ui.open_popup(PROFILE_RELOAD_POPUP);
    }
    let flags = WindowFlags::ALWAYS_AUTO_RESIZE
        | WindowFlags::NO_RESIZE
        | WindowFlags::NO_MOVE
        | WindowFlags::NO_COLLAPSE
        | WindowFlags::NO_SAVED_SETTINGS
        | WindowFlags::NO_DOCKING;
    let _modal = begin_centered_modal(ui, PROFILE_RELOAD_POPUP, flags)?;

    ui.text_wrapped(message);
    ui.text("Open maps and unsaved changes will be preserved.");

    let pressed = dialog_buttons(
        ui,
        &[DialogButton::cancel("Cancel"), DialogButton::primary("Reload Codebase")],
        true,
    );
    if pressed == Some(0) {
        *pending_profile = None;
        ui.close_current_popup();

        return None;
    }

    if pressed == Some(1) {
        let request = pending_profile.take();
        ui.close_current_popup();

        return request;
    }

    None
}
