use std::str::from_utf8;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::{Map, Value};

use crate::api::api_boundaries::ApiLimits;

use super::BatchOutputContent;
use super::error::{BatchLineError, BatchOutputUsageError};

#[derive(Debug, Clone, PartialEq)]
pub enum DecodedBatchLine {
    Entry(Json),
    Malformed { message: String },
    NonObject { kind: &'static str },
}

pub(super) fn decoded_lines(
    content: &BatchOutputContent,
    limits: &ApiLimits,
) -> Result<Vec<DecodedBatchLine>, BatchOutputUsageError> {
    match content {
        BatchOutputContent::Bytes { bytes } => super::decode_batch_output_usage(bytes, limits),
        BatchOutputContent::Lines { lines } => Ok(lines.iter().map(decoded_line).collect()),
    }
}

fn decoded_line(value: &Json) -> DecodedBatchLine {
    match value {
        Json::Object(_) => DecodedBatchLine::Entry(value.clone()),
        other => DecodedBatchLine::NonObject {
            kind: json_type_name(other),
        },
    }
}

pub(super) fn decode_line(line: &[u8]) -> DecodedBatchLine {
    let Ok(text) = from_utf8(line) else {
        return DecodedBatchLine::Malformed {
            message: "the line is not valid UTF-8".to_owned(),
        };
    };
    let document = text.strip_prefix('\u{feff}').unwrap_or(text);
    match serde_json::from_str::<Json>(document) {
        Ok(value @ Json::Object(_)) => DecodedBatchLine::Entry(value),
        Ok(other) => DecodedBatchLine::NonObject {
            kind: json_type_name(&other),
        },
        Err(error) => DecodedBatchLine::Malformed {
            message: error.to_string(),
        },
    }
}

pub(super) fn json_type_name(value: &Json) -> &'static str {
    match value {
        Json::Null => "NoneType",
        Json::Bool(_) => "bool",
        Json::Number(number) => {
            if number_is_integer(number) {
                "int"
            } else {
                "float"
            }
        }
        Json::Text(_) => "str",
        Json::List(_) => "list",
        Json::Object(_) => "dict",
    }
}

pub(super) fn number_is_integer(number: &serde_json::Number) -> bool {
    !number
        .to_string()
        .bytes()
        .any(|byte| matches!(byte, b'.' | b'e' | b'E'))
}

// Python's bytes.strip() trims the separators U+001C to U+001F, which Rust's White_Space leaves alone.
const fn is_strip_byte(byte: u8) -> bool {
    matches!(
        byte,
        b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c | 0x1c..=0x1f
    )
}

pub(super) fn trim_python_bytes(line: &[u8]) -> &[u8] {
    let start = line
        .iter()
        .position(|byte| !is_strip_byte(*byte))
        .unwrap_or(line.len());
    let rest = line.get(start..).unwrap_or_default();
    let end = rest
        .iter()
        .rposition(|byte| !is_strip_byte(*byte))
        .map_or(0, |last| last.saturating_add(1));
    rest.get(..end).unwrap_or_default()
}

pub(super) fn object_field<'line>(
    fields: &'line IndexMap<String, Json>,
    key: &'static str,
) -> Result<Option<&'line IndexMap<String, Json>>, BatchLineError> {
    match fields.get(key) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(found)) => Ok(Some(found)),
        Some(_) => Err(BatchLineError::NotAnObject { field: key }),
    }
}

pub(super) fn object_value<'line>(
    fields: &'line IndexMap<String, Json>,
    key: &'static str,
) -> Result<Option<&'line Json>, BatchLineError> {
    match fields.get(key) {
        None | Some(Json::Null) => Ok(None),
        Some(found @ Json::Object(_)) => Ok(Some(found)),
        Some(_) => Err(BatchLineError::NotAnObject { field: key }),
    }
}

pub(super) fn integer_field(
    fields: &IndexMap<String, Json>,
    key: &'static str,
) -> Result<Option<i64>, BatchLineError> {
    match fields.get(key) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Number(number)) => number
            .as_i64()
            .map(Some)
            .ok_or(BatchLineError::NotAnInteger { field: key }),
        Some(_) => Err(BatchLineError::NotAnInteger { field: key }),
    }
}

pub(super) fn number_field(
    fields: &IndexMap<String, Json>,
    key: &'static str,
) -> Result<Option<&serde_json::Number>, BatchLineError> {
    match fields.get(key) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Number(number)) => Ok(Some(number)),
        Some(_) => Err(BatchLineError::NotANumber { field: key }),
    }
}

pub(super) fn empty_object(value: &Json) -> bool {
    let Json::Object(fields) = value else {
        return false;
    };
    fields.is_empty()
}

// The crate's JSON keeps IndexMap objects, so moving one into a serde_json document maps key for key.
pub(super) fn serde_object(
    fields: &IndexMap<String, Json>,
) -> Result<Map<String, Value>, BatchLineError> {
    let mut object = Map::new();
    for (key, value) in fields {
        object.insert(
            key.clone(),
            serde_json::to_value(value).map_err(|source| BatchLineError::Json { source })?,
        );
    }
    Ok(object)
}
