use std::collections::HashSet;

use dmm::Coord;
use editor::{
    blame::BlameCell,
    document::{DocumentId, PrefabInstanceId},
};

use super::{Session, SharedHighlights};

pub(super) fn always_highlighted(bake: Option<&editor::bake::Bake>) -> Vec<PrefabInstanceId> {
    let Some(bake) = bake else {
        return Vec::new();
    };

    let mut always = bake
        .highlighted()
        .filter(|(_, list)| {
            list.iter()
                .any(|highlight| highlight.shown_when(editor::bake::HIGHLIGHT_ALWAYS))
        })
        .filter_map(|(id, _)| PrefabInstanceId::from_raw(id))
        .collect::<Vec<_>>();
    always.sort_unstable();

    always
}

/// Highlights for one frame, borrowed from the bake or shared from the git caches
pub(crate) struct HighlightSet<'a> {
    borrowed: Vec<&'a editor::bake::Highlight>,
    shared: Vec<SharedHighlights>,
}

impl HighlightSet<'_> {
    pub(crate) fn iter(&self) -> impl Iterator<Item = &editor::bake::Highlight> + '_ {
        self.borrowed
            .iter()
            .copied()
            .chain(self.shared.iter().flat_map(|highlights| highlights.iter()))
    }
}

impl Session {
    pub(crate) fn highlights(&self, id: DocumentId, hovered: Option<Coord>) -> HighlightSet<'_> {
        let mut set = HighlightSet {
            borrowed: Vec::new(),
            shared: Vec::new(),
        };
        let Some(document) = self.state.document(id) else {
            return set;
        };
        let Some(cache) = self.caches.get(&id) else {
            return set;
        };
        let z = document.z as i32;
        let mut seen = HashSet::new();
        let mut sources = cache
            .always_highlights
            .iter()
            .map(|owner| (*owner, editor::bake::HIGHLIGHT_ALWAYS))
            .collect::<Vec<_>>();
        if let Some(selected) = document.selected_instance() {
            sources.push((selected, editor::bake::HIGHLIGHT_SELECTED));
        }

        if let Some(hovered) = hovered {
            sources.extend(
                document
                    .instance_ids_at(hovered)
                    .iter()
                    .map(|owner| (*owner, editor::bake::HIGHLIGHT_HOVERED)),
            );
        }

        if let Some(bake) = cache.bake.as_ref() {
            for (owner, when) in sources {
                for (index, highlight) in bake.highlights(owner.get()).iter().enumerate() {
                    if highlight.z == z && highlight.shown_when(when) && seen.insert((owner, index)) {
                        set.borrowed.push(highlight);
                    }
                }
            }
        }

        if let Some(conflicts) = self.with_conflict_view(id, |view, conflicts| view.highlights(conflicts, document.z)) {
            set.shared.push(conflicts);
        }

        if cache.git.as_ref().is_some_and(|git| git.show_diff)
            && let Some(highlights) = self.with_diff_view(id, |view| {
                view.highlights
                    .entry(document.z)
                    .or_insert_with(|| view.diff.highlights(document.z).into())
                    .clone()
            })
        {
            set.shared.push(highlights);
        }

        if let Some(git) = cache.git.as_ref().filter(|git| self.git_enabled && git.show_blame)
            && let Some(blame) = git.blame.as_ref()
        {
            set.shared.push(blame.heatmap(document.z));
            let hovered_commit = hovered.and_then(|coord| match blame.result.at(coord) {
                Some(BlameCell::Commit(index, _)) => Some(index),
                _ => None,
            });
            for index in [blame.pinned, hovered_commit]
                .into_iter()
                .flatten()
                .collect::<HashSet<_>>()
            {
                set.shared.push(blame.outline(document.z, index));
            }
        }

        set
    }
}
