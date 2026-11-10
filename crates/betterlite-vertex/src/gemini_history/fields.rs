use betterlite_prompts::arguments::json::{Json, Object};

use crate::gemini_history::error::HistoryError;
use crate::gemini_media::mime::MimeHints;

pub const FORMAT: &str = "format";
pub const MIME_TYPE: &str = "mime_type";
pub const CONTENT_TYPE: &str = "content_type";
pub const DETAIL: &str = "detail";

pub fn optional_string(
    fields: &Object,
    name: &str,
    path: &str,
) -> Result<Option<String>, HistoryError> {
    match fields.get(name) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(value)) => Ok(Some(value.clone())),
        Some(_) => Err(HistoryError::WrongField {
            path: format!("{path}.{name}"),
            expected: "a string",
        }),
    }
}

pub fn required_string(fields: &Object, name: &str, path: &str) -> Result<String, HistoryError> {
    match fields.get(name) {
        Some(Json::Text(value)) => Ok(value.clone()),
        None | Some(Json::Null) => Err(HistoryError::MissingField {
            path: format!("{path}.{name}"),
        }),
        Some(_) => Err(HistoryError::WrongField {
            path: format!("{path}.{name}"),
            expected: "a string",
        }),
    }
}

pub fn required_object<'fields>(
    fields: &'fields Object,
    name: &str,
    path: &str,
) -> Result<&'fields Object, HistoryError> {
    match fields.get(name) {
        Some(Json::Object(object)) => Ok(object),
        None | Some(Json::Null) => Err(HistoryError::MissingField {
            path: format!("{path}.{name}"),
        }),
        Some(_) => Err(HistoryError::WrongField {
            path: format!("{path}.{name}"),
            expected: "an object",
        }),
    }
}

pub fn mime_hints(fields: &Object, path: &str) -> Result<MimeHints, HistoryError> {
    Ok(MimeHints {
        format: optional_string(fields, FORMAT, path)?,
        mime_type: optional_string(fields, MIME_TYPE, path)?,
        content_type: optional_string(fields, CONTENT_TYPE, path)?,
    })
}
