use std::{collections::BTreeMap, mem};

use dmm::{Coord, Prefab};
use editor::{document::DocumentId, patch};
use net::{DeleteLevel, InsertAt, InsertLevel, LevelChange, LevelOp, MapEdit};

use super::{LevelRequest, LevelStep};
use crate::session::Session;

impl Session {
    pub(super) fn apply_coop_edits(&mut self) {
        let Some(coop) = self.coop.as_mut() else {
            return;
        };

        // we need ourselves here, this function is also used to keep us synced with the server
        // I WANT YOU TO KEEP IN SYNC!
        let you = coop.you;

        let mut incoming = Vec::new();
        for (path, shared_map) in coop
            .shared_maps
            .iter_mut()
            .filter(|(_, shared_map)| shared_map.is_ready())
        {
            while let Some((by, edit)) = shared_map.inbox.remove(&shared_map.next_seq) {
                incoming.push((path.clone(), shared_map.next_seq, by, edit));
                shared_map.next_seq = shared_map.next_seq.next();
            }
        }

        let mut batches = BTreeMap::<String, BTreeMap<Coord, Vec<Prefab>>>::new();
        for (path, seq, by, edit) in incoming {
            let is_ours = Some(by) == you;
            self.batch_coop_tiles(&mut batches, &path, is_ours, &edit);

            let Some(op) = edit.level.as_ref() else {
                continue;
            };

            let levels = self
                .shared_document(&path)
                .and_then(|id| self.state.document(id))
                .map_or(0, |document| document.map.size().z);
            let Some(shared_map) = self.coop.as_mut().and_then(|coop| coop.shared_maps.get_mut(&path)) else {
                continue;
            };

            // answered even when it lost a race, a carried one after a new share has no request left
            let step = is_ours.then(|| {
                shared_map
                    .level_request
                    .take()
                    .map_or(LevelStep::Change, |request| request.step())
            });

            // the second of two peers asking for the same append gets nothing, on every copy
            let Some(change) = shared_map.level_log.sequence(levels, seq, edit.seen, op) else {
                continue;
            };

            // tiles before the level change go in first, like they arrived
            if let Some(applied) = batches.remove(&path) {
                self.apply_coop_tiles(&path, applied);
            }

            self.apply_coop_level(&path, change, step, op);
        }

        // one apply per map, the last edit to a tile wins like it would one edit at a time
        for (path, applied) in batches {
            self.apply_coop_tiles(&path, applied);
        }
    }

    fn batch_coop_tiles(
        &mut self, batches: &mut BTreeMap<String, BTreeMap<Coord, Vec<Prefab>>>, path: &str, is_ours: bool,
        edit: &MapEdit,
    ) {
        if edit.coords.is_empty() {
            return;
        }

        let tiles = match patch::decode(&edit.patch, edit.coords.len()) {
            Ok(tiles) => tiles,
            Err(e) => {
                log::warn!("{path}: dropping an edit that does not parse: {e}");
                return;
            },
        };

        let Some(shared_map) = self.coop.as_mut().and_then(|coop| coop.shared_maps.get_mut(path)) else {
            return;
        };

        // the author didn't have the levels deleted since `seen`, its tiles there are gone
        let coords = edit
            .coords
            .iter()
            .map(|&[x, y, z]| shared_map.level_log.rebase(edit.seen, z).map(|z| Coord::new(x, y, z)));

        // a tile we have in flight keeps our version until the server echoes it back,
        // anything else applies, including our own edits replayed after a resync
        let applied = batches.entry(path.to_owned()).or_default();
        for (coord, tile) in coords.zip(tiles) {
            let Some(coord) = coord else {
                continue;
            };

            let Some(count) = shared_map.in_flight.get_mut(&coord) else {
                applied.insert(coord, tile);
                continue;
            };

            // bookkeeping
            if is_ours {
                *count -= 1;
                if *count == 0 {
                    shared_map.in_flight.remove(&coord);
                }
            }
        }
    }

