use std::collections::BTreeSet;

use betterlite_redact::structured::{Payload, SensitiveDataMasker};
use secrecy::{ExposeSecret as _, SecretString};

use crate::secret_codec::diagnostics::SecretDiagnostics;
use crate::secret_codec::{
    CALLBACK_VAR_ENCRYPTED_PREFIX, DecryptPolicy, FailureLog, FailureValue, NonceSource,
    StoredCipher, StoredValue,
};

const EXTRA_SENSITIVE_KEY: &str = "gcs_path_service_account";
const CALLBACK_CONFIG_SLOTS: [&str; 3] =
    ["logging", "callback_settings", "secret_manager_settings"];

#[must_use]
pub fn strip_callback_config(metadata: &Payload) -> Payload {
    let Some(map) = metadata.as_object() else {
        return metadata.clone();
    };
    let stripped = map
        .iter()
        .filter(|(key, _)| !CALLBACK_CONFIG_SLOTS.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    Payload::Object(stripped)
}

#[must_use]
pub fn is_sensitive_callback_key(key: &str, extra: &BTreeSet<String>) -> bool {
    if extra.contains(key) {
        return true;
    }
    if key.to_lowercase() == EXTRA_SENSITIVE_KEY {
        return true;
    }
    SensitiveDataMasker::default().is_sensitive_key(key, &[])
}

pub fn encrypt_callback_vars<N, D>(metadata: &Payload, cipher: &StoredCipher<N, D>) -> Payload
where
    N: NonceSource,
    D: SecretDiagnostics,
{
    let transform = |key: &str, value: &Payload| encrypt_if_plaintext(key, value, cipher);
    transform_callback_vars(metadata, transform)
}

pub fn decrypt_callback_vars<N, D>(metadata: &Payload, cipher: &StoredCipher<N, D>) -> Payload
where
    D: SecretDiagnostics,
{
    let transform = |key: &str, value: &Payload| decrypt_or_passthrough(key, value, cipher);
    transform_callback_vars(metadata, transform)
}

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

fn encrypt_if_plaintext<N, D>(key: &str, value: &Payload, cipher: &StoredCipher<N, D>) -> Payload
where
    N: NonceSource,
    D: SecretDiagnostics,
{
    let Some(text) = value.as_str() else {
        return value.clone();
    };
    if text.is_empty() || !is_sensitive_callback_key(key, &BTreeSet::new()) {
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
    let policy = DecryptPolicy {
        log: FailureLog::Debug,
        value: FailureValue::Nothing,
    };
    let stored = StoredValue::<()>::Text(SecretString::from(inner));
    match cipher.decrypt_value(stored, key, policy) {
        Some(StoredValue::Text(plaintext)) => Payload::String(plaintext.expose_secret().to_owned()),
        Some(StoredValue::Other(())) | None => value.clone(),
    }
}

mod tests;
