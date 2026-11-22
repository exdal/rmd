use std::{cmp::Reverse, collections::BTreeMap};

use base::coord::Coord;
use dear_imgui_rs::{PopupQueryFlags, Ui};
use prefab::Prefab;

use self::{
    candidate::{ActionCandidate, AreaCandidate, PrefabCandidate, build_actions, build_areas, build_prefabs},
    command::Command,
    matcher::{Query, score},
};
use super::{OpenRequest, ScreenshotRequest, UiState, menu::MenuActions};
use crate::{
    search::SearchQuery,
    session::Session,
    settings::{Settings, keybind::KeybindAction},
};

mod candidate;
mod command;
mod draw;
mod matcher;

const POPUP: &str = "Command palette##command-palette";
const MAX_RESULTS: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Prefabs,
    Actions,
    Areas,
}

#[derive(Debug, Clone, PartialEq)]
enum Choice {
    Prefab(Prefab),
    Command(Command),
    Area(Prefab),
}

enum Outcome {
    Open,
    Close,
    Choose(Choice),
}

pub(crate) struct CommandPalette {
    prefabs: Vec<PrefabCandidate>,
    areas: Vec<AreaCandidate>,
    actions: Vec<ActionCandidate>,
    query: String,
    ranked: Option<String>,
    mode: Mode,
    results: Vec<usize>,
    selected: usize,
    is_scroll_pending: bool,
}

impl CommandPalette {
    fn new(session: &Session) -> Self {
        let placed = session
            .state
            .active()
            .map(|document| session.placed_prefabs(document))
            .unwrap_or_default();
        let mut palette = Self {
            prefabs: build_prefabs(session, &placed),
            areas: build_areas(session, &placed),
            actions: build_actions(session),
            query: String::new(),
            ranked: None,
            mode: Mode::Prefabs,
            results: Vec::new(),
            selected: 0,
            is_scroll_pending: false,
        };
        palette.rank("");

        palette
    }

    fn rank(&mut self, query: &str) {
        if self.ranked.as_deref() == Some(query) {
            return;
        }

        self.ranked = Some(query.to_owned());
        self.selected = 0;
        self.is_scroll_pending = true;

        let (mode, text) = split_mode(query);
        let query = Query::parse(text);
        self.mode = mode;
        self.results = match mode {
            Mode::Prefabs => rank_prefabs(&self.prefabs, &query),
            Mode::Actions => rank_actions(&self.actions, &query),
            Mode::Areas => rank_areas(&self.areas, &query),
        };
        self.results.truncate(MAX_RESULTS);
    }

    fn choice(&self, row: usize) -> Option<Choice> {
        let index = *self.results.get(row)?;

        Some(match self.mode {
            Mode::Prefabs => Choice::Prefab(self.prefabs[index].prefab.clone()),
            Mode::Actions => Choice::Command(self.actions[index].command),
            Mode::Areas => Choice::Area(self.areas[index].prefab.clone()),
        })
    }
}

fn split_mode(query: &str) -> (Mode, &str) {
    if let Some(rest) = query.strip_prefix('>') {
        (Mode::Actions, rest.trim_start())
    } else if let Some(rest) = query.strip_prefix('@') {
        (Mode::Areas, rest.trim_start())
    } else {
        (Mode::Prefabs, query)
    }
}

fn rank_prefabs(prefabs: &[PrefabCandidate], query: &Query) -> Vec<usize> {
    if query.is_empty() {
        let mut recents = prefabs
            .iter()
            .enumerate()
            .filter_map(|(index, candidate)| Some((candidate.recent_rank?, index)))
            .collect::<Vec<_>>();
        recents.sort_unstable();

        let mut used = prefabs
            .iter()
            .enumerate()
            .filter(|(_, candidate)| candidate.recent_rank.is_none() && candidate.count > 0)
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        used.sort_by(|&a, &b| {
            prefabs[b]
                .count
                .cmp(&prefabs[a].count)
                .then_with(|| prefabs[a].path.cmp(&prefabs[b].path))
        });

        return recents
            .into_iter()
            .map(|(_, index)| index)
            .chain(used)
            .collect::<Vec<_>>();
    }

    let mut scored = prefabs
        .iter()
        .enumerate()
        .filter_map(|(index, candidate)| Some((index, score(query, &candidate.haystack)?)))
        .collect::<Vec<_>>();
    scored.sort_by(|&(a, a_score), &(b, b_score)| {
        let (a, b) = (&prefabs[a], &prefabs[b]);

        b_score
            .cmp(&a_score)
            .then_with(|| b.recent_rank.is_some().cmp(&a.recent_rank.is_some()))
            .then_with(|| b.count.cmp(&a.count))
            .then_with(|| a.path.len().cmp(&b.path.len()))
            .then_with(|| a.path.cmp(&b.path))
    });

    scored.into_iter().map(|(index, _)| index).collect::<Vec<_>>()
}

fn rank_areas(areas: &[AreaCandidate], query: &Query) -> Vec<usize> {
    if query.is_empty() {
        let mut all = (0..areas.len()).collect::<Vec<_>>();
        all.sort_by(|&a, &b| areas[a].name.cmp(&areas[b].name));

        return all;
    }

    let mut scored = areas
        .iter()
        .enumerate()
        .filter_map(|(index, candidate)| Some((index, score(query, &candidate.haystack)?)))
        .collect::<Vec<_>>();
    scored
        .sort_by(|&(a, a_score), &(b, b_score)| b_score.cmp(&a_score).then_with(|| areas[a].name.cmp(&areas[b].name)));

    scored.into_iter().map(|(index, _)| index).collect::<Vec<_>>()
}

