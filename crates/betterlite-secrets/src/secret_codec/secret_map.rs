use std::collections::BTreeMap;

use secrecy::{ExposeSecret as _, SecretString};

use crate::error::SecretError;
use crate::secret_codec::diagnostics::SecretDiagnostics;
use crate::secret_codec::{
    DecryptPolicy, FailureLog, FailureValue, MAX_ENCODED_BYTES, NonceSource, StoredCipher,
    StoredValue,
};

pub type SecretMap = BTreeMap<String, SecretString>;

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
    let plaintext = SecretString::from(
        serde_json::to_string(&exposed).map_err(|source| SecretError::MapEncode { source })?,
    );
    let ciphertext = cipher.encrypt_text(plaintext.expose_secret(), override_key)?;
    serde_json::to_string(&ciphertext).map_err(|source| SecretError::MapEncode { source })
}

pub fn decode_secret_map<N, D>(
    cipher: &StoredCipher<N, D>,
    stored: Option<&str>,
    field: &str,
) -> Result<Option<SecretMap>, SecretError>
where
    D: SecretDiagnostics,
{
    let rejected = || SecretError::MapDecode {
        field: field.to_owned(),
    };
    let Some(raw) = stored else {
        return Ok(None);
    };
    if raw.len() > MAX_ENCODED_BYTES {
        return Err(rejected());
    }
    // Typed attempts reject nested values immediately without buffering a generic JSON tree
    if let Ok(map) = serde_json::from_str::<SecretMap>(raw) {
        return Ok(Some(map));
    }
    let text: Option<SecretString> = serde_json::from_str(raw).map_err(|_json| rejected())?;
    text.map_or_else(
        || Ok(None),
        |value| decode_text_map(cipher, value.expose_secret(), field).map(Some),
    )
}

fn decode_text_map<N, D>(
    cipher: &StoredCipher<N, D>,
    text: &str,
    field: &str,
) -> Result<SecretMap, SecretError>
where
    D: SecretDiagnostics,
{
    let rejected = || SecretError::MapDecode {
        field: field.to_owned(),
    };
    // Python's str.lstrip also strips the separators U+001C to U+001F
    let trimmed = text.trim_start_matches(|symbol: char| {
        symbol.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&symbol)
    });
    // typed parsing fails at the first nested value, so it never recurses past depth one
    let ciphertext: SecretString = if trimmed.starts_with('{') {
        return serde_json::from_str(text).map_err(|_json| rejected());
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
    serde_json::from_str(plaintext.expose_secret()).map_err(|_json| rejected())
}
