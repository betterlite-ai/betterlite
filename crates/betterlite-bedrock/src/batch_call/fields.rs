use betterlite_models::decisions::Json;
use betterlite_openai::batches::records::BatchStatus;
use indexmap::IndexMap;
use serde_json::Number;

use super::error::BatchCallError;

// completed_at, failed_at, cancelled_at and expired_at carry the job's end time only in the
// status that names them
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
    match fields.get(field) {
        None => Err(BatchCallError::MissingField { field }),
        Some(Json::Object(entries)) => Ok(entries),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(BatchCallError::Field { field })
        }
    }
}

pub fn required_text(
    fields: &IndexMap<String, Json>,
    field: &'static str,
) -> Result<String, BatchCallError> {
    match fields.get(field) {
        None => Err(BatchCallError::MissingField { field }),
        Some(Json::Text(text)) => Ok(text.clone()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(BatchCallError::Field { field })
        }
    }
}

pub fn required_integer(
    fields: &IndexMap<String, Json>,
    field: &'static str,
) -> Result<i64, BatchCallError> {
    match fields.get(field) {
        None => Err(BatchCallError::MissingField { field }),
        Some(Json::Number(number)) => number.as_i64().ok_or(BatchCallError::Field { field }),
        Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(BatchCallError::Field { field })
        }
    }
}

pub fn required_optional_integer(
    fields: &IndexMap<String, Json>,
    field: &'static str,
) -> Result<Option<i64>, BatchCallError> {
    match fields.get(field) {
        None => Err(BatchCallError::MissingField { field }),
        Some(Json::Null) => Ok(None),
        Some(value) => integer(value, field).map(Some),
    }
}

pub fn optional_integer(
    fields: &IndexMap<String, Json>,
    field: &'static str,
) -> Result<Option<i64>, BatchCallError> {
    match fields.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(value) => integer(value, field).map(Some),
    }
}

fn integer(value: &Json, field: &'static str) -> Result<i64, BatchCallError> {
    match value {
        Json::Number(number) => number.as_i64().ok_or(BatchCallError::Field { field }),
        Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => {
            Err(BatchCallError::Field { field })
        }
    }
}

pub fn text_field(
    fields: &IndexMap<String, Json>,
    field: &'static str,
) -> Result<Option<String>, BatchCallError> {
    match fields.get(field) {
        None => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
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
