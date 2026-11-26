use betterlite_models::controls::json_value::JsonValue;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::trusted_network::cidr::CidrRanges;

pub const MAX_NETWORK_HEADER_BYTES: usize = 0x0010_0000;
pub const MAX_NETWORK_HEADER_DEPTH: usize = 64;

const FIELD_NAMES: [&str; 5] = [
    "x_forwarded_for",
    "x_forwarded_proto",
    "host",
    "trusted_proxy_ranges",
    "mcp_trusted_proxy_ranges",
];

// The header document the service writes and reads back through the one bounded decoder.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TypedValue {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x_forwarded_for: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x_forwarded_proto: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trusted_proxy_ranges: Option<CidrRanges>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp_trusted_proxy_ranges: Option<Vec<String>>,
}

#[derive(Debug, Error)]
pub enum DecodeError {
    #[error("the network header document is {size} bytes, over the {limit}-byte limit")]
    TooLarge { size: usize, limit: usize },
    #[error("the network header document nests {depth} levels, over the {limit}-level limit")]
    TooDeep { depth: usize, limit: usize },
    #[error("the network header document is not valid JSON")]
    Json(#[source] serde_json::Error),
    #[error("the network header document is not a JSON object")]
    NotAnObject,
    #[error("the network header document carries an unknown field {field}")]
    UnknownField { field: String },
    #[error("the network header document could not be written")]
    Write(#[source] serde_json::Error),
}

// Outside bytes stop here: the bounds run before any policy function sees the document.
pub fn decode_network_headers(bytes: &[u8]) -> Result<TypedValue, DecodeError> {
    if bytes.len() > MAX_NETWORK_HEADER_BYTES {
        return Err(DecodeError::TooLarge {
            size: bytes.len(),
            limit: MAX_NETWORK_HEADER_BYTES,
        });
    }
    let document: JsonValue = serde_json::from_slice(bytes).map_err(DecodeError::Json)?;
    let depth = nesting(&document);
    if depth > MAX_NETWORK_HEADER_DEPTH {
        return Err(DecodeError::TooDeep {
            depth,
            limit: MAX_NETWORK_HEADER_DEPTH,
        });
    }
    let JsonValue::Object(fields) = &document else {
        return Err(DecodeError::NotAnObject);
    };
    if let Some(name) = fields
        .keys()
        .find(|name| !FIELD_NAMES.contains(&name.as_str()))
    {
        return Err(DecodeError::UnknownField {
            field: (*name).clone(),
        });
    }
    serde_json::from_slice(bytes).map_err(DecodeError::Json)
}

// serde_json bounds its own parser at 128 levels, so this walk of the parsed tree cannot overflow the stack.
fn nesting(value: &JsonValue) -> usize {
    match value {
        JsonValue::Array(items) => items
            .iter()
            .map(nesting)
            .max()
            .map_or(1_usize, |deepest| deepest.saturating_add(1)),
        JsonValue::Object(fields) => fields
            .values()
            .map(nesting)
            .max()
            .map_or(1_usize, |deepest| deepest.saturating_add(1)),
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Text(_) => 0,
    }
}
