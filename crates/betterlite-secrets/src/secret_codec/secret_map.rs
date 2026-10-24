use std::collections::BTreeMap;

use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::error::SecretError;
use crate::secret_codec::diagnostics::SecretDiagnostics;
use crate::secret_codec::{
    DecryptPolicy, FailureLog, FailureValue, MAX_ENCODED_BYTES, NonceSource, StoredCipher,
    StoredValue,
};

pub type SecretMap = BTreeMap<String, SecretString>;

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum StoredMap {
    Map(SecretMap),
    Text(SecretString),
    Other(IgnoredAny),
}

pub fn encrypt_secret_map<N, D>(
    cipher: &StoredCipher<N, D>,
    map: &SecretMap,
    override_key: Option<&SecretString>,
) -> Result<String, SecretError>
where
    N: NonceSource,
    D: SecretDiagnostics,
{
    if map.is_empty() {
        return Ok("{}".to_owned());
    }
    let exposed: BTreeMap<&str, &str> = map
        .iter()
        .map(|(name, value)| (name.as_str(), value.expose_secret()))
        .collect();
    let plaintext =
        serde_json::to_string(&exposed).map_err(|source| SecretError::MapEncode { source })?;
    let ciphertext = cipher.encrypt_text(&plaintext, override_key)?;
    serde_json::to_string(&ciphertext).map_err(|source| SecretError::MapEncode { source })
}

pub fn decode_secret_map<N, D>(
    cipher: &StoredCipher<N, D>,
    stored: Option<&StoredMap>,
    field: &str,
) -> Result<Option<SecretMap>, SecretError>
where
    N: NonceSource,
    D: SecretDiagnostics,
{
    let rejected = || SecretError::MapDecode {
        field: field.to_owned(),
    };
    let text = match stored {
        None => return Ok(None),
        Some(StoredMap::Map(map)) => return Ok(Some(map.clone())),
        Some(StoredMap::Text(text)) if text.expose_secret().len() <= MAX_ENCODED_BYTES => {
            text.expose_secret()
        }
        Some(StoredMap::Text(_) | StoredMap::Other(_)) => return Err(rejected()),
    };
    // Python's str.lstrip also strips the separators U+001C to U+001F
    let trimmed = text.trim_start_matches(|symbol: char| {
        symbol.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&symbol)
    });
    // typed parsing fails at the first nested value, so it never recurses past depth one
    let ciphertext: SecretString = if trimmed.starts_with('{') {
        return serde_json::from_str(text)
            .map(Some)
            .map_err(|_json| rejected());
    } else if trimmed.starts_with('"') {
        serde_json::from_str(text).map_err(|_json| rejected())?
    } else {
        SecretString::from(text)
    };
    let policy = DecryptPolicy {
        log: FailureLog::Debug,
        value: FailureValue::Nothing,
    };
    let decrypted = cipher.decrypt_value(StoredValue::<()>::Text(ciphertext), field, policy);
    let Some(StoredValue::Text(plaintext)) = decrypted else {
        return Err(rejected());
    };
    serde_json::from_str(plaintext.expose_secret())
        .map(Some)
        .map_err(|_json| rejected())
}
