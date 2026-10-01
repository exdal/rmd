mod client;
mod proxy;
mod server;
mod stream;
mod tls;

use std::{error, fmt};

pub use protocol::coop::{
    CodebaseHash,
    CodebaseId,
    Comment,
    CommentId,
    Cursor,
    GenerationId,
    MAX_COMMENT_LEN,
    MapEdit,
    PeerId,
    PeerInfo,
    SeqId,
    Tool,
    View,
    is_map_path,
};
use tokio::runtime::{Builder, Runtime};

pub use crate::{
    client::{Client, Direction, Event},
    proxy::{Impairment, LossyProxy},
    server::{Server, ServerConfig, random_password},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(&self.0) }
}

impl error::Error for Error {}

pub(crate) fn fail(error: impl fmt::Display) -> Error { Error(error.to_string()) }

pub(crate) fn runtime() -> Result<Runtime, Error> { Builder::new_current_thread().enable_all().build().map_err(fail) }
