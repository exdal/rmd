use std::{path::Path, thread};

use dmm::{Size, writer};
use editor::{
    document::{DocumentId, MapDocument},
    git,
};
use net::PeerId;

use super::{Activity, Coop, Prepared, ReceivedMap, SharedMap, SharedState};
use crate::{loader::LoadedMap, session::Session};

impl Session {
    pub(super) fn follow_shared_documents(&mut self) {
        self.follow_pending_documents();
        let Some(coop) = self.coop.as_ref() else {
            return;
        };

        let mut closed = Vec::new();
        let mut reopened = Vec::new();
        for (path, shared_map) in &coop.shared_maps {
            match (&shared_map.state, self.shared_document(path).is_some()) {
                (SharedState::Ready, false) => closed.push(path.clone()),
                (SharedState::Closed, true) => reopened.push(path.clone()),
                _ => {},
            }
        }

        if let Some(coop) = self.coop.as_mut() {
            for path in &closed {
                if let Some(shared_map) = coop.shared_maps.get_mut(path) {
                    shared_map.restart(SharedState::Closed);
                }
            }
        }

        for path in reopened {
            self.resync_coop_map(&path);
        }

        self.lock_coop_documents();
    }

    fn follow_pending_documents(&mut self) {
        let Some(codebase) = self.codebase_dir().map(Path::to_path_buf) else {
            return;
        };

        let Some(coop) = self.coop.as_mut() else {
            return;
        };

        let is_connected = coop.is_connected();
        let mut stale = Vec::new();
        let mut gone = Vec::new();
        let mut wanted = Vec::new();
        for (path, shared_map) in &mut coop.shared_maps {
            match shared_map.pending_document {
                // the user closed it before the map arrived
                Some(id) if self.state.document(id).is_none() => {
                    shared_map.pending_document = None;
                    if shared_map.state.is_arriving() {
                        shared_map.restart(SharedState::Closed);
                    }
                },
                // stale map
                Some(id) if !is_connected || !shared_map.state.is_arriving() => {
                    shared_map.pending_document = None;
                    stale.push(id);
                    if shared_map.is_abandoned() {
                        gone.push(path.clone());
                    }
                },
                None if is_connected && shared_map.state.is_arriving() => wanted.push(path.clone()),
                _ => {},
            }
        }

        if is_connected {
            for path in gone {
                coop.shared_maps.remove(&path);
            }
        } else {
            coop.shared_maps.clear();
        }

        for id in stale {
            self.close_map(id);
        }

        for path in wanted {
            // an open copy shows the progress itself
            if self.shared_document(&path).is_some() {
                continue;
            }

            let mut document = MapDocument::open(codebase.join(&path), dmm::Map::new(Size { x: 1, y: 1, z: 1 }), 1);
            document.set_read_only(true);
            let id = self.activate_document(document);
            if let Some(shared_map) = self.coop.as_mut().and_then(|coop| coop.shared_maps.get_mut(&path)) {
                shared_map.pending_document = Some(id);
            }
        }
    }

    pub(super) fn lock_coop_documents(&mut self) {
        let locks = self
            .state
            .document_ids()
            .into_iter()
            .map(|id| {
                (
                    id,
                    self.coop_receiving(id).is_some()
                        || self.coop_out_of_date(id).is_some()
                        || self.coop.as_ref().is_some_and(|coop| coop.paused) && self.coop_shared_map(id).is_some(),
                )
            })
            .collect::<Vec<_>>();

        for (id, locked) in locks {
            if let Some(document) = self.state.document_mut(id) {
                document.set_read_only(locked);
            }
        }
    }

    fn coop_shared_map(&self, id: DocumentId) -> Option<&SharedMap> {
        self.coop.as_ref()?.shared_maps.get(&self.coop_map_path(id)?)
    }

    // the map is on its way and will replace this document
    pub fn coop_receiving(&self, id: DocumentId) -> Option<&SharedState> {
        let shared_map = self.coop_shared_map(id)?;
        let is_replaced = shared_map.pending_document == Some(id)
            || shared_map.received
            || self.state.document(id).is_some_and(|document| !document.is_dirty());

        (shared_map.state.is_arriving() && is_replaced).then_some(&shared_map.state)
    }

    // a newer snapshot from this peer waits on the document's unsaved changes
    pub fn coop_out_of_date(&self, id: DocumentId) -> Option<PeerId> {
        let shared_map = self.coop_shared_map(id)?;
        // saved or handed over, the next poll replaces it
        let is_waiting = matches!(shared_map.state, SharedState::Waiting(_))
            && !shared_map.received
            && self.state.document(id).is_some_and(MapDocument::is_dirty);

        is_waiting.then_some(shared_map.by)
    }

