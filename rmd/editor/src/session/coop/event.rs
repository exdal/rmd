use std::{fs, mem, path::Path, thread};

use dmm::parser;
use net::{Direction, Event, GenerationId, PeerId};

use super::{Activity, Coop, CoopStatus, Prepared, Received, ReceivedMap, RemotePeer, SharedMap, SharedState};

impl Coop {
    fn receive_map(&mut self, path: String, generation: GenerationId, bytes: Vec<u8>, codebase: Option<&Path>) {
        if !net::is_map_path(&path) {
            log::warn!("ignoring a shared map outside the codebase: {path}");
            return;
        }

        let Some(file) = codebase.map(|codebase| codebase.join(&path)) else {
            return;
        };

        let Some(shared_map) = self
            .shared_maps
            .get_mut(&path)
            .filter(|shared_map| shared_map.state.accepts(generation))
        else {
            log::debug!("{path}: dropping a snapshot nobody is waiting for");
            return;
        };

        shared_map.state = SharedState::Loading(generation);

        let prepared = self.prepared.0.clone();
        thread::spawn(move || {
            let is_modified = fs::read(&file).map_or(true, |disk| disk != bytes);
            let text = match String::from_utf8(bytes) {
                Ok(text) => text,
                Err(e) => {
                    log::warn!("{path}: the shared map is not text: {e}");
                    let _ = prepared.send(Prepared::Unreadable { path, generation });
                    return;
                },
            };

            let (map, errors) = parser::parse(&text);
            let _ = prepared.send(Prepared::Snapshot(ReceivedMap {
                path,
                generation,
                file,
                map,
                errors,
                is_modified,
            }));
        });
    }

