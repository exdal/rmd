use protocol::{
    FrameReader,
    MAX_FRAME_LEN,
    live::{MAX_MAP_LEN, Transfer},
};
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

pub(crate) async fn send_transfer(
    connection: &quinn::Connection, header: &Transfer, payload: &[u8],
) -> Result<(), Error> {
    let mut send = connection.open_uni().await.map_err(fail)?;
    send.write_all(&protocol::encode_frame(header).map_err(fail)?)
        .await
        .map_err(fail)?;

    send.write_all(payload).await.map_err(fail)?;
    send.finish().map_err(fail)
}

pub(crate) async fn receive_transfer(mut recv: quinn::RecvStream) -> Result<(Transfer, Vec<u8>), Error> {
    let bytes = recv.read_to_end(MAX_MAP_LEN + MAX_FRAME_LEN).await.map_err(fail)?;
    let (header, payload) = protocol::split_frame(&bytes).map_err(fail)?;

    Ok((protocol::decode(header).map_err(fail)?, payload.to_vec()))
}
