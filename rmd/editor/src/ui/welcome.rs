use std::path::{Path, PathBuf};

use dear_imgui_rs::{StyleColor, Ui};
use editor::icons::materialdesignicons::{ICON_ALERT, ICON_ALERT_CIRCLE, ICON_CLOSE_THICK, ICON_WEB};

use super::{
    DIAGNOSTIC_WARNING_COLOR,
    OpenRequest,
    SAVE_ERROR_COLOR,
    UiState,
    common::{dpi, focus_window_on_hover},
    coop::shared_maps,
};
use crate::{
    session::{DiagnosticSeverity, Session},
    settings::Settings,
    update::Release,
};

const WELCOME_TITLE_SIZE: f32 = 40.0;

const WELCOME_CONTENT_WIDTH: f32 = 640.0;

const WELCOME_MIN_INDENT: f32 = 24.0;

const WELCOME_MAP_PREVIEW: usize = 10;

const BUILD_VERSION: &str = concat!("v", env!("CARGO_PKG_VERSION"));

const BUILD_GIT_SHORT_HASH: Option<&str> = option_env!("RMD_GIT_SHORT_HASH");

const BUILD_VERSION_URL: Option<&str> = option_env!("RMD_VERSION_URL");

const BUILD_COMMIT_URL: Option<&str> = option_env!("RMD_COMMIT_URL");