    pub(super) fn apply(&mut self, event: Event, codebase: Option<&Path>) {
        if matches!(self.status, CoopStatus::Ended(_) | CoopStatus::CodebaseMismatch { .. }) {
            return;
        }

        match event {
            Event::Connected {
                you, peers, comments, ..
            } => {
                self.status = CoopStatus::Connected;
                self.you = Some(you);
                self.peers = peers.into_iter().map(|info| (info.id, RemotePeer::new(info))).collect();
                self.comments = comments.into_iter().map(|comment| (comment.id, comment)).collect();
            },
            Event::PeerJoined(info) => {
                self.record(Activity::Joined(info.clone()));
                self.peers.insert(info.id, RemotePeer::new(info));
            },
            Event::PeerLeft(id) => {
                if let Some(peer) = self.peers.remove(&id) {
                    self.record(Activity::Left(peer.info));
                }

                if self.following.as_ref().is_some_and(|following| following.peer == id) {
                    self.following = None;
                }
            },
            Event::Cursor { from, cursor } => {
                if let Some(peer) = self.peers.get_mut(&from) {
                    peer.set_cursor(cursor);
                }
            },
            Event::View { from, view } => {
                if let Some(peer) = self.peers.get_mut(&from) {
                    peer.view = view
                        .filter(|view| {
                            view.zoom.is_finite()
                                && view.zoom > 0.0
                                && view.center.iter().all(|value| value.is_finite())
                        })
                        .map(Received::now);
                }
            },
            Event::Selection { from, selection } => {
                if let Some(peer) = self.peers.get_mut(&from) {
                    peer.selection = selection
                        .filter(|selection| {
                            selection.z >= 1
                                && selection.min.iter().all(|&value| value >= 1)
                                && selection.min[0] <= selection.max[0]
                                && selection.min[1] <= selection.max[1]
                        })
                        .map(Received::now);
                }
            },
            Event::Comment(comment) => {
                self.comments.insert(comment.id, comment);
            },
            Event::CommentDeleted(id) => {
                self.comments.remove(&id);
            },
            Event::MapIncoming { path, by, len } => {
                let shared_map = self
                    .shared_maps
                    .entry(path)
                    .or_insert_with(|| SharedMap::new(by, SharedState::Closed));

                let before = match mem::replace(&mut shared_map.state, SharedState::Closed) {
                    SharedState::Incoming { before, .. } => before,
                    state => Box::new(state),
                };

                shared_map.state = SharedState::Incoming { by, len, before };
            },
            Event::MapCancelled { path, by } => {
                let Some(shared_map) = self.shared_maps.get_mut(&path) else {
                    return;
                };

                shared_map.state = match mem::replace(&mut shared_map.state, SharedState::Closed) {
                    SharedState::Incoming {
                        by: uploader, before, ..
                    } if uploader == by => *before,
                    state => state,
                };

                if shared_map.is_abandoned() {
                    self.shared_maps.remove(&path);
                }
            },
            Event::MapShared { path, by, generation } => {
                let you = self.you;
                let shared_map = self
                    .shared_maps
                    .entry(path)
                    .or_insert_with(|| SharedMap::new(by, SharedState::RECEIVING));

                let is_known = shared_map.generation == Some(generation);
                shared_map.by = by;
                shared_map.renumber(generation);
                if Some(by) == you {
                    // we share this map, skip all sharing stuff
                    shared_map.state = SharedState::Ready;
                    shared_map.received = true;
                } else if !is_known
                    && matches!(
                        shared_map.state,
                        SharedState::Incoming { .. } | SharedState::Ready | SharedState::Waiting(_)
                    )
                {
                    // a map closed while it was uploading stays closed, and the snapshot may have come first
                    shared_map.state = SharedState::RECEIVING;
                }
            },
            Event::Progress {
                path,
                direction: Direction::Sending,
                done,
                total,
            } => {
                if let Some(shared_map) = self.shared_maps.get_mut(&path)
                    && matches!(shared_map.state, SharedState::Sending { .. })
                {
                    shared_map.state = SharedState::Sending { done, total };
                }
            },
            Event::Progress {
                path,
                direction: Direction::Receiving,
                done,
                total,
            } => {
                // special case handling:L the download can beat its announcement
                let shared_map = self
                    .shared_maps
                    .entry(path)
                    .or_insert_with(|| SharedMap::new(PeerId(0), SharedState::RECEIVING));

                if matches!(
                    shared_map.state,
                    SharedState::Incoming { .. } | SharedState::Receiving { .. }
                ) {
                    shared_map.state = SharedState::Receiving { done, total };
                }
            },
            Event::MapSnapshot {
                path,
                generation,
                bytes,
            } => self.receive_map(path, generation, bytes, codebase),
            Event::TransferFailed {
                path,
                direction,
                reason,
            } => {
                log::warn!("{path}: the transfer failed: {reason}");
                let Some(shared_map) = self.shared_maps.get_mut(&path) else {
                    return;
                };

                let is_sending = matches!(shared_map.state, SharedState::Sending { .. });
                let is_receiving = matches!(shared_map.state, SharedState::Receiving { .. });
                match direction {
                    // our copy no longer matches what peers have, so take the server's again
                    Direction::Sending if is_sending && shared_map.generation.is_some() => {
                        shared_map.restart(SharedState::Closed)
                    },
                    Direction::Sending if is_sending => {
                        self.shared_maps.remove(&path);
                    },
                    Direction::Receiving if is_receiving => shared_map.restart(SharedState::Closed),
                    _ => {},
                }
            },
            Event::Edit { by, seq, edit } => {
                if let Some(shared_map) = self.shared_maps.get_mut(&edit.path).filter(|shared_map| {
                    shared_map.generation == Some(edit.generation)
                        && !matches!(shared_map.state, SharedState::Closed)
                        && seq >= shared_map.next_seq
                }) {
                    shared_map.inbox.insert(seq, (by, edit));
                }
            },
            // the session closes its pending document too, see `forget_coop_map`
            Event::MapUnshared { .. } => {},
            Event::CodebaseMismatch { expected } => self.mismatch(expected),
            Event::Rejected(reason) => self.end(format!("rejected: {reason}")),
            Event::Disconnected(reason) => self.end(reason),
        }
    }
}
