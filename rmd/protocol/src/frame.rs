use serde::{Serialize, de::DeserializeOwned};

use crate::Error;

pub const MAX_FRAME_LEN: usize = 16 * 1024 * 1024;
const HEADER_LEN: usize = size_of::<u32>();

pub fn write_frame(out: &mut Vec<u8>, payload: &[u8]) -> Result<(), Error> {
    if payload.len() > MAX_FRAME_LEN {
        return Err(Error::FrameTooLarge {
            len: payload.len(),
            max: MAX_FRAME_LEN,
        });
    }

    out.reserve(HEADER_LEN + payload.len());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(payload);

    Ok(())
}

pub fn encode_frame<T: Serialize>(value: &T) -> Result<Vec<u8>, Error> {
    let payload = crate::encode(value)?;
    let mut out = Vec::new();
    write_frame(&mut out, &payload)?;

    Ok(out)
}

pub fn split_frame(bytes: &[u8]) -> Result<(&[u8], &[u8]), Error> {
    let header = bytes.first_chunk::<HEADER_LEN>().ok_or(Error::Truncated)?;
    let len = u32::from_le_bytes(*header) as usize;
    if len > MAX_FRAME_LEN {
        return Err(Error::FrameTooLarge {
            len,
            max: MAX_FRAME_LEN,
        });
    }

    let rest = &bytes[HEADER_LEN..];
    if rest.len() < len {
        return Err(Error::Truncated);
    }

    Ok(rest.split_at(len))
}

#[derive(Debug)]
pub struct FrameReader {
    buffer: Vec<u8>,
    start: usize,
    max: usize,
}

impl Default for FrameReader {
    fn default() -> Self { Self::new() }
}

impl FrameReader {
    pub const fn new() -> Self { Self::with_max(MAX_FRAME_LEN) }

    pub const fn with_max(max: usize) -> Self {
        Self {
            buffer: Vec::new(),
            start: 0,
            max,
        }
    }

    pub fn push(&mut self, bytes: &[u8]) {
        if self.start > 0 && self.start == self.buffer.len() {
            self.buffer.clear();
            self.start = 0;
        }

        self.buffer.extend_from_slice(bytes);
    }

    pub fn next_frame(&mut self) -> Result<Option<&[u8]>, Error> {
        let pending = &self.buffer[self.start..];
        let Some(header) = pending.first_chunk::<HEADER_LEN>() else {
            self.compact();
            return Ok(None);
        };

        let len = u32::from_le_bytes(*header) as usize;
        if len > self.max {
            return Err(Error::FrameTooLarge { len, max: self.max });
        }

        if pending.len() < HEADER_LEN + len {
            self.compact();
            return Ok(None);
        }

        let begin = self.start + HEADER_LEN;
        self.start = begin + len;

        Ok(Some(&self.buffer[begin..self.start]))
    }

    pub fn next_message<T: DeserializeOwned>(&mut self) -> Result<Option<T>, Error> {
        self.next_frame()?.map(crate::decode).transpose()
    }

    fn compact(&mut self) {
        if self.start > 0 {
            self.buffer.drain(..self.start);
            self.start = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::slice;

    use super::*;

    fn framed(payloads: &[&[u8]]) -> Vec<u8> {
        let mut out = Vec::new();
        for payload in payloads {
            write_frame(&mut out, payload).unwrap();
        }

        out
    }

    #[test]
    fn reads_back_to_back_frames() {
        let mut reader = FrameReader::new();
        reader.push(&framed(&[b"one", b"", b"three"]));

        assert_eq!(reader.next_frame().unwrap(), Some(&b"one"[..]));
        assert_eq!(reader.next_frame().unwrap(), Some(&b""[..]));
        assert_eq!(reader.next_frame().unwrap(), Some(&b"three"[..]));
        assert_eq!(reader.next_frame().unwrap(), None);
    }

    #[test]
    fn reassembles_frames_split_at_every_byte() {
        let bytes = framed(&[b"hello", b"world!"]);
        let mut reader = FrameReader::new();
        let mut frames = Vec::new();

        for byte in &bytes {
            reader.push(slice::from_ref(byte));
            while let Some(frame) = reader.next_frame().unwrap() {
                frames.push(frame.to_vec());
            }
        }

        assert_eq!(frames, [b"hello".to_vec(), b"world!".to_vec()]);
    }

    #[test]
    fn rejects_oversized_frames_from_the_header_alone() {
        let mut reader = FrameReader::with_max(4);
        reader.push(&5u32.to_le_bytes());

        assert_eq!(reader.next_frame(), Err(Error::FrameTooLarge { len: 5, max: 4 }));
    }

    #[test]
    fn a_frame_splits_off_its_trailing_payload() {
        let mut bytes = framed(&[b"head"]);
        bytes.extend_from_slice(b"raw payload");

        assert_eq!(split_frame(&bytes), Ok((&b"head"[..], &b"raw payload"[..])));
        assert_eq!(split_frame(&bytes[..5]), Err(Error::Truncated));
        assert_eq!(split_frame(&[1, 0]), Err(Error::Truncated));
    }

    #[test]
    fn typed_messages_round_trip_through_frames() {
        let hello = crate::Hello::new(crate::Service::Bridge);
        let mut reader = FrameReader::new();
        reader.push(&encode_frame(&hello).unwrap());

        assert_eq!(reader.next_message::<crate::Hello>().unwrap(), Some(hello));
        assert_eq!(reader.next_message::<crate::Hello>().unwrap(), None);
    }
}
