pub mod frame;
pub mod live;

use serde::{Deserialize, Serialize, de::DeserializeOwned};

pub use crate::frame::{FrameReader, MAX_FRAME_LEN, encode_frame, write_frame};

pub const MAGIC: [u8; 4] = *b"RMD\0";
pub const VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Service {
    LiveShare,
    Bridge,
}

/// The first frame on every connection, whatever the transport
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub magic: [u8; 4],
    pub version: u16,
    pub service: Service,
}

impl Hello {
    pub const fn new(service: Service) -> Self {
        Self {
            magic: MAGIC,
            version: VERSION,
            service,
        }
    }

    pub fn check(&self, service: Service) -> Result<(), Error> {
        if self.magic != MAGIC {
            return Err(Error::BadMagic);
        }

        if self.version != VERSION {
            return Err(Error::Version {
                ours: VERSION,
                theirs: self.version,
            });
        }

        if self.service != service {
            return Err(Error::WrongService {
                expected: service,
                got: self.service,
            });
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Codec(postcard::Error),
    FrameTooLarge { len: usize, max: usize },
    BadMagic,
    Version { ours: u16, theirs: u16 },
    WrongService { expected: Service, got: Service },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Codec(e) => write!(f, "malformed message: {e}"),
            Self::FrameTooLarge { len, max } => write!(f, "frame of {len} bytes exceeds the {max} byte limit"),
            Self::BadMagic => write!(f, "peer does not speak the rmd protocol"),
            Self::Version { ours, theirs } => {
                write!(f, "protocol version mismatch: ours is {ours}, theirs is {theirs}")
            },
            Self::WrongService { expected, got } => write!(f, "expected the {expected:?} service, got {got:?}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<postcard::Error> for Error {
    fn from(e: postcard::Error) -> Self { Self::Codec(e) }
}

pub fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, Error> { Ok(postcard::to_stdvec(value)?) }

pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, Error> { Ok(postcard::from_bytes(bytes)?) }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_round_trips_and_checks() {
        let hello = Hello::new(Service::LiveShare);
        let decoded: Hello = decode(&encode(&hello).unwrap()).unwrap();

        assert_eq!(decoded, hello);
        assert_eq!(decoded.check(Service::LiveShare), Ok(()));
    }

    #[test]
    fn hello_rejects_foreign_peers() {
        let mut hello = Hello::new(Service::LiveShare);
        hello.version = VERSION + 1;
        assert_eq!(
            hello.check(Service::LiveShare),
            Err(Error::Version {
                ours: VERSION,
                theirs: VERSION + 1
            })
        );

        assert_eq!(
            Hello::new(Service::Bridge).check(Service::LiveShare),
            Err(Error::WrongService {
                expected: Service::LiveShare,
                got: Service::Bridge
            })
        );

        let mut hello = Hello::new(Service::LiveShare);
        hello.magic = *b"HTTP";
        assert_eq!(hello.check(Service::LiveShare), Err(Error::BadMagic));
    }

    #[test]
    fn garbage_is_a_codec_error() {
        assert!(matches!(decode::<Hello>(&[0xff; 3]), Err(Error::Codec(_))));
    }
}
