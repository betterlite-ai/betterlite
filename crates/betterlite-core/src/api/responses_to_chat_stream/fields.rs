use serde_json::{Map, Value};

use super::error::ResponsesChatStreamError;
use super::translate::ToolCallIndices;

pub type Fields = Map<String, Value>;

pub trait FieldValue<'a>: Sized {
    fn read(value: &'a Value) -> Option<Self>;
}

impl<'a> FieldValue<'a> for &'a str {
    fn read(value: &'a Value) -> Option<Self> {
        value.as_str()
    }
}

impl<'a> FieldValue<'a> for &'a Fields {
    fn read(value: &'a Value) -> Option<Self> {
        value.as_object()
    }
}

impl FieldValue<'_> for i64 {
    fn read(value: &Value) -> Option<Self> {
        value.as_i64()
    }
}

pub fn field<'a, T: FieldValue<'a>>(
    fields: &'a Fields,
    name: &'static str,
) -> Result<Option<T>, ResponsesChatStreamError> {
    match fields.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => T::read(value)
            .map(Some)
            .ok_or(ResponsesChatStreamError::Field { field: name }),
    }
}

#[must_use]
pub fn listed_choices(fields: &Fields) -> bool {
    match fields.get("choices") {
        Some(Value::Array(items)) => !items.is_empty(),
        None
        | Some(
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::Object(_) | Value::String(_),
        ) => false,
    }
}

#[must_use]
pub fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Array(_) => "list",
        Value::Bool(_) => "bool",
        Value::Number(number) if number.is_i64() || number.is_u64() => "int",
        Value::Number(_) => "float",
        Value::Object(_) => "dict",
        Value::String(_) => "str",
        Value::Null => "NoneType",
    }
}

pub fn sequential_index(
    map: Option<&mut ToolCallIndices>,
    output_index: i64,
) -> Result<i64, ResponsesChatStreamError> {
    let Some(entries) = map else {
        return Ok(output_index);
    };
    if let Some(index) = entries.get(&output_index) {
        return Ok(*index);
    }
    let next = i64::try_from(entries.len())
        .map_err(|source| ResponsesChatStreamError::IndexCount { source })?;
    entries.insert(output_index, next);
    Ok(next)
}
