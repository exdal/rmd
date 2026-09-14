pub use crate::interner::Symbol as Identifier;
use crate::path::TreePath;

/// Declaration modifiers on a `var/`.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct VarModifiers {
    pub is_const: bool,
    pub is_final: bool,
    pub is_global: bool,
    pub is_static: bool,
    pub is_tmp: bool,
}

/// A constant-folded DM value.
///
/// The editor only ever needs the constant subset - `icon`, `icon_state`, `dir`, `pixel_x`, `name`
/// and friends are all literals in practice, and anything that isn't gets rendered as a placeholder.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    /// BYOND numbers are 32 bit floats, not doubles. Keep the precision loss visible.
    Num(f32),
    Text(String),
    /// `'icons/obj/items.dmi'`
    Resource(String),
    Path(TreePath),
    List(Vec<ListEntry>),
    /// The initializer exists but isn't a compile time constant.
    Unevaluated,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListEntry {
    pub key: Value,
    pub value: Option<Value>,
}

impl Value {
    pub fn as_num(&self) -> Option<f32> {
        match self {
            Value::Num(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_text(&self) -> Option<&str> {
        match self {
            Value::Text(s) | Value::Resource(s) => Some(s),
            _ => None,
        }
    }

    /// DM truthiness: `null`, `0` and `""` are false, everything else is true.
    pub fn is_truthy(&self) -> bool {
        match self {
            Value::Null => false,
            Value::Num(v) => *v != 0.0,
            Value::Text(s) => !s.is_empty(),
            _ => true,
        }
    }
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Null => write!(f, "null"),
            Value::Num(v) => write!(f, "{v}"),
            Value::Text(s) => write!(f, "\"{s}\""),
            Value::Resource(s) => write!(f, "'{s}'"),
            Value::Path(p) => write!(f, "{p}"),
            Value::List(_) => write!(f, "list(...)"),
            Value::Unevaluated => write!(f, "<runtime>"),
        }
    }
}

pub fn decode_string(source: &str) -> String {
    let mut out = String::new();
    let mut chars = source.chars();

    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);

            continue;
        }

        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some(ch @ ('\\' | '"' | '[' | ']' | '\'')) => out.push(ch),
            Some(ch) => {
                out.push('\\');
                out.push(ch);
            },
            None => out.push('\\'),
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::decode_string;

    #[test]
    fn decodes_the_escapes_dm_defines() {
        assert_eq!(decode_string(r"a\nb"), "a\nb");
        assert_eq!(decode_string(r"a\tb"), "a\tb");
        assert_eq!(decode_string(r"a\rb"), "a\rb");
        assert_eq!(decode_string(r"a\\b"), r"a\b");
        assert_eq!(decode_string(r#"a\"b"#), "a\"b");
        assert_eq!(decode_string(r"a\'b"), "a'b");
    }

    /// `"\[x]"` is how DM writes a literal bracket where interpolation would otherwise start.
    #[test]
    fn decodes_escaped_interpolation_brackets() {
        assert_eq!(decode_string(r"\[x\]"), "[x]");
    }

    #[test]
    fn leaves_undefined_escapes_as_written() {
        assert_eq!(decode_string(r"a\qb"), r"a\qb");
        assert_eq!(decode_string(r"100\%"), r"100\%");
    }

    /// A string ending mid-escape must not drop the backslash or panic.
    #[test]
    fn a_trailing_backslash_survives() {
        assert_eq!(decode_string(r"ab\"), r"ab\");
        assert_eq!(decode_string(r"\"), r"\");
    }

    #[test]
    fn passes_through_text_with_no_escapes() {
        assert_eq!(decode_string(""), "");
        assert_eq!(decode_string("icon_state"), "icon_state");
    }
}
