use protocol::{
    FrameReader,
    coop::{MAX_MAP_LEN, Transfer},
};
use serde::{Serialize, de::DeserializeOwned};

use crate::{Error, fail};

const CHUNK_LEN: usize = 64 * 1024;
const MAX_HEADER_LEN: usize = 64 * 1024;

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
    connection: &quinn::Connection, header: &Transfer, payload: &[u8], mut progress: impl FnMut(u64),
) -> Result<(), Error> {
    let mut send = Outgoing {
        stream: connection.open_uni().await.map_err(fail)?,
        acked: false,
    };

    send.stream
        .write_all(&protocol::encode_frame(header).map_err(fail)?)
        .await
        .map_err(fail)?;

    let mut written = 0;
    progress(0);
    for chunk in payload.chunks(CHUNK_LEN) {
        send.stream.write_all(chunk).await.map_err(fail)?;
        written += chunk.len() as u64;
        progress(written);
    }

    send.stream.finish().map_err(fail)?;
    send.stream.stopped().await.map_err(fail)?;
    send.acked = true;

    Ok(())
}

// quinn finishes a dropped stream, which would keep sending the rest of an abandoned transfer
struct Outgoing {
    stream: quinn::SendStream,
    acked: bool,
}

impl Drop for Outgoing {
    fn drop(&mut self) {
        if !self.acked {
            let _ = self.stream.reset(0u32.into());
        }
    }
}

pub(crate) async fn read_transfer_header(recv: &mut quinn::RecvStream) -> Result<Transfer, Error> {
    let mut len = [0; 4];
    recv.read_exact(&mut len).await.map_err(fail)?;

    let len = u32::from_le_bytes(len) as usize;
    if len > MAX_HEADER_LEN {
        return Err(fail(format!("a transfer header of {len} bytes is too large")));
    }

    let mut header = vec![0; len];
    recv.read_exact(&mut header).await.map_err(fail)?;

    protocol::decode(&header).map_err(fail)
}

pub(crate) async fn read_payload(
    recv: &mut quinn::RecvStream, len: u64, mut progress: impl FnMut(u64),
) -> Result<Vec<u8>, Error> {
    if len > MAX_MAP_LEN as u64 {
        return Err(fail(format!("a transfer of {len} bytes is too large")));
    }

    let mut payload = Vec::with_capacity(len as usize);
    progress(0);
    while let Some(chunk) = recv.read_chunk(CHUNK_LEN, true).await.map_err(fail)? {
        if (payload.len() + chunk.bytes.len()) as u64 > len {
            return Err(fail(format!("the transfer ran past its {len} bytes")));
        }

        payload.extend_from_slice(&chunk.bytes);
        progress(payload.len() as u64);
    }

    if (payload.len() as u64) < len {
        return Err(fail(format!(
            "the transfer ended after {} of {len} bytes",
            payload.len()
        )));
    }

    Ok(payload)
}