    // hands the unsaved copy over to the session, so the waiting snapshot replaces it
    pub fn discard_for_coop_map(&mut self, id: DocumentId) {
        let Some(path) = self.coop_map_path(id) else {
            return;
        };

        if let Some(shared_map) = self.coop.as_mut().and_then(|coop| coop.shared_maps.get_mut(&path))
            && matches!(shared_map.state, SharedState::Waiting(_))
        {
            shared_map.received = true;
        }
    }

    pub fn coop_transfer(&self, id: DocumentId) -> Option<&SharedState> {
        let shared_map = self.coop_shared_map(id)?;
        let is_sending = matches!(shared_map.state, SharedState::Sending { .. });

        (is_sending || shared_map.state.is_arriving()).then_some(&shared_map.state)
    }

    pub fn is_coop_shared_file(&self, file: &Path) -> bool {
        self.coop_file_path(file)
            .zip(self.coop.as_ref())
            .is_some_and(|(path, coop)| coop.shared_maps.contains_key(&path))
    }

    pub fn open_coop_file(&mut self, file: &Path) -> bool {
        let Some(path) = self.coop_file_path(file).filter(|_| self.is_coop_shared_file(file)) else {
            return false;
        };

        self.open_coop_map(&path).is_some()
    }

    pub fn open_coop_map(&mut self, path: &str) -> Option<DocumentId> {
        if let Some(id) = self.shared_document(path) {
            return self.set_active_document(id).then_some(id);
        }

        if self
            .coop
            .as_ref()
            .and_then(|coop| coop.shared_maps.get(path))
            .is_some_and(|shared_map| matches!(shared_map.state, SharedState::Closed))
        {
            self.resync_coop_map(path);
            self.follow_pending_documents();
        }

        self.shared_document(path)
    }

    fn resync_coop_map(&mut self, path: &str) {
        let Some(coop) = self.coop.as_mut() else {
            return;
        };

        let Some(shared_map) = coop.shared_maps.get_mut(path) else {
            return;
        };

        shared_map.restart(SharedState::RECEIVING);
        // the snapshot replaces whatever copy is open, unsaved or not
        shared_map.received = true;
        if let Some(client) = coop.client.as_ref() {
            client.resync(path.to_owned());
        }
    }

    pub(super) fn shared_document(&self, path: &str) -> Option<DocumentId> {
        self.state.document_for_path(&self.codebase_dir()?.join(path))
    }

    pub fn can_share_coop_maps(&self) -> bool { self.coop.as_ref().is_some_and(Coop::can_collaborate) }

    pub fn can_share_coop_map(&self) -> bool { self.state.active().is_some_and(|id| self.can_share_coop_document(id)) }

    pub fn can_share_coop_document(&self, id: DocumentId) -> bool {
        self.can_share_coop_maps() && self.coop_map_path(id).is_some()
    }

    pub fn share_coop_file(&mut self, file: &Path) -> bool {
        let Some(id) = self
            .state
            .document_for_path(file)
            .filter(|_| self.can_share_coop_maps())
        else {
            return false;
        };

        self.set_active_document(id);
        self.share_coop_document(id);

        true
    }

    pub fn share_coop_map(&mut self) {
        if let Some(id) = self.state.active() {
            self.share_coop_document(id);
        }
    }

    pub fn share_coop_document(&mut self, id: DocumentId) {
        if !self.coop.as_ref().is_some_and(Coop::can_collaborate) {
            return;
        }

        let Some(path) = self.coop_map_path(id) else {
            return;
        };

        let Some(document) = self.state.document_mut(id) else {
            return;
        };

        // edits so far are part of the snapshot, the journal only carries what comes after it
        document.start_journal();

        let Some(coop) = self.coop.as_mut() else {
            return;
        };

        let you = coop.you.unwrap_or(PeerId(0));
        let shared_map = coop
            .shared_maps
            .entry(path.clone())
            .or_insert_with(|| SharedMap::new(you, SharedState::Closed));

        // only a copy that follows its generation has the edits before `next_seq`
        let base = shared_map
            .generation
            .filter(|_| shared_map.is_ready())
            .map(|generation| (generation, shared_map.next_seq));

        // hold back new edits until the server numbers this share
        shared_map.state = SharedState::Sending { done: 0, total: 0 };

        let map = document.to_map();
        let levels = map.size.z;
        let prepared = coop.prepared.0.clone();
        thread::spawn(move || {
            let bytes = writer::write(&map).into_bytes();
            let _ = prepared.send(Prepared::Upload {
                path,
                base,
                levels,
                bytes,
            });
        });
    }

