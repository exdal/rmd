use core::types::{ListEntry, Value};

use crate::FaultKind;

const MAX_DEPTH: usize = 64;

pub fn decode(source: &str, budget: usize) -> Result<Value, FaultKind> {
    if source.len() > budget {
        return Err(FaultKind::Memory);
    }

    let table = source
        .parse::<toml::Table>()
        .map_err(|error| FaultKind::InvalidOperation(error.message().to_string()))?;

    convert(toml::Value::Table(table), 0)
}

fn convert(value: toml::Value, depth: usize) -> Result<Value, FaultKind> {
    if depth >= MAX_DEPTH {
        return Err(FaultKind::Memory);
    }

    Ok(match value {
        toml::Value::String(text) => Value::Text(text),
        toml::Value::Integer(number) => Value::Num(number as f32),
        toml::Value::Float(number) => Value::Num(number as f32),
        toml::Value::Boolean(flag) => Value::Num(if flag { 1.0 } else { 0.0 }),
        toml::Value::Datetime(datetime) => Value::Text(datetime.to_string()),
        toml::Value::Array(values) => Value::List(
            values
                .into_iter()
                .map(|value| {
                    Ok(ListEntry {
                        key: convert(value, depth + 1)?,
                        value: None,
                    })
                })
                .collect::<Result<_, FaultKind>>()?,
        ),
        toml::Value::Table(table) => Value::List(
            table
                .into_iter()
                .map(|(key, value)| {
                    Ok(ListEntry {
                        key: Value::Text(key),
                        value: Some(convert(value, depth + 1)?),
                    })
                })
                .collect::<Result<_, FaultKind>>()?,
        ),
    })
}
