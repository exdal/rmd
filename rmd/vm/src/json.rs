use base::types::{ListEntry, Value, strip_text_macros};
use std::fmt;

use serde::{
    Deserialize,
    de::{MapAccess, SeqAccess, Visitor},
};

use crate::{FaultKind, value::number_text};

pub(crate) const MAX_DEPTH: usize = 64;

pub fn decode(source: &str, budget: usize) -> Result<Value, FaultKind> {
    if source.len() > budget {
        return Err(FaultKind::Memory);
    }

    let mut source = source.as_bytes().to_vec();
    let value = simd_json::from_slice::<JsonValue>(&mut source)
        .map_err(|_| invalid())?
        .0;
    validate_depth(&value, 0)?;

    Ok(value)
}

pub(crate) fn write_number(out: &mut String, number: f32) {
    if number.is_finite() {
        out.push_str(&number_text(number));
        return;
    }

    let name = match number {
        number if number.is_nan() && number.is_sign_negative() => "-NaN",
        number if number.is_nan() => "NaN",
        number if number < 0.0 => "-Infinity",
        _ => "Infinity",
    };
    out.push_str("{\"__number__\":\"");
    out.push_str(name);
    out.push_str("\"}");
}

pub(crate) fn write_string(out: &mut String, text: &str) {
    out.push('"');
    for character in strip_text_macros(text).chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            character if character < ' ' => out.push_str(&format!("\\udc{:02x}", character as u32)),
            character if character.is_ascii() => out.push(character),
            character => {
                for unit in character.encode_utf16(&mut [0; 2]) {
                    out.push_str(&format!("\\u{unit:04x}"));
                }
            },
        }
    }

    out.push('"');
}

fn invalid() -> FaultKind {
    FaultKind::InvalidOperation("invalid JSON".into())
}

fn validate_depth(value: &Value, depth: usize) -> Result<(), FaultKind> {
    if depth >= MAX_DEPTH {
        return Err(FaultKind::Memory);
    }

    if let Value::List(entries) = value {
        for entry in entries {
            match &entry.value {
                Some(value) => validate_depth(value, depth + 1)?,
                None => validate_depth(&entry.key, depth + 1)?,
            }
        }
    }

    Ok(())
}

struct JsonValue(Value);

impl<'de> Deserialize<'de> for JsonValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(JsonVisitor)
    }
}

struct JsonVisitor;

impl<'de> Visitor<'de> for JsonVisitor {
    type Value = JsonValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(JsonValue(Value::Null))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(JsonValue(Value::Null))
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(JsonValue(Value::Num(if value { 1.0 } else { 0.0 })))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(JsonValue(Value::Num(value as f32)))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(JsonValue(Value::Num(value as f32)))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E> {
        Ok(JsonValue(Value::Num(value as f32)))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
        Ok(JsonValue(Value::Text(value.into())))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(JsonValue(Value::Text(value)))
    }

    fn visit_seq<A>(self, mut values: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut entries = Vec::with_capacity(values.size_hint().unwrap_or(0));
        while let Some(JsonValue(value)) = values.next_element()? {
            entries.push(ListEntry {
                key: value,
                value: None,
            });
        }
        Ok(JsonValue(Value::List(entries)))
    }

    fn visit_map<A>(self, mut values: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut entries = Vec::with_capacity(values.size_hint().unwrap_or(0));
        while let Some((key, JsonValue(value))) = values.next_entry::<String, JsonValue>()? {
            entries.push(ListEntry {
                key: Value::Text(key),
                value: Some(value),
            });
        }
        Ok(JsonValue(Value::List(entries)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(key: Value, value: Option<Value>) -> ListEntry {
        ListEntry { key, value }
    }

    #[test]
    fn decodes_nested_arrays_and_objects() {
        assert_eq!(
            decode(r#"[1,{"items":[true,null]}]"#, usize::MAX),
            Ok(Value::List(vec![
                entry(Value::Num(1.0), None),
                entry(
                    Value::List(vec![entry(
                        Value::Text("items".into()),
                        Some(Value::List(vec![
                            entry(Value::Num(1.0), None),
                            entry(Value::Null, None),
                        ])),
                    )]),
                    None,
                ),
            ]))
        );
    }

    #[test]
    fn rejects_nesting_at_the_depth_limit() {
        let accepted = format!("{}0{}", "[".repeat(63), "]".repeat(63));
        assert!(decode(&accepted, accepted.len()).is_ok());

        let rejected = format!("{}0{}", "[".repeat(64), "]".repeat(64));
        assert_eq!(decode(&rejected, rejected.len()), Err(FaultKind::Memory));
    }

    #[test]
    fn rejects_input_larger_than_the_budget() {
        assert_eq!(decode("[1]", 2), Err(FaultKind::Memory));
        assert!(decode("[1]", 3).is_ok());
    }

    #[test]
    fn maps_objects_to_associative_dm_lists() {
        assert_eq!(
            decode(r#"{"name":"Ada","active":false}"#, usize::MAX),
            Ok(Value::List(vec![
                entry(Value::Text("name".into()), Some(Value::Text("Ada".into()))),
                entry(Value::Text("active".into()), Some(Value::Num(0.0))),
            ]))
        );
    }
}
