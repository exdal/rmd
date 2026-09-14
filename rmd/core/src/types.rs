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
