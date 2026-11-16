use betterlite_secrets::secret_codec::python_base64::decode_lenient;
use betterlite_secrets::secret_codec::{DecryptPolicy, FailureLog, FailureValue, StoredValue};
use secrecy::{ExposeSecret as _, SecretString};

use crate::budget_permission_rows::JsonData;
use crate::stored_cipher::DbCipher;

use super::rows::stored_text;
use super::types::{CredentialError, OAuthCredentialPayload};

const CREDENTIAL_KEY: &str = "mcp_user_credential";
const OAUTH2: &str = "oauth2";

// LiteLLM decrypts under the "debug" policy, so a failed open logs at debug and never at error
const UNREADABLE: DecryptPolicy = DecryptPolicy {
    log: FailureLog::Debug,
    value: FailureValue::Nothing,
};

// returns None where LiteLLM's decrypt_value_helper returns None: a wrong key, a tampered cipher, or a value written before this deployment encrypted credentials
fn decrypt_credential(cipher: &DbCipher, stored: &str) -> Option<SecretString> {
    let value = StoredValue::<()>::Text(SecretString::from(stored.to_owned()));
    let StoredValue::Text(plaintext) = cipher.decrypt_value(value, CREDENTIAL_KEY, UNREADABLE)?
    else {
        return None;
    };
    Some(plaintext)
}

// LiteLLM uses decrypt_value_helper(return_original_value=False), so an unreadable ciphertext falls through to the plain-base64 decode (URL-safe alphabet first, then the standard one) instead of being the answer
#[must_use]
pub fn decode_user_credential(cipher: &DbCipher, stored: &str) -> Option<SecretString> {
    if let Some(decrypted) = decrypt_credential(cipher, stored) {
        return Some(decrypted);
    }
    String::from_utf8(decode_lenient(stored, true).ok()?)
        .ok()
        .map(SecretString::from)
}

// LiteLLM's _parse_oauth_payload: only a JSON object whose "type" is the text "oauth2" is a payload, so an array or scalar is not one
pub fn parse_oauth_payload(
    decoded: Option<&SecretString>,
) -> Result<Option<OAuthCredentialPayload>, CredentialError> {
    let Some(text) = decoded else {
        return Ok(None);
    };
    let Ok(fields) =
        serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(text.expose_secret())
    else {
        return Ok(None);
    };
    if fields.get("type").and_then(serde_json::Value::as_str) != Some(OAUTH2) {
        return Ok(None);
    }
    serde_json::from_value(serde_json::Value::Object(fields))
        .map(Some)
        .map_err(CredentialError::Json)
}

// every reader that shows a user's token warns once about the row it cannot open; the BYOK guard keeps its own message because it replaces the row
pub(super) fn decoded_row(
    row: &JsonData,
    cipher: &DbCipher,
    user_id: &str,
    server_id: &str,
) -> Result<Option<SecretString>, CredentialError> {
    let decoded = decode_user_credential(cipher, stored_text(row)?);
    if decoded.is_none() {
        warn_undecryptable_credential(user_id, server_id);
    }
    Ok(decoded)
}

pub(crate) fn warn_undecryptable_credential(user_id: &str, server_id: &str) {
    tracing::warn!(
        user_id,
        server_id,
        "MCP user credential could not be decrypted (likely written under a previous LITELLM_SALT_KEY); the user is treated as not connected and must re-authorize"
    );
}