const UPDATE_AVAILABLE_COLOR: [f32; 4] = [0.3, 0.68, 0.36, 1.0];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ForgetRequest {
    Codebase(PathBuf),
    Map(PathBuf),
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct WelcomeOutput {
    pub(super) open: Option<OpenRequest>,
    pub(super) new_map_dialog: bool,
    pub(super) forget: Option<ForgetRequest>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct RecentEntry {
    open: bool,
    share: bool,
    forget: bool,
}

fn draw_welcome_subtitle(ui: &Ui, release: Option<&Release>) {
    if let Some(release) = release {
        let _color = ui.push_style_color(StyleColor::TextLink, UPDATE_AVAILABLE_COLOR);
        ui.text_link_open_url(format!("{} available", release.version), &release.url);
        ui.same_line();
        ui.text_disabled("\u{2022}");
        ui.same_line();
    }

    match BUILD_VERSION_URL {
        Some(url) => {
            ui.text_link_open_url(BUILD_VERSION, url);
        },
        None => ui.text_disabled(BUILD_VERSION),
    }

    ui.same_line();
    ui.text_disabled("\u{2022}");
    ui.same_line();

    let hash = BUILD_GIT_SHORT_HASH.unwrap_or("unknown");
    match (BUILD_GIT_SHORT_HASH, BUILD_COMMIT_URL) {
        (Some(_), Some(url)) => {
            ui.text_link_open_url(hash, url);
        },
        _ => ui.text_disabled(hash),
    }

    ui.same_line();
    ui.text_disabled("\u{2022}");
    ui.same_line();
    ui.text_disabled("A map editor for Space Station 13");
}

fn draw_recent_entry(ui: &Ui, label: &str, id: &str, share: Option<&str>, forget: bool) -> RecentEntry {
    let spacing = ui.clone_style().item_spacing()[0];
    let mut entry = RecentEntry {
        open: ui.text_link(format!("{label}##{id}")),
        ..RecentEntry::default()
    };

    let mut hovered = ui.is_item_hovered();
    let height = ui.item_rect_size()[1];
    let buttons = [
        share.map(|tooltip| (ICON_WEB, "share", tooltip, &mut entry.share)),
        forget.then_some((ICON_CLOSE_THICK, "forget", "Remove from this list", &mut entry.forget)),
    ];

    let mut icons = Vec::new();
    for (icon, name, tooltip, clicked) in buttons.into_iter().flatten() {
        let icon = icon.to_string();
        // a real item spanning the gap and the icon, so a disabled scope locks it like any widget
        ui.same_line_with_spacing(0.0, 0.0);
        *clicked = ui.invisible_button(
            format!("##{name}-{id}"),
            [spacing + ui.calc_text_size(&icon)[0], height],
        );

        let icon_hovered = ui.is_item_hovered();
        if icon_hovered {
            ui.tooltip_text(tooltip);
        }

        hovered |= icon_hovered;
        let min = ui.item_rect_min();
        icons.push((icon, [min[0] + spacing, min[1] + 2.0], icon_hovered));
    }

    if hovered {
        let draw = ui.get_window_draw_list();
        for (icon, pos, icon_hovered) in icons {
            let color = if icon_hovered {
                StyleColor::Text
            } else {
                StyleColor::TextDisabled
            };

            draw.add_text(pos, ui.style_color(color), &icon);
        }
    }

    entry
}

fn map_matches(base: &Path, map: &Path, needle: &str) -> bool {
    needle.is_empty() || codebase_relative(base, map).to_ascii_lowercase().contains(needle)
}

fn share_tooltip(session: &Session, map: &Path) -> Option<&'static str> {
    if !session.can_share_coop_maps() {
        return None;
    }

    Some(if session.is_coop_shared_file(map) {
        "Open the shared map"
    } else {
        "Share in co-op"
    })
}

pub(super) fn codebase_relative(base: &Path, path: &Path) -> String {
    path.strip_prefix(base).unwrap_or(path).display().to_string()
}

impl UiState {
    pub(super) fn draw_welcome(
        &mut self, ui: &Ui, session: &Session, settings: &Settings, loading: bool, out: &mut WelcomeOutput,
    ) {
        if !self.show_welcome {
            return;
        }

        let show_welcome = &mut self.show_welcome;
        let central_node = &mut self.central_node;
        let map_filter = &mut self.welcome_map_filter;
        let maps_expanded = &mut self.welcome_maps_expanded;
        let diagnostics = &mut self.diagnostics;
        let codebase = session.environment_path();
        let release = if settings.check_for_updates {
            self.update_check.poll()
        } else {
            None
        };
        ui.window(&self.welcome_window).opened(show_welcome).build(|| {
            if settings.focus_windows_on_hover {
                focus_window_on_hover(ui);
            }
            let dock = ui.get_window_dock_id();
            if dock.raw() != 0 {
                *central_node = Some(dock);
            }

            // everything below stays locked until the codebase finishes loading
            let _disabled = ui.begin_disabled_with_cond(loading);
            let scale = dpi(ui);
            let min_indent = WELCOME_MIN_INDENT * scale;
            let indent = ((ui.content_region_avail()[0] - WELCOME_CONTENT_WIDTH * scale) / 2.0).max(min_indent);
            ui.dummy([0.0, min_indent]);
            ui.indent_by(indent);

            {
                let _font = ui.push_font_with_size(None, WELCOME_TITLE_SIZE);
                ui.text("Rapid Mapping Device");
            }

            match codebase {
                Some(codebase) => ui.text_disabled(codebase.display().to_string()),
                None => draw_welcome_subtitle(ui, release),
            }

            ui.dummy([0.0, min_indent]);

            let warnings = diagnostics.count(DiagnosticSeverity::Warning);
            if warnings > 0 {
                let _color = ui.push_style_color(StyleColor::TextLink, DIAGNOSTIC_WARNING_COLOR);
                let label = if warnings == 1 { "warning" } else { "warnings" };
                if ui.text_link(format!("{ICON_ALERT} {warnings} {label}##load-warnings")) {
                    diagnostics.open = Some(DiagnosticSeverity::Warning);
                }
            }
            let errors = diagnostics.count(DiagnosticSeverity::Error);
            if errors > 0 {
                if warnings > 0 {
                    ui.same_line();
                }
                let _color = ui.push_style_color(StyleColor::TextLink, SAVE_ERROR_COLOR);
                let label = if errors == 1 { "error" } else { "errors" };
                if ui.text_link(format!("{ICON_ALERT_CIRCLE} {errors} {label}##load-errors")) {
                    diagnostics.open = Some(DiagnosticSeverity::Error);
                }
            }
            if warnings + errors > 0 {
                ui.dummy([0.0, min_indent]);
            }

            match codebase {
                None => {
                    ui.text("Start");
                    if ui.text_link("Open codebase...") {
                        out.open = Some(OpenRequest::PickCodebase);
                    }

                    ui.dummy([0.0, min_indent]);
                    ui.text("Recent codebases");
                    if settings.recent_codebases.is_empty() {
                        ui.text_disabled("No recent codebases");
                    }
                    for (index, recent) in settings.recent_codebases.iter().enumerate() {
                        let entry = draw_recent_entry(
                            ui,
                            &recent.display().to_string(),
                            &format!("codebase-{index}"),
                            None,
                            true,
                        );
                        if entry.open {
                            out.open = Some(OpenRequest::Codebase(recent.clone()));
                        }
                        if entry.forget {
                            out.forget = Some(ForgetRequest::Codebase(recent.clone()));
                        }
                    }
                },

                Some(codebase) => {
                    let base = session.codebase_dir().unwrap_or(codebase);

                    ui.text("Start");
                    if ui.text_link("New map...") {
                        out.new_map_dialog = true;
                    }
                    if ui.text_link("Open map...") {
                        out.open = Some(OpenRequest::PickMap);
                    }

                    ui.dummy([0.0, min_indent]);
                    ui.text("Recent maps");
                    let mut empty = true;
                    for (index, recent) in settings.recent_maps_for(codebase).enumerate() {
                        empty = false;
                        let entry = draw_recent_entry(
                            ui,
                            &codebase_relative(base, &recent.map),
                            &format!("recent-{index}"),
                            share_tooltip(session, &recent.map),
                            true,
                        );
                        if entry.open {
                            out.open = Some(OpenRequest::Map(recent.map.clone()));
                        }

                        if entry.share {
                            out.open = Some(OpenRequest::ShareMap(recent.map.clone()));
                        }

                        if entry.forget {
                            out.forget = Some(ForgetRequest::Map(recent.map.clone()));
                        }
                    }

                    if empty {
                        ui.text_disabled("No recent maps in this codebase");
                    }

                    let shared = session.coop().map(shared_maps).unwrap_or_default();
                    if !shared.is_empty() {
                        ui.dummy([0.0, min_indent]);
                        ui.text("Shared maps");
                        for (index, (path, note)) in shared.iter().enumerate() {
                            if ui.text_link(format!("{path} {note}##shared-{index}")) {
                                out.open = Some(OpenRequest::Map(base.join(path)));
                            }
                        }
                    }

                    ui.dummy([0.0, min_indent]);
                    ui.text("Maps");
                    if session.maps().is_empty() {
                        ui.text_disabled("No maps found in this codebase");
                    } else {
                        ui.set_next_item_width(WELCOME_CONTENT_WIDTH * scale);
                        ui.input_text("##welcome-map-filter", map_filter)
                            .hint("Filter maps")
                            .build();
                    }

                    let needle = map_filter.trim().to_ascii_lowercase();
                    let matching = session
                        .maps()
                        .iter()
                        .enumerate()
                        .filter(|(_, map)| map_matches(base, map, &needle));
                    let limit = if *maps_expanded {
                        usize::MAX
                    } else {
                        WELCOME_MAP_PREVIEW
                    };

                    let mut shown = 0;
                    for (index, map) in matching.clone().take(limit) {
                        shown += 1;
                        let entry = draw_recent_entry(
                            ui,
                            &codebase_relative(base, map),
                            &format!("map-{index}"),
                            share_tooltip(session, map),
                            false,
                        );
                        if entry.open {
                            out.open = Some(OpenRequest::Map(map.clone()));
                        }

                        if entry.share {
                            out.open = Some(OpenRequest::ShareMap(map.clone()));
                        }
                    }

                    if shown == 0 && !session.maps().is_empty() {
                        ui.text_disabled("No maps match the filter");
                    }
                    let hidden = matching.count().saturating_sub(shown);
                    if hidden > 0 {
                        if ui.text_link(format!("Show more... ({hidden})")) {
                            *maps_expanded = true;
                        }
                    } else if *maps_expanded && shown > WELCOME_MAP_PREVIEW && ui.text_link("Show less") {
                        *maps_expanded = false;
                    }
                },
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{codebase_relative, map_matches};

    #[test]
    fn a_map_reads_as_its_path_inside_the_codebase() {
        let base = Path::new("/tg");
        let map = PathBuf::from("/tg/_maps/map_files/station.dmm");

        assert_eq!(
            codebase_relative(base, &map),
            Path::new("_maps/map_files/station.dmm").display().to_string()
        );
    }

    #[test]
    fn the_map_filter_matches_any_part_of_the_codebase_relative_path() {
        let base = Path::new("/tg");
        let map = Path::new("/tg/_maps/map_files/MetaStation/MetaStation.dmm");

        assert!(map_matches(base, map, ""), "an empty filter keeps everything");
        assert!(map_matches(base, map, "metastation"), "matching ignores case");
        assert!(map_matches(base, map, "map_files"), "a directory segment matches");
        assert!(!map_matches(base, map, "deltastation"));
    }

    #[test]
    fn the_map_filter_does_not_match_the_codebase_directory_itself() {
        // The label is relative, so a codebase living under a directory called "maps" must not make
        // every one of its maps match the word.
        let base = Path::new("/home/maps/tg");
        let map = Path::new("/home/maps/tg/station.dmm");

        assert!(!map_matches(base, map, "home"));
    }

    #[test]
    fn a_map_outside_the_codebase_keeps_its_full_path() {
        let outside = PathBuf::from("/elsewhere/station.dmm");

        assert_eq!(
            codebase_relative(Path::new("/tg"), &outside),
            outside.display().to_string()
        );
    }
}
