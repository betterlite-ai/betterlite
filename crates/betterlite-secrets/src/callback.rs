use std::collections::BTreeSet;
use std::iter;

use betterlite_redact::structured::{Payload, SensitiveDataMasker};
use secrecy::{ExposeSecret as _, SecretString};

use crate::error::SecretError;
use crate::secret_codec::diagnostics::{Level, SecretDiagnostics, SecretEvent};
use crate::secret_codec::{
    CALLBACK_VAR_ENCRYPTED_PREFIX, NonceSource, StoredCipher, StoredValue, open_stored,
};

const EXTRA_SENSITIVE_KEY: &str = "gcs_path_service_account";

// Port of callback_utils.is_sensitive_callback_key.
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

// Port of callback_utils.encrypt_callback_vars.
pub fn encrypt_callback_vars<N, D>(
    metadata: &Payload,
    cipher: &StoredCipher<N, D>,
    extra: &BTreeSet<String>,
) -> Payload
where
    N: NonceSource,
    D: SecretDiagnostics,
{
    let transform = |key: &str, value: &Payload| encrypt_if_plaintext(key, value, cipher, extra);
    transform_callback_vars(metadata, transform)
}

// Port of callback_utils.decrypt_callback_vars.
pub fn decrypt_callback_vars<N, D>(metadata: &Payload, cipher: &StoredCipher<N, D>) -> Payload
where
    D: SecretDiagnostics,
{
    let transform = |key: &str, value: &Payload| decrypt_or_passthrough(key, value, cipher);
    transform_callback_vars(metadata, transform)
}

// Port of callback_utils._transform_callback_vars: visits logging[*].callback_vars and callback_settings.callback_vars.
fn transform_callback_vars<F>(metadata: &Payload, transform: F) -> Payload
where
    F: Fn(&str, &Payload) -> Payload,
{
    let Some(map) = metadata.as_object() else {
        return metadata.clone();
    };
    let mut out = map.clone();
    if let Some(entries) = out.get_mut("logging").and_then(Payload::as_array_mut) {
        for entry in entries.iter_mut() {
            visit_callback_vars(entry, &transform);
        }
    }
    if let Some(settings) = out.get_mut("callback_settings") {
        visit_callback_vars(settings, &transform);
    }
    Payload::Object(out)
}

fn visit_callback_vars<F>(slot: &mut Payload, transform: F)
where
    F: Fn(&str, &Payload) -> Payload,
{
    let Some(vars) = slot
        .as_object()
        .and_then(|fields| fields.get("callback_vars"))
        .and_then(Payload::as_object)
    else {
        return;
    };
    let transformed = vars
        .iter()
        .map(|(key, value)| (key.clone(), transform(key, value)))
        .collect();
    if let Some(fields) = slot.as_object_mut() {
        fields.insert("callback_vars".to_owned(), Payload::Object(transformed));
    }
}

// Port of callback_utils._encrypt_if_plaintext.
fn encrypt_if_plaintext<N, D>(
    key: &str,
    value: &Payload,
    cipher: &StoredCipher<N, D>,
    extra: &BTreeSet<String>,
) -> Payload
where
    N: NonceSource,
    D: SecretDiagnostics,
{
    let Some(text) = value.as_str() else {
        return value.clone();
    };
    if text.is_empty() || !is_sensitive_callback_key(key, extra) {
        return value.clone();
    }
    if text.starts_with(CALLBACK_VAR_ENCRYPTED_PREFIX) {
        return value.clone();
    }
    let stored = StoredValue::<()>::Text(SecretString::from(text));
    match cipher.encrypt_value(stored, None) {
        Ok(StoredValue::Text(sealed)) => Payload::String(format!(
            "{CALLBACK_VAR_ENCRYPTED_PREFIX}{}",
            sealed.expose_secret()
        )),
        Ok(StoredValue::Other(())) | Err(_) => value.clone(),
    }
}

// Port of callback_utils._decrypt_or_passthrough: a failed open keeps the marked value and logs at debug level.
fn decrypt_or_passthrough<N, D>(key: &str, value: &Payload, cipher: &StoredCipher<N, D>) -> Payload
where
    D: SecretDiagnostics,
{
    let Some(text) = value.as_str() else {
        return value.clone();
    };
    let Some(inner) = (!text.is_empty())
        .then(|| text.strip_prefix(CALLBACK_VAR_ENCRYPTED_PREFIX))
        .flatten()
    else {
        return value.clone();
    };
    let opened = cipher
        .settings
        .signing_key
        .as_ref()
        .ok_or(SecretError::MissingKey)
        .and_then(|signing_key| open_stored(inner, signing_key));
    let error = match opened {
        Ok(plaintext) => return Payload::String(plaintext.expose_secret().to_owned()),
        Err(error) => error,
    };
    let failed = SecretEvent::DecryptFailed {
        field: key,
        error: &error,
    };
    cipher.diagnostics.record(Level::Debug, &failed);
    value.clone()
}

mod tests;