    fn apply_coop_tiles(&mut self, path: &str, applied: BTreeMap<Coord, Vec<Prefab>>) {
        let Some(id) = self.shared_document(path).filter(|_| !applied.is_empty()) else {
            return;
        };

        if !self.apply_remote_tiles(id, applied.into_iter().collect()) {
            return;
        }

        if let Some(cache) = self.caches.get_mut(&id) {
            cache.map_revision = cache.map_revision.wrapping_add(1);
        }
    }

    // `step` is none for a peer's change
    fn apply_coop_level(&mut self, path: &str, change: LevelChange, step: Option<LevelStep>, op: &LevelOp) {
        let Some(id) = self.shared_document(path) else {
            return;
        };

        // our tiles waiting on an echo follow their level, or go with it
        if let Some(shared_map) = self.coop.as_mut().and_then(|coop| coop.shared_maps.get_mut(path)) {
            shared_map.in_flight = mem::take(&mut shared_map.in_flight)
                .into_iter()
                .filter_map(|(coord, count)| {
                    let z = change.level(coord.z)?;

                    Some((Coord { z, ..coord }, count))
                })
                .collect();
        }

        if self.node_edit.as_ref().is_some_and(|edit| edit.document == id) {
            self.cancel_node_edit();
        }

        let Some(document) = self.state.document_mut(id) else {
            return;
        };

        // a peer's level change in between cleared the history this was meant for
        let is_history_step = match step {
            Some(LevelStep::Undo) => document.undo_requested_level(change),
            Some(LevelStep::Redo) => document.redo_requested_level(change),
            _ => false,
        };
        let is_recorded = step == Some(LevelStep::Change);
        let is_applied = is_history_step
            || match (change, op) {
                (LevelChange::Appended(z) | LevelChange::Inserted(z), LevelOp::Insert(insert)) => {
                    let cells = document.level_cells(&insert.contents);
                    if is_recorded {
                        document.insert_requested_level(z, cells)
                    } else {
                        document.insert_remote_level(z, cells)
                    }
                },
                (LevelChange::Deleted(z), _) if is_recorded => document.delete_requested_level(z),
                (LevelChange::Deleted(z), _) => document.delete_remote_level(z),
                _ => false,
            };

        if !is_applied {
            return;
        }

        match change {
            LevelChange::Appended(z) if !is_history_step => {
                self.bake_appended_level(id, z);
                if is_recorded {
                    self.show_level(id, z);
                }
            },
            // do a full rebake, multiz lighting needs this
            _ => self.rebake_levels(id),
        }
    }

    pub(in crate::session) fn request_coop_level(&mut self, id: DocumentId, op: LevelOp) -> bool {
        self.queue_coop_level(id, op, LevelStep::Change)
    }

    // a shared map steps through level history one server round trip at a time
    pub(in crate::session) fn can_step_coop_level(&self, id: DocumentId) -> bool {
        self.coop_map_path(id)
            .and_then(|path| self.coop.as_ref()?.shared_maps.get(&path))
            .is_none_or(|shared_map| shared_map.is_ready() && shared_map.level_request.is_none())
    }

    // none for a map that isn't shared, its history steps stay local
    pub(in crate::session) fn request_coop_level_step(&mut self, id: DocumentId, step: LevelStep) -> Option<bool> {
        let path = self.coop_map_path(id)?;
        let shared_map = self.coop.as_ref()?.shared_maps.get(&path)?;
        if !shared_map.is_ready() || shared_map.level_request.is_some() {
            return Some(false);
        }

        let history = &self.state.document(id)?.history;
        let edit = match step {
            LevelStep::Undo => history.next_undo(),
            LevelStep::Redo => history.next_redo(),
            LevelStep::Change => None,
        };
        let Some(level) = edit.and_then(|edit| edit.level()) else {
            return Some(false);
        };

        let change = if step == LevelStep::Undo {
            level.undo_change()
        } else {
            level.change()
        };
        let op = match change {
            LevelChange::Deleted(z) => LevelOp::Delete(DeleteLevel { z }),
            LevelChange::Appended(z) | LevelChange::Inserted(z) => LevelOp::Insert(InsertLevel {
                at: InsertAt::Z(z),
                contents: patch::encode_level(level.cells()),
            }),
        };

        Some(self.queue_coop_level(id, op, step))
    }

