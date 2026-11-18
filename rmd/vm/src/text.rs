use base::types::TEXT_MACRO;
use std::iter::repeat_n;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Bytes,
    Chars,
}

fn width(character: char) -> usize {
    if character == TEXT_MACRO {
        1
    } else {
        character.len_utf8()
    }
}

fn is_ascii_before(text: &str, position: usize) -> bool {
    text.as_bytes().get(..position).is_some_and(<[u8]>::is_ascii)
}

pub fn len(text: &str, unit: Unit) -> usize {
    match unit {
        Unit::Bytes if text.is_ascii() => text.len(),
        Unit::Bytes => text.chars().map(width).sum(),
        Unit::Chars => text.chars().count(),
    }
}

pub fn offset(text: &str, index: usize, unit: Unit) -> usize {
    len(text.get(..index).unwrap_or(text), unit)
}

// a byte inside a multi-byte character is one U+FFFD, so lengths stay consistent
pub fn slice(text: &str, start: usize, end: usize, unit: Unit) -> String {
    if start >= end {
        return String::new();
    }

    match unit {
        Unit::Chars => text.chars().skip(start).take(end - start).collect(),
        Unit::Bytes if is_ascii_before(text, end) => text.get(start..end).unwrap_or_default().to_string(),
        Unit::Bytes => {
            let (index, mut position) = if is_ascii_before(text, start) {
                (start, start)
            } else {
                (0, 0)
            };
            let mut out = String::new();
            for character in text.get(index..).unwrap_or_default().chars() {
                let next = position + width(character);
                if position >= end {
                    break;
                }

                let covered = next.min(end).saturating_sub(position.max(start));
                if covered == width(character) {
                    out.push(character);
                } else {
                    out.extend(repeat_n(TEXT_MACRO, covered));
                }

                position = next;
            }

            out
        },
    }
}

pub fn char_at(text: &str, position: usize, unit: Unit) -> Option<char> {
    match unit {
        Unit::Chars => text.chars().nth(position),
        Unit::Bytes if is_ascii_before(text, position) => text.get(position..)?.chars().next(),
        Unit::Bytes => {
            let mut start = 0;
            for character in text.chars() {
                let next = start + width(character);
                if position < next {
                    return Some(if position == start { character } else { TEXT_MACRO });
                }

                start = next;
            }

            None
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{Unit::*, char_at, len, offset, slice};

    const W: &str = "çay1";

    #[test]
    fn ascii_maps_one_to_one() {
        assert_eq!(len("abc", Bytes), 3);
        assert_eq!(slice("abcd", 1, 3, Bytes), "bc");
        assert_eq!(char_at("abc", 1, Bytes), Some('b'));
        assert_eq!(char_at("abc", 3, Bytes), None);
        assert_eq!(offset("abcd", 2, Bytes), 2);
    }

    #[test]
    fn multibyte_characters_count_their_bytes() {
        assert_eq!((len(W, Bytes), len(W, Chars)), (5, 4));
        assert_eq!(char_at(W, 0, Bytes), Some('ç'));
        assert_eq!(char_at(W, 1, Bytes), Some('\u{FFFD}'));
        assert_eq!(char_at(W, 2, Bytes), Some('a'));
        assert_eq!(char_at(W, 1, Chars), Some('a'));
        assert_eq!(offset(W, 'ç'.len_utf8(), Bytes), 2);
        assert_eq!(offset(W, 'ç'.len_utf8(), Chars), 1);
    }

    #[test]
    fn the_replacement_character_is_one_byte() {
        assert_eq!(len("a\u{FFFD}b", Bytes), 3);
        assert_eq!(char_at("a\u{FFFD}b", 2, Bytes), Some('b'));
        assert_eq!(slice("a\u{FFFD}b", 1, 3, Bytes), "\u{FFFD}b");
    }

    #[test]
    fn cuts_inside_a_character_leave_one_replacement_per_byte() {
        assert_eq!(slice(W, 0, 1, Bytes), "\u{FFFD}");
        assert_eq!(slice(W, 1, 5, Bytes), "\u{FFFD}ay1");
        assert_eq!(slice(W, 0, 2, Bytes), "ç");
        assert_eq!(slice(W, 0, 1, Chars), "ç");
        assert_eq!(slice("😀", 1, 3, Bytes), "\u{FFFD}\u{FFFD}");
        assert_eq!(slice("ab", 1, 1, Bytes), "");
    }
}
