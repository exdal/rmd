use std::collections::HashMap;

use base::path::TreePath;
use objtree::ObjectTree;
use prefab::{Prefab, prefabs_equal};

use super::{command::Command, matcher::Haystack};
use crate::{
    palette::panel::prefab_menu::override_lines,
    session::Session,
    tool::place::is_placeable,
    viewport::{panel::context_menu::display_name, preview::PlacementPreview},
};

pub(super) enum Thumbnail {
    Pending,
    Resolved(Option<PlacementPreview>),
}

pub(super) struct PrefabCandidate {
    pub(super) prefab: Prefab,
    pub(super) path: String,
    pub(super) details: String,
    pub(super) haystack: Haystack,
    pub(super) count: u32,
    pub(super) recent_rank: Option<usize>,
    pub(super) thumbnail: Thumbnail,
}

pub(super) struct AreaCandidate {
    pub(super) prefab: Prefab,
    pub(super) name: String,
    pub(super) path: String,
    pub(super) haystack: Haystack,
    pub(super) count: u32,
}

pub(super) struct ActionCandidate {
    pub(super) command: Command,
    pub(super) haystack: Haystack,
}

fn edited_candidate(tree: &ObjectTree, prefab: &Prefab, count: u32) -> Option<PrefabCandidate> {
    tree.id_of(&prefab.path).filter(|id| is_placeable(tree, *id))?;
    let path = prefab.path.to_string();
    let details = override_lines(prefab).join(", ");

    Some(PrefabCandidate {
        haystack: Haystack::new(&path, Some(&display_name(Some(tree), prefab)), Some(&details)),
        details,
        prefab: prefab.clone(),
        path,
        count,
        recent_rank: None,
        thumbnail: Thumbnail::Pending,
    })
}

pub(super) fn build_prefabs(session: &Session, placed: &[(Prefab, u32)]) -> Vec<PrefabCandidate> {
    let Some(tree) = session.tree() else {
        return Vec::new();
    };

    let mut counts = HashMap::<String, u32>::new();
    for (prefab, count) in placed {
        *counts.entry(prefab.path.to_string()).or_default() += count;
    }

    let mut prefabs = tree
        .iter()
        .filter(|decl| is_placeable(tree, decl.id))
        .map(|decl| {
            let path = decl.path.to_string();
            let prefab = Prefab::new(TreePath::parse(&path));

            PrefabCandidate {
                haystack: Haystack::new(&path, Some(&display_name(Some(tree), &prefab)), None),
                count: counts.get(&path).copied().unwrap_or(0),
                prefab,
                path,
                details: String::new(),
                recent_rank: None,
                thumbnail: Thumbnail::Pending,
            }
        })
        .collect::<Vec<_>>();

    let edited = placed.iter().filter(|(prefab, _)| !prefab.vars.is_empty());
    for (prefab, count) in edited {
        if let Some(candidate) = edited_candidate(tree, prefab, *count) {
            prefabs.push(candidate);
        }
    }

    for (rank, recent) in session.recent_prefabs().iter().enumerate() {
        let found = prefabs
            .iter_mut()
            .find(|candidate| prefabs_equal(&candidate.prefab, &recent.prefab));
        if let Some(candidate) = found {
            candidate.recent_rank = Some(rank);

            continue;
        }

        if let Some(mut candidate) = edited_candidate(tree, &recent.prefab, 0) {
            candidate.recent_rank = Some(rank);
            prefabs.push(candidate);
        }
    }

    prefabs
}

pub(super) fn build_areas(session: &Session, placed: &[(Prefab, u32)]) -> Vec<AreaCandidate> {
    let Some(tree) = session.tree() else {
        return Vec::new();
    };

    let Some(root) = tree.roots().area else {
        return Vec::new();
    };

    placed
        .iter()
        .filter_map(|(prefab, count)| {
            tree.id_of(&prefab.path).filter(|id| tree.is_subtype_of(*id, root))?;
            let path = prefab.path.to_string();
            let name = display_name(Some(tree), prefab);

            Some(AreaCandidate {
                haystack: Haystack::new(&path, Some(&name), None),
                prefab: prefab.clone(),
                name,
                path,
                count: *count,
            })
        })
        .collect::<Vec<_>>()
}

pub(super) fn build_actions(session: &Session) -> Vec<ActionCandidate> {
    Command::all()
        .into_iter()
        .filter(|command| command.is_enabled(session))
        .map(|command| ActionCandidate {
            haystack: Haystack::label(command.label()),
            command,
        })
        .collect::<Vec<_>>()
}
