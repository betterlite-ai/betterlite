use betterlite_models::controls::json_value::JsonValue;
use serde::Serialize;
use thiserror::Error;

use crate::trusted_network::cidr::CidrRanges;

pub const MAX_NETWORK_HEADER_BYTES: usize = 1_048_576;
pub const MAX_NETWORK_HEADER_DEPTH: usize = 64;

const FIELD_NAMES: [&str; 5] = [
    "x_forwarded_for",
    "x_forwarded_proto",
    "host",
    "trusted_proxy_ranges",
    "mcp_trusted_proxy_ranges",
];

// The typed network header document: the service writes the request headers and configured ranges into it,
// then reads them back through the one bounded decoder before any policy function sees them.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
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
    #[error("the network header field {field} is not a string")]
    NotText { field: String },
    #[error("the network header field {field} is not a list of strings")]
    NotRanges { field: String },
    #[error("the network header document could not be written")]
    Write(#[source] serde_json::Error),
}

// Outside bytes stop here: the size and nesting bounds run before the document is projected,
// so a caller's header text can never hand an unbounded tree to the policy functions.
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
    Ok(TypedValue {
        x_forwarded_for: text_field(fields.get("x_forwarded_for"), "x_forwarded_for")?,
        x_forwarded_proto: text_field(fields.get("x_forwarded_proto"), "x_forwarded_proto")?,
        host: text_field(fields.get("host"), "host")?,
        trusted_proxy_ranges: range_field(
            fields.get("trusted_proxy_ranges"),
            "trusted_proxy_ranges",
        )?,
        mcp_trusted_proxy_ranges: list_field(
            fields.get("mcp_trusted_proxy_ranges"),
            "mcp_trusted_proxy_ranges",
        )?,
    })
}

pub fn encode_network_headers(value: &TypedValue) -> Result<Vec<u8>, DecodeError> {
    serde_json::to_vec(value).map_err(DecodeError::Write)
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

fn text_field(value: Option<&JsonValue>, field: &str) -> Result<Option<String>, DecodeError> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Text(text)) => Ok(Some(text.clone())),
        Some(
            JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Array(_) | JsonValue::Object(_),
        ) => Err(DecodeError::NotText {
            field: field.to_owned(),
        }),
    }
}

fn range_field(value: Option<&JsonValue>, field: &str) -> Result<Option<CidrRanges>, DecodeError> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Text(text)) => Ok(Some(CidrRanges::Text(text.clone()))),
        Some(JsonValue::Array(items)) => Ok(Some(CidrRanges::List(texts(items, field)?))),
        Some(JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Object(_)) => {
            Err(DecodeError::NotRanges {
                field: field.to_owned(),
            })
        }
    }
}

fn list_field(value: Option<&JsonValue>, field: &str) -> Result<Option<Vec<String>>, DecodeError> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Array(items)) => Ok(Some(texts(items, field)?)),
        Some(
            JsonValue::Text(_) | JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Object(_),
        ) => Err(DecodeError::NotRanges {
            field: field.to_owned(),
        }),
    }
}

fn texts(items: &[JsonValue], field: &str) -> Result<Vec<String>, DecodeError> {
    let mut values = Vec::with_capacity(items.len());
    for item in items {
        match item {
            JsonValue::Text(text) => values.push(text.clone()),
            JsonValue::Null
            | JsonValue::Bool(_)
            | JsonValue::Number(_)
            | JsonValue::Array(_)
            | JsonValue::Object(_) => {
                return Err(DecodeError::NotText {
                    field: field.to_owned(),
                });
            }
        }
    }
    Ok(values)
}
