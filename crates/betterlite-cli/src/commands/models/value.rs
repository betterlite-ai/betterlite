use betterlite_models::controls::json_value::JsonValue;
use indexmap::IndexMap;
use rust_decimal::Decimal;

use super::ModelsError;

pub(super) type ModelObject = IndexMap<String, JsonValue>;
type ObjectResult<'value> = Result<Option<&'value ModelObject>, ModelsError>;
type ValueResult<T> = Result<Option<T>, ModelsError>;

pub(super) fn cell_text(value: &JsonValue) -> String {
    match value {
        JsonValue::Null => String::new(),
        JsonValue::Text(text) => text.clone(),
        JsonValue::Bool(flag) => flag.to_string(),
        JsonValue::Number(number) => number.to_string(),
        JsonValue::Array(_) | JsonValue::Object(_) => {
            serde_json::to_string(value).unwrap_or_default()
        }
    }
}

pub(super) const fn object_value<'value>(
    value: Option<&'value JsonValue>,
    field: &'static str,
) -> ObjectResult<'value> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Object(fields)) => Ok(Some(fields)),
        Some(_) => Err(ModelsError::FieldType { field }),
    }
}

pub(super) fn cost_value(value: Option<&JsonValue>, field: &'static str) -> ValueResult<Decimal> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Number(number)) => number
            .to_string()
            .parse::<Decimal>()
            .map(Some)
            .map_err(|source| ModelsError::Number { field, source }),
        Some(_) => Err(ModelsError::FieldType { field }),
    }
}
