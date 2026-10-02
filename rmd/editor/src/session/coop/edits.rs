use dmm::Coord;
use editor::patch;
use net::MapEdit;

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

        for (path, by, edit) in incoming {
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
            let mut applied = Vec::new();
            for (coord, tile) in coords.zip(tiles) {
                let Some(count) = shared_map.in_flight.get_mut(&coord) else {
                    applied.push((coord, tile));
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

            let Some(id) = self.shared_document(&path).filter(|_| !applied.is_empty()) else {
                continue;
            };

            let Some(document) = self.state.document_mut(id) else {
                continue;
            };

            let affected = document.apply_remote(applied);
            self.update_document_instances(id, &affected);
            if let Some(cache) = self.caches.get_mut(&id) {
                cache.map_revision = cache.map_revision.wrapping_add(1);
            }
        }
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

            let Some(journal) = self.state.document_mut(id).and_then(|document| document.take_journal()) else {
                continue;
            };

            if journal.reshaped {
                self.share_coop_document(id);
                continue;
            }

            let Some((coords, patch)) = self
                .state
                .document(id)
                .and_then(|document| patch::encode(&document.map, journal.coords))
            else {
                continue;
            };

            let Some(coop) = self.coop.as_mut() else {
                return;
            };

            if let Some(shared_map) = coop.shared_maps.get_mut(&path) {
                for coord in &coords {
                    *shared_map.in_flight.entry(*coord).or_insert(0) += 1;
                }
            }

            if let Some(client) = coop.client.as_ref() {
                client.send_edit(MapEdit {
                    path,
                    generation,
                    coords: coords.iter().map(|coord| [coord.x, coord.y, coord.z]).collect(),
                    patch,
                });
            }
        }
    }
}
