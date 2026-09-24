use std::path::{Path, PathBuf};

use dear_imgui_rs::{MouseButton, StyleColor, Ui};
use editor::icons::materialdesignicons::{ICON_ALERT, ICON_ALERT_CIRCLE, ICON_CLOSE_THICK};

use super::{DIAGNOSTIC_WARNING_COLOR, OpenRequest, SAVE_ERROR_COLOR, UiState, common::focus_window_on_hover};
use crate::{
    session::{DiagnosticSeverity, Session},
    settings::Settings,
};

const WELCOME_TITLE_SIZE: f32 = 40.0;

const WELCOME_CONTENT_WIDTH: f32 = 640.0;

const WELCOME_MIN_INDENT: f32 = 24.0;

const WELCOME_MAP_PREVIEW: usize = 10;

const BUILD_VERSION: &str = concat!("v", env!("CARGO_PKG_VERSION"));

const BUILD_GIT_SHORT_HASH: Option<&str> = option_env!("RMD_GIT_SHORT_HASH");

const BUILD_VERSION_URL: Option<&str> = option_env!("RMD_VERSION_URL");

const BUILD_COMMIT_URL: Option<&str> = option_env!("RMD_COMMIT_URL");

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
    forget: bool,
}

fn draw_welcome_subtitle(ui: &Ui) {
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
    ui.text_disabled("A map editor for BYOND");
}

fn draw_recent_entry(ui: &Ui, label: &str, id: &str) -> RecentEntry {
    let icon = ICON_CLOSE_THICK.to_string();
    let icon_width = ui.calc_text_size(&icon)[0];

    let open = ui.text_link(format!("{label}##{id}"));
    let link_min = ui.item_rect_min();
    let link_max = ui.item_rect_max();
    let icon_min = [link_max[0] + ui.clone_style().item_spacing()[0], link_min[1]];
    let icon_max = [icon_min[0] + icon_width, link_max[1]];
    let mut forget = false;

    if ui.is_mouse_hovering_rect(link_min, icon_max) {
        let hovered = ui.is_mouse_hovering_rect(icon_min, icon_max);
        let color = if hovered {
            StyleColor::Text
        } else {
            StyleColor::TextDisabled
        };

        ui.get_window_draw_list()
            .add_text([icon_min[0], icon_min[1] + 2.0], ui.style_color(color), &icon);

        if hovered {
            ui.tooltip_text("Remove from this list");
            forget = ui.is_mouse_clicked(MouseButton::Left);
        }
    }

    RecentEntry { open, forget }
}

fn map_matches(base: &Path, map: &Path, needle: &str) -> bool {
    needle.is_empty() || codebase_relative(base, map).to_ascii_lowercase().contains(needle)
}

fn codebase_relative(base: &Path, path: &Path) -> String {
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
        ui.window(&self.welcome_window).opened(show_welcome).build(|| {
            if settings.focus_windows_on_hover {
                focus_window_on_hover(ui);
            }
            let dock = ui.get_window_dock_id();
            if dock.raw() != 0 {
                *central_node = Some(dock);
            }

            let indent = ((ui.content_region_avail()[0] - WELCOME_CONTENT_WIDTH) / 2.0).max(WELCOME_MIN_INDENT);
            ui.dummy([0.0, WELCOME_MIN_INDENT]);
            ui.indent_by(indent);

            {
                let _font = ui.push_font_with_size(None, WELCOME_TITLE_SIZE);
                ui.text("Rapid Mapping Device");
            }

            match codebase {
                Some(codebase) => ui.text_disabled(codebase.display().to_string()),
                None => draw_welcome_subtitle(ui),
            }

            ui.dummy([0.0, WELCOME_MIN_INDENT]);

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
                ui.dummy([0.0, WELCOME_MIN_INDENT]);
            }

            let _disabled = ui.begin_disabled_with_cond(loading);
            match codebase {
                None => {
                    ui.text("Start");
                    if ui.text_link("Open codebase...") {
                        out.open = Some(OpenRequest::PickCodebase);
                    }

                    ui.dummy([0.0, WELCOME_MIN_INDENT]);
                    ui.text("Recent codebases");
                    if settings.recent_codebases.is_empty() {
                        ui.text_disabled("No recent codebases");
                    }
                    for (index, recent) in settings.recent_codebases.iter().enumerate() {
                        let entry = draw_recent_entry(ui, &recent.display().to_string(), &format!("codebase-{index}"));
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

                    ui.dummy([0.0, WELCOME_MIN_INDENT]);
                    ui.text("Recent maps");
                    let mut empty = true;
                    for (index, recent) in settings.recent_maps_for(codebase).enumerate() {
                        empty = false;
                        let label = codebase_relative(base, &recent.map);
                        let entry = draw_recent_entry(ui, &label, &format!("recent-{index}"));
                        if entry.open {
                            out.open = Some(OpenRequest::Map(recent.map.clone()));
                        }
                        if entry.forget {
                            out.forget = Some(ForgetRequest::Map(recent.map.clone()));
                        }
                    }

                    if empty {
                        ui.text_disabled("No recent maps in this codebase");
                    }

                    ui.dummy([0.0, WELCOME_MIN_INDENT]);
                    ui.text("Maps");
                    if session.maps().is_empty() {
                        ui.text_disabled("No maps found in this codebase");
                    } else {
                        ui.set_next_item_width(WELCOME_CONTENT_WIDTH);
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
                        if ui.text_link(format!("{}##map-{index}", codebase_relative(base, map))) {
                            out.open = Some(OpenRequest::Map(map.clone()));
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
