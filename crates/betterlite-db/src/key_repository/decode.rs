use super::TokenModel;
use super::record::DbRecord;
use crate::budget_permission_rows::JsonData;
use crate::postgres_store::diagnostics::DriverError;
use std::collections::BTreeMap;
use std::str::{Utf8Error, from_utf8};

pub const JSON_ENCODED_TOKEN_FIELDS: [&str; 9] = [
    "aliases",
    "config",
    "permissions",
    "metadata",
    "model_spend",
    "model_max_budget",
    "router_settings",
    "budget_limits",
    "litellm_budget_table",
];

pub const MAX_FIELD_BYTES: usize = 0x0010_0000;
pub const MAX_JSON_DEPTH: usize = 64;

#[derive(Debug, thiserror::Error)]
pub enum TokenDecodeError {
    #[error("stored token row is not valid UTF-8")]
    InvalidUtf8(#[source] Utf8Error),
    #[error("stored token row is invalid JSON")]
    RowJson(#[source] serde_json::Error),
    #[error("stored token row is not a JSON object")]
    NotAnObject,
    #[error("stored token field {field} is {len} bytes, over the {MAX_FIELD_BYTES} byte limit")]
    FieldTooLarge { field: String, len: usize },
    #[error("stored token field {field} exceeds JSON depth {MAX_JSON_DEPTH}")]
    FieldTooDeep { field: String },
    #[error("stored token field {field} is invalid JSON")]
    FieldJson {
        field: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("stored token record failed validation")]
    Model {
        #[source]
        source: serde_json::Error,
    },
    #[error("stored token row column is unavailable")]
    Column {
        #[source]
        source: DriverError,
    },
}

pub fn decode_token_record(bytes: &[u8]) -> Result<TokenModel, TokenDecodeError> {
    let text = from_utf8(bytes).map_err(TokenDecodeError::InvalidUtf8)?;
    let value: JsonData = serde_json::from_str(text).map_err(TokenDecodeError::RowJson)?;
    let JsonData::Object(fields) = value else {
        return Err(TokenDecodeError::NotAnObject);
    };
    decode_mapping(fields)
}

pub fn to_model(record: Option<&DbRecord>) -> Result<Option<TokenModel>, TokenDecodeError> {
    record
        .map(DbRecord::project)
        .map(decode_mapping)
        .transpose()
}

pub fn decode_mapping(
    mut fields: BTreeMap<String, JsonData>,
) -> Result<TokenModel, TokenDecodeError> {
    for field in JSON_ENCODED_TOKEN_FIELDS {
        let decoded = match fields.get(field) {
            Some(JsonData::String(text)) => decode_field(field, text)?,
            _ => continue,
        };
        fields.insert(String::from(field), decoded);
    }
    let org_id_missing = fields
        .get("org_id")
        .is_none_or(|value| matches!(value, JsonData::Null));
    if org_id_missing
        && let Some(organization_id) = fields.get("organization_id")
        && !matches!(organization_id, JsonData::Null)
    {
        let copied = organization_id.clone();
        fields.insert(String::from("org_id"), copied);
    }
    let text = serde_json::to_string(&JsonData::Object(fields))
        .map_err(|source| TokenDecodeError::Model { source })?;
    serde_json::from_str(&text).map_err(|source| TokenDecodeError::Model { source })
}

fn decode_field(field: &str, text: &str) -> Result<JsonData, TokenDecodeError> {
    if text.len() > MAX_FIELD_BYTES {
        return Err(TokenDecodeError::FieldTooLarge {
            field: String::from(field),
            len: text.len(),
        });
    }
    if !depth_within_limit(text) {
        return Err(TokenDecodeError::FieldTooDeep {
            field: String::from(field),
        });
    }
    serde_json::from_str(text).map_err(|source| TokenDecodeError::FieldJson {
        field: String::from(field),
        source,
    })
}

fn depth_within_limit(text: &str) -> bool {
    let mut depth = 0_usize;
    let mut quoted = false;
    let mut escaped = false;
    for byte in text.bytes() {
        if quoted {
            match byte {
                b'\\' if !escaped => escaped = true,
                b'"' if !escaped => quoted = false,
                _ => escaped = false,
            }
            continue;
        }
        match byte {
            b'"' => quoted = true,
            b'{' | b'[' => {
                depth = depth.saturating_add(1);
                if depth > MAX_JSON_DEPTH {
                    return false;
                }
            }
            b'}' | b']' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    true
}
