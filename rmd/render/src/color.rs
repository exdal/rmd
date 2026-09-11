/// `color = "#ff0000"`, `color = "red"`
pub fn parse(text: &str) -> Option<[f32; 4]> {
    let text = text.trim();

    if let Some(hex) = text.strip_prefix('#') {
        return parse_hex(hex);
    }

    named(&text.to_ascii_lowercase())
}

fn parse_hex(hex: &str) -> Option<[f32; 4]> {
    let channel = |byte: u8| f32::from(byte) / 255.0;

    match hex.len() {
        // `#rgb` and `#rgba` repeat each nibble
        3 | 4 => {
            let mut out = [1.0; 4];
            for (slot, digit) in out.iter_mut().zip(hex.chars()) {
                let value = digit.to_digit(16)? as u8;
                *slot = channel(value * 17);
            }

            Some(out)
        },

        6 | 8 => {
            let mut out = [1.0; 4];
            for (slot, pair) in out.iter_mut().zip(hex.as_bytes().chunks_exact(2)) {
                let pair = std::str::from_utf8(pair).ok()?;
                *slot = channel(u8::from_str_radix(pair, 16).ok()?);
            }

            Some(out)
        },

        _ => None,
    }
}

fn named(name: &str) -> Option<[f32; 4]> {
    let rgb: [u8; 3] = match name {
        "black" => [0x00, 0x00, 0x00],
        "silver" => [0xc0, 0xc0, 0xc0],
        "gray" | "grey" => [0x80, 0x80, 0x80],
        "white" => [0xff, 0xff, 0xff],
        "maroon" => [0x80, 0x00, 0x00],
        "red" => [0xff, 0x00, 0x00],
        "purple" => [0x80, 0x00, 0x80],
        "fuchsia" | "magenta" => [0xff, 0x00, 0xff],
        "green" => [0x00, 0xc0, 0x00],
        "lime" => [0x00, 0xff, 0x00],
        "olive" => [0x80, 0x80, 0x00],
        "gold" => [0xff, 0xd7, 0x00],
        "yellow" => [0xff, 0xff, 0x00],
        "navy" => [0x00, 0x00, 0x80],
        "blue" => [0x00, 0x00, 0xff],
        "teal" => [0x00, 0x80, 0x80],
        "aqua" | "cyan" => [0x00, 0xff, 0xff],
        _ => return None,
    };

    Some([
        f32::from(rgb[0]) / 255.0,
        f32::from(rgb[1]) / 255.0,
        f32::from(rgb[2]) / 255.0,
        1.0,
    ])
}

#[cfg(test)]
mod tests {
    use crate::color::parse;

    #[test]
    fn parses_the_hex_forms() {
        assert_eq!(parse("#ff0000"), Some([1.0, 0.0, 0.0, 1.0]));
        assert_eq!(parse("#f00"), Some([1.0, 0.0, 0.0, 1.0]));
        assert_eq!(parse("#00ff0080"), Some([0.0, 1.0, 0.0, 128.0 / 255.0]));
        assert_eq!(parse("#0f08"), Some([0.0, 1.0, 0.0, 136.0 / 255.0]));
    }

    #[test]
    fn parses_named_colors_case_insensitively() {
        assert_eq!(parse("red"), Some([1.0, 0.0, 0.0, 1.0]));
        assert_eq!(parse("  Cyan "), Some([0.0, 1.0, 1.0, 1.0]));
        assert_eq!(parse("grey"), parse("gray"));
    }

    #[test]
    fn rejects_what_it_cannot_read() {
        assert_eq!(parse("#ff000"), None);
        assert_eq!(parse("#gggggg"), None);
        assert_eq!(parse("chartreuse"), None);
        assert_eq!(parse(""), None);
    }
}
