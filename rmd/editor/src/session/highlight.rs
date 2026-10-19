use std::collections::{HashMap, HashSet};

use dmm::Coord;
use editor::{
    bake::{HIGHLIGHT_ALWAYS, Highlight, highlight_tiles},
    blame::BlameCell,
    document::{DocumentId, PrefabInstanceId},
};

use super::{Session, SharedHighlights};

pub(super) const UNKNOWN_TYPE_COLOR: [f32; 3] = [0.58431375, 0.0, 1.0];

#[derive(Default)]
pub(super) struct UnknownHighlights {
    key: (u64, u64),
    levels: HashMap<u32, SharedHighlights>,
}

impl UnknownHighlights {
    fn get(
        &mut self, generation: u64, revision: u64, instances: &editor::frame::FrameInstances, z: u32,
    ) -> SharedHighlights {
        if self.key != (generation, revision) {
            self.key = (generation, revision);
            self.levels.clear();
        }

        self.levels
            .entry(z)
            .or_insert_with(|| {
                let tiles = instances
                    .unknown_type_coords()
                    .filter(|coord| coord.z == z)
                    .map(|coord| [coord.x as i32, coord.y as i32])
                    .collect::<HashSet<_>>();
                if tiles.is_empty() {
                    return Vec::new().into();
                }

                vec![Highlight {
                    tiles: highlight_tiles(&tiles),
                    z: z as i32,
                    color: UNKNOWN_TYPE_COLOR,
                    fill: 0.2,
                    outline: true,
                    when: HIGHLIGHT_ALWAYS,
                    label: None,
                }]
                .into()
            })
            .clone()
    }
}

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

/// Highlights for one frame, borrowed from the bake or shared from the document caches
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
                self.effective_grid(id)
                    .unwrap_or_else(|| (&document.map).into())
                    .ids_at(hovered)
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

        set.shared.push(cache.unknown_highlights.borrow_mut().get(
            document.generation(),
            cache.revision,
            &cache.instances,
            document.z,
        ));

        if let Some(conflicts) = self.with_conflict_view(id, |view, conflicts| view.highlights(conflicts, document.z)) {
            set.shared.push(conflicts);
        }

        set
    }
}

#[cfg(test)]
mod tests {
    use core::{location::Location, path::TreePath};
    use std::sync::Arc;

    use dmm::{Map, Prefab, Size};
    use editor::{Environment, document::MapDocument};
    use objtree::ObjectTree;

    use super::*;
    use crate::{loader::LoadedCodebase, session::fixtures::conflicted_session};

    fn codebase(extra_paths: &[&str]) -> LoadedCodebase {
        let mut tree = ObjectTree::new();
        for path in ["/turf/floor", "/obj/known"]
            .into_iter()
            .chain(extra_paths.iter().copied())
        {
            tree.register(&TreePath::parse(path), Location::default());
        }
        LoadedCodebase {
            environment: Environment::new("test.dme", tree),
            diagnostics: Default::default(),
            textures: Default::default(),
            thumbnails: HashMap::new(),
            maps: Vec::new(),
        }
    }

    fn unknown_session() -> Session {
        let mut session = Session::new();
        session.apply_codebase(codebase(&[]));
        let mut map = Map::new(Size { x: 3, y: 1, z: 2 });
        let known = map.intern_tile(vec![
            Prefab::new(TreePath::parse("/turf/floor")),
            Prefab::new(TreePath::parse("/obj")),
        ]);
        let unknown = map.intern_tile(
            ["/turf/floor", "/obj/missing", "/obj/missing", "/turf/missing"]
                .into_iter()
                .map(|path| Prefab::new(TreePath::parse(path)))
                .collect::<Vec<_>>(),
        );
        map.intern_tile(vec![Prefab::new(TreePath::parse("/obj/unused"))]);
        map.grid[0][0] = vec![unknown, unknown, known];
        map.grid[1][0] = vec![known, known, unknown];
        session.activate_document(MapDocument::new(map, 1));

        session
    }

    fn positions(session: &Session) -> Vec<[i32; 2]> {
        let id = session.state.active().unwrap();
        session
            .highlights(id, None)
            .iter()
            .filter(|highlight| highlight.color == UNKNOWN_TYPE_COLOR)
            .flat_map(|highlight| highlight.tiles.iter().map(|tile| tile.position))
            .collect::<Vec<_>>()
    }

