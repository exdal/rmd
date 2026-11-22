use std::path::{Path, PathBuf};

use super::{Haystack, Query, score};
use crate::session::Session;

pub(crate) struct MapCandidate {
    pub(crate) path: PathBuf,
    haystack: Haystack,
}

impl MapCandidate {
    pub(crate) fn new(base: Option<&Path>, path: &Path) -> Self {
        let relative = base.and_then(|base| path.strip_prefix(base).ok()).unwrap_or(path);
        let text = relative
            .with_extension("")
            .components()
            .map(|component| component.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");

        Self {
            haystack: Haystack::new(&text, None, None),
            path: path.to_owned(),
        }
    }
}

pub(crate) fn build_maps(session: &Session) -> Vec<MapCandidate> {
    session
        .maps()
        .iter()
        .map(|path| MapCandidate::new(session.codebase_dir(), path))
        .collect::<Vec<_>>()
}

/// Ties go to the shorter path
pub(crate) fn rank_maps(maps: &[MapCandidate], query: &Query) -> Vec<usize> {
    if query.is_empty() {
        return (0..maps.len()).collect::<Vec<_>>();
    }

    let mut scored = maps
        .iter()
        .enumerate()
        .filter_map(|(index, candidate)| Some((index, score(query, &candidate.haystack)?)))
        .collect::<Vec<_>>();
    scored.sort_by(|&(a, a_score), &(b, b_score)| {
        let (a, b) = (&maps[a].path, &maps[b].path);

        b_score
            .cmp(&a_score)
            .then_with(|| a.as_os_str().len().cmp(&b.as_os_str().len()))
            .then_with(|| a.cmp(b))
    });

    scored.into_iter().map(|(index, _)| index).collect::<Vec<_>>()
}
