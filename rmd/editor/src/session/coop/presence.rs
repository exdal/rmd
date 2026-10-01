use editor::document::DocumentId;
use net::{CommentId, Cursor, PeerId, View};

use super::{Coop, Following};
use crate::session::Session;

impl Session {
    pub fn comment_tool_available(&self) -> bool { self.coop.as_ref().is_some_and(Coop::can_collaborate) }

    pub fn add_coop_comment(&self, id: DocumentId, pos: [f32; 2], text: String) {
        let (Some(map), Some(document)) = (self.coop_shared_map_path(id), self.state.document(id)) else {
            return;
        };

        if let Some(client) = self.collaborating_client() {
            client.send_comment(map, document.z, pos, text);
        }
    }

    pub fn delete_coop_comment(&self, id: CommentId) {
        if let Some(client) = self.collaborating_client() {
            client.delete_comment(id);
        }
    }

    pub fn coop_cursor(&self, cursor: Option<Cursor>) {
        if let Some(client) = self.collaborating_client() {
            client.send_cursor(cursor);
        }
    }

    pub fn coop_view(&self, view: Option<View>) {
        if let Some(coop) = self.coop.as_ref()
            && let Some(client) = coop.client.as_ref()
        {
            client.send_view(view.filter(|_| coop.can_collaborate()));
        }
    }

    pub fn coop_following(&self) -> Option<PeerId> {
        self.coop.as_ref()?.following.as_ref().map(|following| following.peer)
    }

    pub fn toggle_coop_follow(&mut self, peer: PeerId) {
        let Some(coop) = self.coop.as_mut().filter(|coop| coop.peers.contains_key(&peer)) else {
            return;
        };

        let is_following = coop.following.as_ref().is_some_and(|following| following.peer == peer);
        coop.following = (!is_following).then_some(Following { peer, document: None });
    }

    pub fn stop_coop_follow(&mut self) {
        if let Some(coop) = self.coop.as_mut() {
            coop.following = None;
        }
    }

    pub fn coop_follow_target(&mut self) -> Option<(DocumentId, View)> {
        let coop = self.coop.as_ref()?;
        let following = coop.following.as_ref()?;
        let active = self.state.active();
        // a map someone just shared opens on its own, only the user picking another map stops following
        let is_pending = coop
            .shared_maps
            .values()
            .any(|shared_map| active.is_some() && shared_map.pending_document == active);
        if following.document.is_some_and(|id| active != Some(id)) && !is_pending {
            self.stop_coop_follow();
            return None;
        }

        let view = coop.peers.get(&following.peer)?.view.clone()?;
        let id = self.open_coop_map(&view.map)?;
        self.set_level_of(id, view.z);
        if let Some(following) = self.coop.as_mut().and_then(|coop| coop.following.as_mut()) {
            following.document = Some(id);
        }

        Some((id, view))
    }
}
