use editor::document::DocumentId;
use net::{CommentId, Cursor};

use super::Coop;
use crate::session::Session;

impl Session {
    pub fn comment_tool_available(&self) -> bool { self.coop.as_ref().is_some_and(Coop::can_collaborate) }

    pub fn add_coop_comment(&self, id: DocumentId, pos: [f32; 2], text: String) {
        let (Some(map), Some(document)) = (self.coop_shared_map_path(id), self.state.document(id)) else {
            return;
        };

        if let Some(client) = self
            .coop
            .as_ref()
            .filter(|coop| coop.can_collaborate())
            .and_then(|coop| coop.client.as_ref())
        {
            client.send_comment(map, document.z, pos, text);
        }
    }

    pub fn delete_coop_comment(&self, id: CommentId) {
        if let Some(client) = self
            .coop
            .as_ref()
            .filter(|coop| coop.can_collaborate())
            .and_then(|coop| coop.client.as_ref())
        {
            client.delete_comment(id);
        }
    }

    pub fn coop_cursor(&self, cursor: Option<Cursor>) {
        if let Some(client) = self
            .coop
            .as_ref()
            .filter(|coop| coop.can_collaborate())
            .and_then(|coop| coop.client.as_ref())
        {
            client.send_cursor(cursor);
        }
    }
}
