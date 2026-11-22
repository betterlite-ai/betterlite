use betterlite_db::identity_rows::key::LiteLLMVerificationToken;
use secrecy::{ExposeSecret as _, SecretString};

use crate::key_fields::response::GenerateKeyResponse;

use super::error::KeyEventError;

// response.token_id as LiteLLM reads it: an absent field and an explicit null are both None
pub(super) fn token_id(response: &GenerateKeyResponse) -> Option<&str> {
    match &response.response.token_id {
        Some(token_id) => token_id.as_deref(),
        None => None,
    }
}

// response.key; the response model rejects a key response without one, so this only guards a hand-built event
pub(super) fn response_key(response: &GenerateKeyResponse) -> Result<&SecretString, KeyEventError> {
    match &response.response.key {
        Some(key) => key.as_ref().ok_or(KeyEventError::MissingResponseKey),
        None => Err(KeyEventError::MissingResponseKey),
    }
}

// existing_key_row.key_alias or f"virtual-key-{existing_key_row.token}"
pub(super) fn stored_name(row: &LiteLLMVerificationToken) -> String {
    let token = row.token.as_ref().map(ExposeSecret::expose_secret);
    match &row.key_alias {
        Some(alias) if !alias.is_empty() => alias.clone(),
        _ => format!("virtual-key-{}", token.map_or("None", String::as_str)),
    }
}
