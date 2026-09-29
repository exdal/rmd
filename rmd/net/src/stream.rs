use protocol::FrameReader;
use serde::{Serialize, de::DeserializeOwned};

use crate::{Error, fail};

pub(crate) async fn read_message<T: DeserializeOwned>(
    recv: &mut quinn::RecvStream, reader: &mut FrameReader,
) -> Result<Option<T>, Error> {
    let mut buffer = [0; 4096];
    loop {
        if let Some(message) = reader.next_message().map_err(fail)? {
            return Ok(Some(message));
        }

        let Some(read) = recv.read(&mut buffer).await.map_err(fail)? else {
            return Ok(None);
        };

        reader.push(&buffer[..read]);
    }
}

pub(crate) async fn write_message<T: Serialize>(send: &mut quinn::SendStream, message: &T) -> Result<(), Error> {
    let frame = protocol::encode_frame(message).map_err(fail)?;

    send.write_all(&frame).await.map_err(fail)
}
