use std::path::{Component, Path, PathBuf};

use super::{Haystack, Query, score};
use crate::session::Session;

pub(crate) struct PathCandidate {
    pub(crate) path: PathBuf,
    haystack: Haystack,
}

impl PathCandidate {
    pub(crate) fn new(base: Option<&Path>, path: &Path) -> Self {
        let relative = base.and_then(|base| path.strip_prefix(base).ok()).unwrap_or(path);
        let text = relative
            .with_extension("")
            .components()
            .filter_map(|component| match component {
                Component::Normal(part) => Some(part.to_string_lossy()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("/");

        Self {
            haystack: Haystack::new(&text, None, None),
            path: path.to_owned(),
        }
    }
}

pub(crate) fn build_maps(session: &Session) -> Vec<PathCandidate> {
    session
        .maps()
        .iter()
        .map(|path| PathCandidate::new(session.codebase_dir(), path))
        .collect::<Vec<_>>()
}

/// Ties go to the shorter path
pub(crate) fn rank_paths(paths: &[PathCandidate], query: &Query) -> Vec<usize> {
    if query.is_empty() {
        return (0..paths.len()).collect::<Vec<_>>();
    }

    let mut scored = paths
        .iter()
        .enumerate()
        .filter_map(|(index, candidate)| Some((index, score(query, &candidate.haystack)?)))
        .collect::<Vec<_>>();
    scored.sort_by(|&(a, a_score), &(b, b_score)| {
        let (a, b) = (&paths[a].path, &paths[b].path);

        b_score
            .cmp(&a_score)
            .then_with(|| a.as_os_str().len().cmp(&b.as_os_str().len()))
            .then_with(|| a.cmp(b))
    });

    scored.into_iter().map(|(index, _)| index).collect::<Vec<_>>()
}
