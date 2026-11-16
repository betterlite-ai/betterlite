use std::collections::BTreeMap;

use betterlite_secrets::secret_codec::diagnostics::SecretDiagnostics;
use betterlite_secrets::secret_codec::{
    DecryptPolicy, FailureLog, FailureValue, NonceSource, StoredCipher, StoredValue,
};
use secrecy::{ExposeSecret as _, SecretString};

use crate::mcp_stored_contracts::JsonValue;
use crate::mcp_stored_contracts::decode::decode_mcp_stored_contract;

use super::error::UserEnvVarsError;
use super::store::UserEnvVars;

pub const USER_ENV_VARS_KEY: &str = "mcp_user_env_vars";

const DECRYPT_POLICY: DecryptPolicy = DecryptPolicy {
    log: FailureLog::Debug,
    value: FailureValue::Nothing,
};

// the JSON step alone, so the fuzz target reaches it without a cipher
#[must_use]
pub fn parse_user_env_vars(decrypted: &str) -> UserEnvVars {
    let Ok(JsonValue::Object(fields)) = decode_mcp_stored_contract(decrypted.as_bytes()) else {
        return UserEnvVars::new();
    };
    let mut values = UserEnvVars::new();
    for (name, value) in fields {
        let JsonValue::String(text) = value else {
            tracing::warn!(
                "MCP per-user env vars hold a value that is not a string; treating them as unset so the user is prompted to re-enter them"
            );
            return UserEnvVars::new();
        };
        values.insert(name, SecretString::from(text));
    }
    values
}

// LiteLLM has no legacy plain-base64 path here, unlike per-user credentials
#[must_use]
pub fn decode_user_env_vars<N, D>(stored: &str, cipher: &StoredCipher<N, D>) -> UserEnvVars
where
    D: SecretDiagnostics,
{
    let sealed = StoredValue::<()>::Text(SecretString::from(stored.to_owned()));
    if let Some(StoredValue::Text(decrypted)) =
        cipher.decrypt_value(sealed, USER_ENV_VARS_KEY, DECRYPT_POLICY)
    {
        return parse_user_env_vars(decrypted.expose_secret());
    }
    if !stored.is_empty() {
        tracing::warn!(
            "MCP per-user env vars failed to decrypt (LITELLM_SALT_KEY changed?); treating as unset so the user is prompted to re-enter them rather than silently forwarding ciphertext"
        );
    }
    UserEnvVars::new()
}

pub fn seal_user_env_vars<N, D>(
    values: &UserEnvVars,
    cipher: &StoredCipher<N, D>,
) -> Result<String, UserEnvVarsError>
where
    N: NonceSource,
    D: SecretDiagnostics,
{
    let exposed: BTreeMap<&str, &str> = values
        .iter()
        .map(|(name, value)| (name.as_str(), value.expose_secret()))
        .collect();
    let encoded = serde_json::to_string(&exposed).map_err(UserEnvVarsError::Encode)?;
    let plaintext = StoredValue::<()>::Text(SecretString::from(encoded));
    match cipher
        .encrypt_value(plaintext, None)
        .map_err(UserEnvVarsError::Encrypt)?
    {
        StoredValue::Text(sealed) => Ok(sealed.expose_secret().to_owned()),
        StoredValue::Other(()) => Err(UserEnvVarsError::NotText),
    }
}
