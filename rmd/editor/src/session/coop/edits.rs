use std::collections::BTreeMap;

use dmm::{Coord, Prefab};
use editor::{
    document::{DocumentId, MapDocument},
    patch,
};
use net::{MapEdit, NewLevel};

use super::LevelRequest;
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
                shared_map.next_seq = shared_map.next_seq.next();
                incoming.push((path.clone(), by, edit));
            }
        }

        let mut batches = BTreeMap::<String, BTreeMap<Coord, Vec<Prefab>>>::new();
        for (path, by, edit) in incoming {
            if let Some(new_level) = &edit.new_level {
                // tiles before the append go in first, like they arrived
                if let Some(applied) = batches.remove(&path) {
                    self.apply_coop_tiles(&path, applied);
                }

                self.append_coop_level(&path, Some(by) == you, new_level);
            }

            if edit.coords.is_empty() {
                continue;
            }

            let tiles = match patch::decode(&edit.patch, edit.coords.len()) {
                Ok(tiles) => tiles,
                Err(e) => {
                    log::warn!("{path}: dropping an edit that does not parse: {e}");
                    continue;
                },
            };

            let coords = edit.coords.iter().map(|&[x, y, z]| Coord::new(x, y, z));
            let Some(shared_map) = self.coop.as_mut().and_then(|coop| coop.shared_maps.get_mut(&path)) else {
                continue;
            };

            // a tile we have in flight keeps our version until the server echoes it back,
            // anything else applies, including our own edits replayed after a resync
            let applied = batches.entry(path).or_default();
            for (coord, tile) in coords.zip(tiles) {
                let Some(count) = shared_map.in_flight.get_mut(&coord) else {
                    applied.insert(coord, tile);
                    continue;
                };

                // bookkeeping
                if Some(by) == you {
                    *count -= 1;
                    if *count == 0 {
                        shared_map.in_flight.remove(&coord);
                    }
                }
            }
        }

        // one apply per map, the last edit to a tile wins like it would one edit at a time
        for (path, applied) in batches {
            self.apply_coop_tiles(&path, applied);
        }
    }

    fn apply_coop_tiles(&mut self, path: &str, applied: BTreeMap<Coord, Vec<Prefab>>) {
        let Some(id) = self.shared_document(path).filter(|_| !applied.is_empty()) else {
            return;
        };

        let Some(document) = self.state.document_mut(id) else {
            return;
        };

        let affected = document.apply_remote(applied.into_iter().collect());
        self.update_document_instances(id, &affected);
        if let Some(cache) = self.caches.get_mut(&id) {
            cache.map_revision = cache.map_revision.wrapping_add(1);
        }
    }

    fn append_coop_level(&mut self, path: &str, is_ours: bool, new_level: &NewLevel) {
        if is_ours && let Some(shared_map) = self.coop.as_mut().and_then(|coop| coop.shared_maps.get_mut(path)) {
            shared_map.level_request = None;
        }

        let Some(id) = self.shared_document(path) else {
            return;
        };

        // the second of two peers asking for the same level gets nothing, on every copy
        if self
            .state
            .document(id)
            .is_none_or(|document| document.map.size().z + 1 != new_level.z)
        {
            return;
        }

        let fill = match patch::decode(&new_level.fill, 1) {
            Ok(mut tiles) => tiles.remove(0),
            Err(e) => {
                log::warn!("{path}: dropping a new level that does not parse: {e}");
                return;
            },
        };

        let Some(z) = self.append_document_level(id, &fill, MapDocument::append_remote_level) else {
            return;
        };

        if is_ours {
            self.cancel_node_edit();
            self.show_level(id, z);
        }
    }

    pub(in crate::session) fn request_coop_level(&mut self, id: DocumentId, z: u32, fill: &[Prefab]) -> bool {
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

        shared_map.level_request = Some(LevelRequest::Queued(NewLevel {
            z,
            fill: patch::encode_tile(fill),
        }));

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

            let new_level = shared_map.send_level_request();
            if tiles.is_none() && new_level.is_none() {
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
                    new_level,
                });
            }
        }
    }
}
