pub use crate::interner::Symbol as Identifier;
use crate::path::TreePath;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProcId(pub u32);

impl ProcId {
    pub const INVALID: Self = Self(u32::MAX);

    pub const fn is_valid(self) -> bool { self.0 != u32::MAX }

    pub const fn is_invalid(self) -> bool { self.0 == u32::MAX }
}

impl std::fmt::Display for ProcId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.is_valid() {
            true => write!(f, "%{}", self.0),
            false => f.write_str("%-"),
        }
    }
}

impl std::fmt::Debug for ProcId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { std::fmt::Display::fmt(self, f) }
}

/// this points to an IR arena in Module
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct IrNodeId(pub u32);

impl IrNodeId {
    pub const INVALID: Self = Self(u32::MAX);

    pub const fn is_valid(self) -> bool { self.0 != u32::MAX }

    pub const fn is_invalid(self) -> bool { self.0 == u32::MAX }
}

impl std::fmt::Display for IrNodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.is_valid() {
            true => write!(f, "%{}", self.0),
            false => f.write_str("%-"),
        }
    }
}

impl std::fmt::Debug for IrNodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { std::fmt::Display::fmt(self, f) }
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcKind {
    Proc,
    Verb,
    Override,
    /// `operator+`, `operator[]`, ...
    Operator,
}

#[derive(Debug, Clone)]
pub struct ProcParam<E> {
    pub spec: VarSpec<E>,
    pub default: Option<E>,
    /// `in list(...)`
    pub in_list: Option<E>,
}

#[derive(Default, Debug, Clone)]
pub struct TypeSpec {
    pub flags: InputType,
    pub path: Option<TreePath>,
}

#[derive(Debug, Clone)]
pub struct VarSpec<E> {
    pub name: Identifier,
    pub var_type: Option<TreePath>,
    pub modifiers: VarModifiers,
    /// `var/items[5][]`
    pub dimensions: Vec<Option<E>>,
    /// `as text|null` or `as /mob/player`
    pub as_type: Option<TypeSpec>,
}

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputType(pub u32);

impl InputType {
    pub const ANYTHING: Self = Self(1 << 5);
    pub const AREA: Self = Self(1 << 13);
    pub const COLOR: Self = Self(1 << 7);
    pub const COMMAND_TEXT: Self = Self(1 << 8);
    pub const FILE: Self = Self(1 << 3);
    pub const ICON: Self = Self(1 << 6);
    pub const KEY: Self = Self(1 << 4);
    pub const MESSAGE: Self = Self(1 << 9);
    pub const MOB: Self = Self(1 << 1);
    pub const NONE: Self = Self(0);
    pub const NULL: Self = Self(1 << 10);
    pub const NUM: Self = Self(1 << 0);
    pub const OBJ: Self = Self(1 << 2);
    pub const PASSWORD: Self = Self(1 << 14);
    pub const PATH: Self = Self(1 << 16);
    pub const SOUND: Self = Self(1 << 15);
    pub const TEXT: Self = Self(1 << 11);
    pub const TURF: Self = Self(1 << 12);

    pub fn contains(self, other: Self) -> bool { (self.0 & other.0) == other.0 }

    pub fn from_word(word: &str) -> Option<Self> {
        Some(match word {
            "anything" => Self::ANYTHING,
            "area" => Self::AREA,
            "color" => Self::COLOR,
            "command_text" => Self::COMMAND_TEXT,
            "file" => Self::FILE,
            "icon" => Self::ICON,
            "key" => Self::KEY,
            "message" => Self::MESSAGE,
            "mob" => Self::MOB,
            "null" => Self::NULL,
            "num" => Self::NUM,
            "obj" => Self::OBJ,
            "password" => Self::PASSWORD,
            "path" => Self::PATH,
            "sound" => Self::SOUND,
            "text" => Self::TEXT,
            "turf" => Self::TURF,
            _ => return None,
        })
    }
}

impl std::ops::BitOr for InputType {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self { Self(self.0 | rhs.0) }
}

impl std::ops::BitOrAssign for InputType {
    fn bitor_assign(&mut self, rhs: Self) { self.0 |= rhs.0; }
}
#[cfg(test)]
mod tests {
    use super::{InputType, IrNodeId, ProcId, decode_string};

    /// The disassembly form comes off `Display`, so `Debug` on a container prints it too.
    #[test]
    fn ids_print_as_their_disassembly_form() {
        assert_eq!(format!("{}", IrNodeId(3)), "%3");
        assert_eq!(format!("{:?}", IrNodeId(3)), "%3");
        assert_eq!(format!("{:?}", Some(IrNodeId(3))), "Some(%3)");
        assert_eq!(format!("{:?}", vec![ProcId(0), ProcId(1)]), "[%0, %1]");
    }

    #[test]
    fn an_invalid_id_is_distinguishable_and_prints_as_a_hole() {
        assert!(!IrNodeId::INVALID.is_valid());
        assert!(IrNodeId::INVALID.is_invalid());
        assert!(IrNodeId(0).is_valid());
        assert_eq!(format!("{}", IrNodeId::INVALID), "%-");
        assert_eq!(format!("{}", ProcId::INVALID), "%-");
    }

    #[test]
    fn input_type_flags_combine_and_parse() {
        let flags = InputType::from_word("mob").expect("mob") | InputType::from_word("null").expect("null");

        assert!(flags.contains(InputType::MOB));
        assert!(flags.contains(InputType::NULL));
        assert!(!flags.contains(InputType::TURF));
        assert_eq!(InputType::from_word("nonsense"), None);
    }

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
