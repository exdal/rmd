mod client;
mod server;
mod stream;
mod tls;

pub use protocol::live::{CodebaseId, Comment, CommentId, Cursor, MAX_COMMENT_LEN, PeerId, PeerInfo, is_map_path};

pub use crate::{
    client::{Client, Event},
    server::{Server, ServerConfig, random_password},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(&self.0) }
}

impl std::error::Error for Error {}

pub(crate) fn fail(error: impl std::fmt::Display) -> Error { Error(error.to_string()) }

pub(crate) fn runtime() -> Result<tokio::runtime::Runtime, Error> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(fail)
}