    fn queue_coop_level(&mut self, id: DocumentId, op: LevelOp, step: LevelStep) -> bool {
        let Some(path) = self.coop_map_path(id) else {
            return false;
        };

        let Some(shared_map) = self
            .coop
            .as_mut()
            .and_then(|coop| coop.shared_maps.get_mut(&path))
            .filter(|shared_map| shared_map.generation.is_some())
        else {
            return false;
        };

        shared_map.level_request = Some(LevelRequest::Queued(op, step));

        true
    }

    pub(in crate::session) fn is_coop_level_requested(&self, id: DocumentId) -> bool {
        self.coop_map_path(id)
            .and_then(|path| self.coop.as_ref()?.shared_maps.get(&path))
            .is_some_and(|shared_map| shared_map.level_request.is_some())
    }

    pub(super) fn send_coop_edits(&mut self) {
        let Some(coop) = self.coop.as_ref() else {
            return;
        };

        let ready = coop
            .shared_maps
            .iter()
            .filter(|(_, shared_map)| shared_map.is_ready())
            .filter_map(|(path, shared_map)| Some((path.clone(), shared_map.generation?)))
            .collect::<Vec<_>>();

        for (path, generation) in ready {
            let Some(id) = self.shared_document(&path) else {
                continue;
            };

            let journal = self.state.document_mut(id).and_then(|document| document.take_journal());
            if journal.as_ref().is_some_and(|journal| journal.reshaped) {
                self.share_coop_document(id);
                continue;
            }

            let tiles = journal.and_then(|journal| {
                let document = self.state.document(id)?;

                patch::encode(&document.map, journal.coords)
            });

            let Some(coop) = self.coop.as_mut() else {
                return;
            };

            let Some(shared_map) = coop.shared_maps.get_mut(&path) else {
                continue;
            };

            let level = shared_map.send_level_request();
            if tiles.is_none() && level.is_none() {
                continue;
            }

            let (coords, patch) = tiles.unwrap_or_default();
            for coord in &coords {
                *shared_map.in_flight.entry(*coord).or_insert(0) += 1;
            }

            if let Some(client) = coop.client.as_ref() {
                client.send_edit(MapEdit {
                    path,
                    generation,
                    coords: coords.iter().map(|coord| [coord.x, coord.y, coord.z]).collect(),
                    patch,
                    level,
                    seen: shared_map.next_seq,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use net::LevelContents;

    use super::*;
    use crate::session::fixtures::{node_map, node_session};

    #[test]
    fn acknowledged_level_cancels_the_original_level_node_route_before_recording() {
        let start = Coord::new(1, 2, 1);
        let end = Coord::new(4, 2, 1);
        let (mut session, target) = node_session(node_map(4, 2, &[(start, vec!["/obj/cable"])]), start);
        let id = session.state.active().unwrap();
        let root = session.codebase_dir().unwrap().to_path_buf();
        session.state.document_mut(id).unwrap().path = Some(root.join("route.dmm"));
        let original = session.map().unwrap().clone();
        assert!(session.begin_node_edit(target));
        assert!(session.start_node_drag(start));
        assert!(session.update_node_drag(end));
        assert_ne!(session.map().unwrap(), &original);
        session.apply_coop_level(
            "route.dmm",
            LevelChange::Appended(2),
            Some(LevelStep::Change),
            &LevelOp::Insert(InsertLevel {
                at: InsertAt::Top,
                contents: LevelContents::Fill(patch::encode_tile(&[Prefab::new(core::path::TreePath::parse("/turf"))])),
            }),
        );
        assert_eq!(session.z(), 2);
        assert!(session.node_edit.is_none());
        let document = session.state.document(id).unwrap();
        assert_eq!(
            document.history.undo_depth(),
            1,
            "only the appended level remains in history"
        );
        for y in 1..=2 {
            for x in 1..=4 {
                let coord = Coord::new(x, y, 1);
                assert_eq!(document.map.tile_at(coord), original.tile_at(coord));
            }
        }

        assert!(session.undo());
        assert_eq!(session.map().unwrap(), &original);
        assert!(!session.undo());
    }
}