    #[test]
    fn unknown_type_overlays_merge_adjacent_tiles_and_follow_each_documents_level() {
        let mut session = unknown_session();
        let id = session.state.active().unwrap();
        let first = session.highlights(id, None);
        let highlight = first.iter().next().unwrap();
        assert_eq!(highlight.color, UNKNOWN_TYPE_COLOR);
        assert_eq!(highlight.z, 1);
        assert!(highlight.outline && highlight.fill > 0.0);
        assert!(highlight.shown_when(HIGHLIGHT_ALWAYS));
        assert_eq!(positions(&session), [[1, 1], [2, 1]]);
        assert_eq!(highlight.tiles[0].edges & editor::bake::HIGHLIGHT_EDGE_EAST, 0);
        assert_eq!(highlight.tiles[1].edges & editor::bake::HIGHLIGHT_EDGE_WEST, 0);
        let again = session.highlights(id, Some(Coord::new(3, 1, 1)));
        assert!(Arc::ptr_eq(&first.shared[0], &again.shared[0]));
        drop((first, again));

        session.set_level(2);
        assert_eq!(positions(&session), [[3, 1]]);
        let second = session.activate_document(MapDocument::new(Map::new(Size { x: 1, y: 1, z: 1 }), 1));
        assert_eq!(session.highlights(second, None).iter().count(), 0);
        assert_eq!(session.highlights(id, None).iter().next().unwrap().z, 2);
    }

    #[test]
    fn removing_unknown_prefabs_updates_overlays_through_undo_and_redo_without_sprite_changes() {
        let mut session = unknown_session();
        let id = session.state.active().unwrap();
        let revision = session.caches[&id].revision;
        assert_eq!(positions(&session), [[1, 1], [2, 1]]);
        let owners = session
            .state
            .active_document()
            .unwrap()
            .instance_ids_at(Coord::new(1, 1, 1))[1..]
            .to_vec();
        assert!(session.delete_context_instance(owners[0]));
        assert_eq!(
            positions(&session),
            [[1, 1], [2, 1]],
            "other unknown prefabs still mark this tile"
        );
        for owner in &owners[1..] {
            assert!(session.delete_context_instance(*owner));
        }
        assert_eq!(session.caches[&id].revision, revision);
        assert_eq!(positions(&session), [[2, 1]]);
        assert!(session.undo());
        assert_eq!(positions(&session), [[1, 1], [2, 1]]);
        assert!(session.redo());
        assert_eq!(positions(&session), [[2, 1]]);
    }

    #[test]
    fn unknown_type_overlays_follow_remote_replacements_and_codebase_reload() {
        let mut session = unknown_session();
        let id = session.state.active().unwrap();
        assert_eq!(positions(&session), [[1, 1], [2, 1]]);
        let unknown = session.map().unwrap().tile_at(Coord::new(2, 1, 1)).unwrap().clone();
        let known = session.map().unwrap().tile_at(Coord::new(3, 1, 1)).unwrap().clone();
        assert!(session.apply_remote_tiles(id, vec![(Coord::new(2, 1, 1), known), (Coord::new(3, 1, 1), unknown)]));
        assert_eq!(positions(&session), [[1, 1], [3, 1]]);
        session.apply_codebase(codebase(&["/obj/missing", "/turf/missing"]));
        assert!(positions(&session).is_empty());
        session.apply_codebase(codebase(&[]));
        assert_eq!(positions(&session), [[1, 1], [3, 1]]);
    }

    #[test]
    fn conflicts_remain_visible_above_unknown_type_overlays() {
        let mut session = conflicted_session();
        session.apply_codebase(codebase(&[]));
        let id = session.state.active().unwrap();
        let unknown = vec![Prefab::new(TreePath::parse("/obj/missing"))];
        session.apply_remote_tiles(id, vec![(Coord::new(1, 1, 1), unknown)]);
        let set = session.highlights(id, None);
        let highlights = set.iter().collect::<Vec<_>>();
        assert_eq!(highlights.len(), 2);
        assert_eq!(highlights[0].color, UNKNOWN_TYPE_COLOR);
        assert_eq!(highlights[1].color, editor::conflict::UNRESOLVED_COLOR);
    }
}