    pub fn can_stop_sharing_coop_map(&self) -> bool {
        self.state
            .active()
            .is_some_and(|id| self.can_stop_sharing_coop_document(id))
    }

    pub fn can_stop_sharing_coop_document(&self, id: DocumentId) -> bool {
        self.can_share_coop_maps() && self.coop_shared_map(id).is_some()
    }

    pub fn stop_sharing_coop_map(&mut self) {
        if let Some(id) = self.state.active() {
            self.stop_sharing_coop_document(id);
        }
    }

    // every peer, us included, drops the map once the server echoes this
    pub fn stop_sharing_coop_document(&mut self, id: DocumentId) {
        let Some(path) = self.coop_map_path(id) else {
            return;
        };

        if let Some(client) = self.collaborating_client() {
            client.unshare_map(path);
        }
    }

    // open copies stay as local maps, only a document still waiting for the map closes
    pub(super) fn forget_coop_map(&mut self, path: &str, by: PeerId) {
        let Some(coop) = self.coop.as_mut() else {
            return;
        };

        coop.comments.retain(|_, comment| comment.map != path);
        let Some(shared_map) = coop.shared_maps.remove(path) else {
            return;
        };

        log::info!("{path}: {} stopped sharing", coop.nick_of(by).unwrap_or("a peer"));
        let nick = coop.nick_of(by).map(str::to_owned);
        coop.record(Activity::Unshared {
            path: path.to_owned(),
            by,
            nick,
        });
        if let Some(id) = shared_map.pending_document {
            self.close_map(id);
        }
    }

    pub(super) fn open_shared_map(&mut self, received: ReceivedMap) {
        let Some(shared_map) = self
            .coop
            .as_mut()
            .and_then(|coop| coop.shared_maps.get_mut(&received.path))
            .filter(|shared_map| {
                matches!(shared_map.state, SharedState::Loading(loading) if loading == received.generation)
            })
        else {
            return;
        };

        if shared_map
            .generation
            .is_some_and(|current| received.generation < current)
        {
            // a newer share is on its way
            shared_map.state = SharedState::RECEIVING;
            return;
        }

        let open = self.state.document_for_path(&received.file);
        // a local copy with unsaved work waits until it is saved or closed, our copy of the session's map does not
        if !shared_map.received && open.is_some_and(|id| self.state.document(id).is_some_and(MapDocument::is_dirty)) {
            shared_map.state = SharedState::Waiting(Box::new(received));
            return;
        }

        let ReceivedMap {
            generation,
            file,
            map,
            errors,
            is_modified,
            ..
        } = received;

        // the snapshot of a new share can beat its announcement
        shared_map.renumber(generation);
        shared_map.state = SharedState::Ready;
        shared_map.in_flight.clear();
        shared_map.pending_document = None;
        shared_map.received = true;

        let repo = self.git_enabled.then(|| git::discover(&file)).flatten();
        let loaded = LoadedMap {
            path: file.clone(),
            map,
            z: 1,
            errors,
            repo,
            conflict: None,
        };

        match open {
            Some(id) => {
                self.reload_map(id, loaded);
            },
            None => self.apply_map(loaded),
        }

        if let Some(document) = self
            .state
            .document_for_path(&file)
            .and_then(|id| self.state.document_mut(id))
        {
            document.start_journal();
            if is_modified {
                document.mark_unsaved();
            }
        }
    }

    pub fn coop_map_path(&self, id: DocumentId) -> Option<String> {
        self.coop_file_path(self.state.document(id)?.path.as_deref()?)
    }

    // peers only line up on a map they all have the shared copy of
    pub fn coop_shared_map_path(&self, id: DocumentId) -> Option<String> {
        let path = self.coop_map_path(id)?;
        let is_ready = self.coop.as_ref()?.shared_maps.get(&path)?.is_ready();

        is_ready.then_some(path)
    }

    fn coop_file_path(&self, file: &Path) -> Option<String> {
        let relative = file.strip_prefix(self.codebase_dir()?).ok()?;
        let path = relative
            .components()
            .map(|part| part.as_os_str().to_str())
            .collect::<Option<Vec<_>>>()?
            .join("/");

        net::is_map_path(&path).then_some(path)
    }
}
