#![expect(
    clippy::disallowed_types,
    reason = "callback_vars metadata is unconstrained proxy config JSON; only the two known callback_vars paths are touched, and the rest passes through unchanged"
)]

use std::collections::BTreeSet;
use std::iter;

use betterlite_redact::structured::SensitiveDataMasker;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Map, Value};

use crate::secret_codec::diagnostics::SecretDiagnostics;
use crate::secret_codec::{
    CALLBACK_VAR_ENCRYPTED_PREFIX, DecryptPolicy, FailureLog, FailureValue, NonceSource,
    StoredCipher, StoredValue,
};

const EXTRA_SENSITIVE_KEY: &str = "gcs_path_service_account";

/// Port of `callback_utils.is_sensitive_callback_key`.
#[must_use]
pub fn is_sensitive_callback_key(key: &str, extra: &BTreeSet<String>) -> bool {
    if extra.contains(key) {
        return true;
    }
    if key.to_lowercase() == EXTRA_SENSITIVE_KEY {
        return true;
    }
    SensitiveDataMasker::with_patterns(iter::empty::<&str>(), iter::empty::<&str>())
        .is_sensitive_key(key, &[])
}

/// Port of `callback_utils.encrypt_callback_vars`.
pub fn encrypt_callback_vars<N, D>(
    metadata: &Value,
    cipher: &StoredCipher<N, D>,
    extra: &BTreeSet<String>,
) -> Value
where
    N: NonceSource,
    D: SecretDiagnostics,
{
    transform_callback_vars(metadata, &|key, value| {
        encrypt_if_plaintext(key, value, cipher, extra)
    })
}

/// Port of `callback_utils.decrypt_callback_vars`.
pub fn decrypt_callback_vars<N, D>(metadata: &Value, cipher: &StoredCipher<N, D>) -> Value
where
    N: NonceSource,
    D: SecretDiagnostics,
{
    transform_callback_vars(metadata, &|key, value| {
        decrypt_or_passthrough(key, value, cipher)
    })
}

/// Port of `callback_utils._transform_callback_vars`: visits
/// `logging[*].callback_vars` and `callback_settings.callback_vars`, leaving
/// every other field of a cloned `metadata` untouched.
fn transform_callback_vars(metadata: &Value, transform: &dyn Fn(&str, &Value) -> Value) -> Value {
    let Value::Object(map) = metadata else {
        return metadata.clone();
    };
    let mut out = map.clone();
    if let Some(Value::Array(entries)) = out.get_mut("logging") {
        for entry in entries.iter_mut() {
            if let Value::Object(entry_map) = entry {
                visit_callback_vars(entry_map, transform);
            }
        }
    }
    if let Some(Value::Object(settings)) = out.get_mut("callback_settings") {
        visit_callback_vars(settings, transform);
    }
    Value::Object(out)
}

fn visit_callback_vars(slot: &mut Map<String, Value>, transform: &dyn Fn(&str, &Value) -> Value) {
    let Some(Value::Object(vars)) = slot.get("callback_vars") else {
        return;
    };
    let transformed: Map<String, Value> = vars
        .iter()
        .map(|(key, value)| (key.clone(), transform(key, value)))
        .collect();
    slot.insert("callback_vars".to_owned(), Value::Object(transformed));
}

/// Port of `callback_utils._encrypt_if_plaintext`: a nonempty, sensitive,
/// not-already-marked string is sealed and prefixed; any other input, or a
/// cipher that has no key configured, passes through unchanged.
fn encrypt_if_plaintext<N, D>(
    key: &str,
    value: &Value,
    cipher: &StoredCipher<N, D>,
    extra: &BTreeSet<String>,
) -> Value
where
    N: NonceSource,
    D: SecretDiagnostics,
{
    let Value::String(text) = value else {
        return value.clone();
    };
    if text.is_empty() || !is_sensitive_callback_key(key, extra) {
        return value.clone();
    }
    if text.starts_with(CALLBACK_VAR_ENCRYPTED_PREFIX) {
        return value.clone();
    }
    let stored = StoredValue::<()>::Text(SecretString::from(text.as_str()));
    match cipher.encrypt_value(stored, None) {
        Ok(StoredValue::Text(sealed)) => Value::String(format!(
            "{CALLBACK_VAR_ENCRYPTED_PREFIX}{}",
            sealed.expose_secret()
        )),
        Ok(StoredValue::Other(())) | Err(_) => value.clone(),
    }
}

/// Port of `callback_utils._decrypt_or_passthrough`: only a marked string is
/// opened, and a failed open keeps the complete marked value, prefix
/// included, rather than the stripped inner ciphertext.
fn decrypt_or_passthrough<N, D>(key: &str, value: &Value, cipher: &StoredCipher<N, D>) -> Value
where
    N: NonceSource,
    D: SecretDiagnostics,
{
    let Value::String(text) = value else {
        return value.clone();
    };
    let Some(inner) = (!text.is_empty())
        .then(|| text.strip_prefix(CALLBACK_VAR_ENCRYPTED_PREFIX))
        .flatten()
    else {
        return value.clone();
    };
    let policy = DecryptPolicy {
        log: FailureLog::Debug,
        value: FailureValue::Nothing,
    };
    let stored = StoredValue::<()>::Text(SecretString::from(inner));
    match cipher.decrypt_value(stored, key, policy) {
        Some(StoredValue::Text(opened)) => Value::String(opened.expose_secret().to_owned()),
        Some(StoredValue::Other(())) | None => value.clone(),
    }
}

mod tests;