fn rank_actions(actions: &[ActionCandidate], query: &Query) -> Vec<usize> {
    if query.is_empty() {
        return (0..actions.len()).collect::<Vec<_>>();
    }

    let mut scored = actions
        .iter()
        .enumerate()
        .filter_map(|(index, candidate)| Some((index, score(query, &candidate.haystack)?)))
        .collect::<Vec<_>>();
    scored.sort_by(|&(a, a_score), &(b, b_score)| {
        b_score
            .cmp(&a_score)
            .then_with(|| actions[a].command.label().cmp(actions[b].command.label()))
    });

    scored.into_iter().map(|(index, _)| index).collect::<Vec<_>>()
}

impl UiState {
    pub(super) fn draw_command_palette(
        &mut self, ui: &Ui, session: &mut Session, settings: &Settings, loading: bool, menu: &mut MenuActions,
    ) {
        let is_key_open = settings.keybindings.get(KeybindAction::CommandPalette).is_pressed(ui)
            && !ui.io().want_text_input()
            && !self.settings_window.is_capturing_keybind()
            && !ui.is_popup_open_with_flags("", PopupQueryFlags::ANY_POPUP);
        if !loading && (is_key_open || menu.command_palette) {
            self.command_palette = Some(CommandPalette::new(session));
            ui.open_popup(POPUP);
        }

        if !ui.is_popup_open(POPUP) {
            self.command_palette = None;
            return;
        }

        let Some(palette) = self.command_palette.as_mut() else {
            return;
        };

        match palette.draw(ui, session, settings) {
            Outcome::Open => {},
            Outcome::Close => self.command_palette = None,
            Outcome::Choose(choice) => {
                self.command_palette = None;
                self.apply_choice(session, menu, choice);
            },
        }
    }

    fn apply_choice(&mut self, session: &mut Session, menu: &mut MenuActions, choice: Choice) {
        match choice {
            Choice::Prefab(prefab) => session.choose_prefab(prefab),
            Choice::Command(command) => self.apply_command(session, menu, command),
            Choice::Area(prefab) => self.jump_to_area(session, prefab),
        }
    }

    fn apply_command(&mut self, session: &mut Session, menu: &mut MenuActions, command: Command) {
        match command {
            Command::Tool(tool) => session.set_tool(tool),
            Command::Layer(layer) => {
                session.toggle_layer(layer);
            },
            Command::ShowAllLayers => {
                session.show_all_types();
            },
            Command::Save => self.save_requested = true,
            Command::Settings => self.settings_window.open(),
            Command::Screenshot { area, copy } => menu.screenshot = Some(ScreenshotRequest { area, copy }),
            Command::Edit(edit) => menu.edit = Some(edit),
            Command::ShowPanel(panel) => menu.show_panel = Some(panel),
            Command::SaveAll => menu.save_all = true,
            Command::CloseMap => menu.close_map = true,
            Command::CloseAll => menu.close_all = true,
            Command::Undo => menu.undo = true,
            Command::Redo => menu.redo = true,
            Command::Find => menu.search = true,
            Command::GoTo => menu.go_to = true,
            Command::ResizeMap => menu.resize_map = true,
            Command::NewMap => menu.new_map = true,
            Command::OpenMap => menu.open = Some(OpenRequest::PickMap),
            Command::OpenCodebase => menu.open = Some(OpenRequest::PickCodebase),
            Command::ToggleAreas => menu.toggle_areas = true,
            Command::ToggleAreaOutlines => menu.toggle_area_outlines = true,
            Command::ToggleLighting => menu.toggle_lighting = true,
            Command::ToggleTileGrid => menu.toggle_tile_grid = true,
            Command::TogglePixelGrid => menu.toggle_pixel_grid = true,
            Command::ToggleMirrorCamera => menu.toggle_mirror_camera = true,
            Command::LevelUp => menu.level_delta += 1,
            Command::LevelDown => menu.level_delta -= 1,
            Command::Refit => menu.refit = true,
            Command::ToggleDmUi => menu.toggle_dm_ui = true,
            Command::ResetLayout => menu.reset_layout = true,
            Command::Welcome => menu.show_welcome = true,
            Command::About => menu.show_about = true,
        }
    }

    fn jump_to_area(&mut self, session: &mut Session, prefab: Prefab) {
        let Some(document) = session.state.active() else {
            return;
        };

        let coords = session
            .find_instances(document, &SearchQuery::Prefab(prefab), None)
            .into_iter()
            .filter_map(|instance| Some(session.state.document(document)?.instance_location(instance)?.coord))
            .collect::<Vec<_>>();
        let Some(index) = representative_tile(&coords) else {
            return;
        };

        self.cancel_edit_gestures(session, Some(document));
        self.center_view_on(session, document, coords[index]);
    }
}

pub(crate) fn representative_tile(coords: &[Coord]) -> Option<usize> {
    let mut counts = BTreeMap::<u32, usize>::new();
    for coord in coords {
        *counts.entry(coord.z).or_default() += 1;
    }

    let (&z, _) = counts.iter().max_by_key(|&(&z, &count)| (count, Reverse(z)))?;

    let on_level = coords.iter().filter(|coord| coord.z == z);
    let count = on_level.clone().count() as f64;
    let center_x = on_level.clone().map(|coord| f64::from(coord.x)).sum::<f64>() / count;
    let center_y = on_level.map(|coord| f64::from(coord.y)).sum::<f64>() / count;

    let distance = |coord: &Coord| (f64::from(coord.x) - center_x).powi(2) + (f64::from(coord.y) - center_y).powi(2);

    coords
        .iter()
        .enumerate()
        .filter(|(_, coord)| coord.z == z)
        .min_by(|(_, a), (_, b)| distance(a).total_cmp(&distance(b)))
        .map(|(index, _)| index)
}

#[cfg(test)]
mod tests;
