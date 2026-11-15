use std::str::from_utf8;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::api_boundaries::ApiLimits;

use super::BatchOutputContent;
use super::error::{BatchOutputUsageError, BatchResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodedBatchLine {
    Entry(Json),
    Malformed { message: String },
    NonObject { kind: &'static str },
}

pub(super) fn decoded_lines(
    content: &BatchOutputContent,
    limits: &ApiLimits,
) -> BatchResult<Vec<DecodedBatchLine>> {
    match content {
        BatchOutputContent::Bytes { bytes } => super::decode_batch_output_usage(bytes, limits),
        BatchOutputContent::Lines { lines } => Ok(lines.iter().map(decoded_line).collect()),
    }
}

fn decoded_line(value: &Json) -> DecodedBatchLine {
    match value {
        Json::Object(_) => DecodedBatchLine::Entry(value.clone()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            DecodedBatchLine::NonObject {
                kind: json_type_name(value),
            }
        }
    }
}

pub(super) fn decode_line(line: &[u8]) -> DecodedBatchLine {
    let Some(text) = from_utf8(line).ok() else {
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
        Json::Number(number) if number_is_integer(number) => "int",
        Json::Number(_) => "float",
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

// Python's bytes.strip() trims these six ASCII bytes and leaves U+001C to U+001F in the line.
const PYTHON_STRIP_BYTES: [u8; 6] = [b' ', b'\t', b'\n', b'\r', 0x0b, 0x0c];

pub(super) fn trim_python_bytes(line: &[u8]) -> &[u8] {
    let start = line
        .iter()
        .position(|byte| !PYTHON_STRIP_BYTES.contains(byte))
        .unwrap_or(line.len());
    let rest = line.get(start..).unwrap_or_default();
    let end = rest
        .iter()
        .rposition(|byte| !PYTHON_STRIP_BYTES.contains(byte))
        .map_or(0, |last| last.saturating_add(1));
    rest.get(..end).unwrap_or_default()
}

pub(super) fn object_field<'line>(
    fields: &'line IndexMap<String, Json>,
    key: &'static str,
) -> BatchResult<Option<&'line IndexMap<String, Json>>> {
    match fields.get(key) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(found)) => Ok(Some(found)),
        Some(_) => Err(BatchOutputUsageError::NotAnObject { field: key }),
    }
}
pub(super) fn integer_field(
    fields: &IndexMap<String, Json>,
    key: &'static str,
) -> BatchResult<Option<i64>> {
    match fields.get(key) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Number(number)) => number
            .as_i64()
            .map(Some)
            .ok_or(BatchOutputUsageError::NotAnInteger { field: key }),
        Some(_) => Err(BatchOutputUsageError::NotAnInteger { field: key }),
    }
}
