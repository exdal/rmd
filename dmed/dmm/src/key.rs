/// `"aab"`
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Key(pub u32);

const ALPHABET: &[u8; 52] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
const BASE: u32 = 52;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidKey;

impl std::fmt::Display for InvalidKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "invalid map key") }
}

impl std::error::Error for InvalidKey {}

impl Key {
    pub fn parse(s: &str) -> Result<Self, InvalidKey> { Self::parse_bytes(s.as_bytes()) }

    pub fn parse_bytes(bytes: &[u8]) -> Result<Self, InvalidKey> {
        let mut value: u32 = 0;

        for byte in bytes {
            let digit = match byte {
                b'a'..=b'z' => u32::from(byte - b'a'),
                b'A'..=b'Z' => u32::from(byte - b'A') + 26,
                _ => return Err(InvalidKey),
            };

            value = value
                .checked_mul(BASE)
                .and_then(|v| v.checked_add(digit))
                .ok_or(InvalidKey)?;
        }

        Ok(Self(value))
    }

    pub fn write_to(self, out: &mut String, length: usize) {
        let start = out.len();
        let mut value = self.0;

        for _ in 0..length {
            let digit = ALPHABET.get((value % BASE) as usize).copied().unwrap_or(b'a');
            out.insert(start, char::from(digit));
            value /= BASE;
        }
    }

    pub fn to_string_with_length(self, length: usize) -> String {
        let mut out = String::with_capacity(length);
        self.write_to(&mut out, length);

        out
    }

    pub fn length_for(count: usize) -> usize {
        let mut length = 1;
        let mut capacity = BASE as usize;

        while capacity < count {
            capacity = capacity.saturating_mul(BASE as usize);
            length += 1;
        }

        length
    }
}

#[cfg(test)]
mod tests {
    use crate::key::Key;

    #[test]
    fn round_trips() {
        for raw in ["aaa", "aab", "abc", "zZZ"] {
            let key = Key::parse(raw).unwrap();
            assert_eq!(key.to_string_with_length(raw.len()), raw);
        }
    }

    #[test]
    fn sizes_the_key_space() {
        assert_eq!(Key::length_for(52), 1);
        assert_eq!(Key::length_for(53), 2);
        assert_eq!(Key::length_for(2704), 2);
        assert_eq!(Key::length_for(2705), 3);
    }

    #[test]
    fn encodes_a_zero_length_key_as_nothing() {
        assert_eq!(Key(0).to_string_with_length(0), "");
    }

    #[test]
    fn appends_rather_than_replacing() {
        let mut out = String::from("(");
        Key::parse("ab").unwrap().write_to(&mut out, 2);
        assert_eq!(out, "(ab");
    }
}
