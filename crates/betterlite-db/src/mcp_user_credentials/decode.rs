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

pub fn parse_oauth_payload(
    decoded: Option<&SecretString>,
) -> Result<Option<OAuthCredentialPayload>, CredentialError> {
    let Some(decoded) = decoded else {
        return Ok(None);
    };
    let Ok(kind) = serde_json::from_str::<CredentialKind>(decoded.expose_secret()) else {
        return Ok(None);
    };
    if kind.credential_type.as_deref() != Some(OAUTH2) {
        return Ok(None);
    }
    let payload = serde_json::from_str(decoded.expose_secret()).map_err(CredentialError::Json)?;
    Ok(Some(payload))
}

// a row that is not OAuth2 is Ok(None); a row that claims oauth2 with a mistyped field is the Json error on every path
pub fn decode_oauth_payload(
    cipher: &DbCipher,
    stored: &str,
) -> Result<Option<OAuthCredentialPayload>, CredentialError> {
    parse_oauth_payload(decode_user_credential(cipher, stored).as_ref())
}

pub(crate) fn warn_undecryptable_credential(user_id: &str, server_id: &str) {
    tracing::warn!(
        user_id,
        server_id,
        "MCP user credential could not be decrypted (likely written under a previous LITELLM_SALT_KEY); the user is treated as not connected and must re-authorize"
    );
}
