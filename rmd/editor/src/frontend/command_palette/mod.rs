use dear_imgui_rs::{PopupQueryFlags, Ui};
use prefab::Prefab;

use self::{
    candidate::{PrefabCandidate, build_prefabs},
    matcher::{Query, score},
};
use super::{UiState, menu::MenuActions};
use crate::{
    session::Session,
    settings::{Settings, keybind::KeybindAction},
};

mod candidate;
mod draw;
mod matcher;

const POPUP: &str = "Command palette##command-palette";
const MAX_RESULTS: usize = 100;

enum Outcome {
    Open,
    Close,
    Choose(Prefab),
}

pub(crate) struct CommandPalette {
    prefabs: Vec<PrefabCandidate>,
    query: String,
    ranked: Option<String>,
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
            query: String::new(),
            ranked: None,
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

        self.results = rank_prefabs(&self.prefabs, &Query::parse(query));
        self.results.truncate(MAX_RESULTS);
    }

    fn choice(&self, row: usize) -> Option<Prefab> {
        let index = *self.results.get(row)?;

        Some(self.prefabs[index].prefab.clone())
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

impl UiState {
    pub(super) fn draw_command_palette(
        &mut self, ui: &Ui, session: &mut Session, settings: &Settings, loading: bool, menu: &MenuActions,
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
            Outcome::Choose(prefab) => {
                self.command_palette = None;
                session.choose_prefab(prefab);
            },
        }
    }
}

#[cfg(test)]
mod tests;
