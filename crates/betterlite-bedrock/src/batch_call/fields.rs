use betterlite_models::decisions::Json;
use betterlite_openai::batches::records::BatchStatus;
use indexmap::IndexMap;

use crate::converse_events::wire::non_empty;

use super::error::BatchCallError;

type Fields = IndexMap<String, Json>;
type Parsed<T> = Result<T, BatchCallError>;

// the end time fills only the field of the status that names it
#[must_use]
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
    text_field(fields, field)?.map_or_else(|| Err(BatchCallError::MissingField { field }), Ok)
}

pub fn required_integer(fields: &Fields, field: &'static str) -> Parsed<i64> {
    fields.get(field).map_or_else(
        || Err(BatchCallError::MissingField { field }),
        |value| integer(value, field),
    )
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
    let Some(value) = fields.get(field) else {
        return Ok(String::new());
    };
    if !non_empty(value) {
        return Ok(String::new());
    }
    let Json::Text(text) = value else {
        return Err(BatchCallError::Field { field });
    };
    Ok(text.clone())
}
