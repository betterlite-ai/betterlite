use std::collections::BTreeMap;
use std::str::{Utf8Error, from_utf8};

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::key::LiteLLMVerificationToken;

use crate::key_fields::decode::{decode_group, to_object};
use crate::key_fields::error::DecodeError;
use crate::key_fields::policy::{CustomKeyPolicyRequest, PolicyOperation, PolicyRequestBody};

pub const JSON_STRING_COLUMNS: [&str; 2] = ["router_settings", "budget_limits"];

#[derive(Debug, thiserror::Error)]
pub enum ColumnError {
    #[error("{0} is not JSON text")]
    Json(String, #[source] serde_json::Error),
    #[error("{0} is not UTF-8 text")]
    Utf8(String, #[source] Utf8Error),
    #[error(transparent)]
    Decode(#[from] DecodeError),
}

// only the two JSON columns hold text a request may hand over; every other column and every non-string passes through
pub fn decode_json_string_column(column: &str, value: &JsonData) -> Result<JsonData, ColumnError> {
    let JsonData::String(text) = value else {
        return Ok(value.clone());
    };
    if !JSON_STRING_COLUMNS.contains(&column) {
        return Ok(value.clone());
    }
    serde_json::from_str::<JsonData>(text)
        .map_err(|source| ColumnError::Json(column.to_owned(), source))
}

// the fuzz target and its corpus replay hand over the bytes a stored column holds, invalid UTF-8 included
pub fn decode_json_column_bytes(column: &str, bytes: &[u8]) -> Result<JsonData, ColumnError> {
    from_utf8(bytes)
        .map_err(|source| ColumnError::Utf8(column.to_owned(), source))
        .and_then(|text| decode_json_string_column(column, &JsonData::String(text.to_owned())))
}

// a row naming organization_id keeps it, even when null; only a row without that key falls back to org_id
pub fn verification_token_from_row(
    row: &BTreeMap<String, JsonData>,
) -> Result<LiteLLMVerificationToken, ColumnError> {
    let organization = row
        .get("organization_id")
        .cloned()
        .unwrap_or_else(|| row.get("org_id").cloned().unwrap_or(JsonData::Null));
    let mut fields = row.clone();
    fields.insert("org_id".to_owned(), organization);
    Ok(decode_group(&fields)?)
}

// the stored row stays untouched: the patch is decoded and laid over a copy of it, with no object permission
pub fn effective_key_after_update(
    existing: &LiteLLMVerificationToken,
    patch: &BTreeMap<String, JsonData>,
) -> Result<LiteLLMVerificationToken, ColumnError> {
    let mut row = to_object(existing)?;
    for (column, value) in patch {
        let decoded = decode_json_string_column(column, value)?;
        row.insert(column.clone(), decoded);
    }
    row.insert("object_permission".to_owned(), JsonData::Null);
    verification_token_from_row(&row)
}

pub fn update_policy_request(
    operation: PolicyOperation,
    existing_key: &LiteLLMVerificationToken,
    patch: &BTreeMap<String, JsonData>,
    request: PolicyRequestBody,
) -> Result<CustomKeyPolicyRequest, ColumnError> {
    let existing = verification_token_from_row(&to_object(existing_key)?)?;
    let effective_key = effective_key_after_update(existing_key, patch)?;
    Ok(CustomKeyPolicyRequest {
        operation,
        existing_key: Some(existing),
        effective_key,
        request,
    })
}
