use betterlite_secrets::secret_codec::python_base64::decode_lenient;
use betterlite_secrets::secret_codec::{DecryptPolicy, FailureLog, FailureValue, StoredValue};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use crate::stored_cipher::DbCipher;

use super::types::{CredentialError, OAuthCredentialPayload};

const CREDENTIAL_KEY: &str = "mcp_user_credential";
const OAUTH2: &str = "oauth2";

// the one field that separates an OAuth2 payload from a BYOK key; a value that is not an object with a text type is not a payload
#[derive(Deserialize)]
struct CredentialKind {
    #[serde(rename = "type")]
    credential_type: Option<String>,
}
// LiteLLM decrypts under the "debug" policy, so a failed open logs at debug and never at error
const UNREADABLE: DecryptPolicy = DecryptPolicy {
    log: FailureLog::Debug,
    value: FailureValue::Nothing,
};

// returns None where LiteLLM's decrypt_value_helper returns None: a wrong key, a tampered cipher, or a value written before this deployment encrypted credentials
fn decrypt_credential(cipher: &DbCipher, stored: &str) -> Option<String> {
    let value = StoredValue::<()>::Text(SecretString::from(stored.to_owned()));
    let StoredValue::Text(plaintext) = cipher.decrypt_value(value, CREDENTIAL_KEY, UNREADABLE)?
    else {
        return None;
    };
    Some(plaintext.expose_secret().to_owned())
}

// LiteLLM decodes older plain-base64 rows with the URL-safe alphabet and then falls back to the standard one
fn legacy_plaintext(stored: &str) -> Option<String> {
    let bytes = decode_lenient(stored, true).ok()?;
    let text = String::from_utf8(bytes).ok()?;
    Some(text)
}

// LiteLLM uses decrypt_value_helper(return_original_value=False), so an unreadable ciphertext falls through to the legacy decode instead of being the answer
#[must_use]
pub fn decode_user_credential(cipher: &DbCipher, stored: &str) -> Option<String> {
    if let Some(decrypted) = decrypt_credential(cipher, stored) {
        return Some(decrypted);
    }
    legacy_plaintext(stored)
}

#[must_use]
pub fn parse_oauth_payload(
    decoded: Option<&str>,
) -> Result<Option<OAuthCredentialPayload>, CredentialError> {
    let Some(decoded) = decoded else {
        return Ok(None);
    };
    let Ok(kind) = serde_json::from_str::<CredentialKind>(decoded) else {
        return Ok(None);
    };
    if kind.credential_type.as_deref() != Some(OAUTH2) {
        return Ok(None);
    }
    let payload = serde_json::from_str(decoded).map_err(CredentialError::Json)?;
    Ok(Some(payload))
}

#[must_use]
pub fn decode_oauth_payload(cipher: &DbCipher, stored: &str) -> Option<OAuthCredentialPayload> {
    let decoded = decode_user_credential(cipher, stored);
    let Some(payload) = parse_oauth_payload(decoded.as_deref()).ok().flatten() else {
        return None;
    };
    Some(payload)
}

pub(crate) fn warn_undecryptable_credential(user_id: &str, server_id: &str) {
    tracing::warn!(
        user_id,
        server_id,
        "MCP user credential could not be decrypted (likely written under a previous LITELLM_SALT_KEY); the user is treated as not connected and must re-authorize"
    );
}
