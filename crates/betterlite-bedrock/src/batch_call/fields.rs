use betterlite_models::decisions::Json;
use betterlite_openai::batches::records::BatchStatus;
use indexmap::IndexMap;
use serde_json::Number;

use super::error::BatchCallError;

type Fields = IndexMap<String, Json>;
type Parsed<T> = Result<T, BatchCallError>;

// the end time fills only the field of the status that names it
pub fn matching_time(status: BatchStatus, wanted: BatchStatus, time: Option<i64>) -> Option<i64> {
    if status == wanted {
        return time;
    }
    None
}

pub fn required_object<'json>(
    fields: &'json IndexMap<String, Json>,
    field: &'static str,
) -> Result<&'json IndexMap<String, Json>, BatchCallError> {
    let value = fields
        .get(field)
        .ok_or(BatchCallError::MissingField { field })?;
    let Json::Object(entries) = value else {
        return Err(BatchCallError::Field { field });
    };
    Ok(entries)
}

pub fn required_text(fields: &Fields, field: &'static str) -> Parsed<String> {
    match text_field(fields, field)? {
        Some(text) => Ok(text),
        None => Err(BatchCallError::MissingField { field }),
    }
}

pub fn required_integer(fields: &Fields, field: &'static str) -> Parsed<i64> {
    match fields.get(field) {
        Some(value) => integer(value, field),
        None => Err(BatchCallError::MissingField { field }),
    }
}

pub fn required_optional_integer(fields: &Fields, field: &'static str) -> Parsed<Option<i64>> {
    if !fields.contains_key(field) {
        return Err(BatchCallError::MissingField { field });
    }
    optional_integer(fields, field)
}

pub fn optional_integer(fields: &Fields, field: &'static str) -> Parsed<Option<i64>> {
    fields
        .get(field)
        .filter(|value| !matches!(value, Json::Null))
        .map(|value| integer(value, field))
        .transpose()
}

fn integer(value: &Json, field: &'static str) -> Parsed<i64> {
    let Json::Number(number) = value else {
        return Err(BatchCallError::Field { field });
    };
    number.as_i64().ok_or(BatchCallError::Field { field })
}

pub fn text_field(
    fields: &IndexMap<String, Json>,
    field: &'static str,
) -> Result<Option<String>, BatchCallError> {
    match fields.get(field) {
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(Json::Null) | None => Ok(None),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(BatchCallError::Field { field })
        }
    }
}

// response.get("message") or "" hands every falsey value to the metadata as the empty string
pub fn falsey_text(
    fields: &IndexMap<String, Json>,
    field: &'static str,
) -> Result<String, BatchCallError> {
    match fields.get(field) {
        None | Some(Json::Null) => Ok(String::new()),
        Some(Json::Text(text)) => Ok(text.clone()),
        Some(Json::Bool(false)) => Ok(String::new()),
        Some(Json::Bool(true)) => Err(BatchCallError::Field { field }),
        Some(Json::Number(number)) if zero(number) => Ok(String::new()),
        Some(Json::Number(_)) => Err(BatchCallError::Field { field }),
        Some(Json::List(items)) if items.is_empty() => Ok(String::new()),
        Some(Json::List(_)) => Err(BatchCallError::Field { field }),
        Some(Json::Object(entries)) if entries.is_empty() => Ok(String::new()),
        Some(Json::Object(_)) => Err(BatchCallError::Field { field }),
    }
}

// both 0 and 0.0 are falsey in Python, and -0.0's absolute value has no bits set either
fn zero(number: &Number) -> bool {
    match number.as_i64() {
        Some(value) => value == 0,
        None => number
            .as_f64()
            .is_some_and(|value| value.abs().to_bits() == 0),
    }
}
