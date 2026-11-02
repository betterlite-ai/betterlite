use betterlite_redact::structured::SensitiveDataMasker;
use serde_json::{Map, Value};

use super::{CACHE_SENSITIVE_FIELDS, REDACTED_VALUE};

// `url` can embed a password (redis://:secret@host:6379/1) but has no sensitive-looking name, so it stays in the explicit set.
#[must_use]
pub fn is_credential_field(key: &str) -> bool {
    CACHE_SENSITIVE_FIELDS.contains(&key)
        || SensitiveDataMasker::default().is_sensitive_key(key, &[])
}

fn redact_where<F>(settings: &Map<String, Value>, should_redact: F) -> Map<String, Value>
where
    F: Fn(&str, &Value) -> bool,
{
    settings
        .iter()
        .map(|(key, value)| {
            let next = if should_redact(key, value) {
                Value::String(REDACTED_VALUE.to_owned())
            } else {
                value.clone()
            };
            (key.clone(), next)
        })
        .collect()
}

#[must_use]
pub fn redact_credentials(settings: &Map<String, Value>) -> Map<String, Value> {
    redact_where(settings, |key, value| {
        !value.is_null() && is_credential_field(key)
    })
}

#[must_use]
pub fn redact_settings(settings: &Map<String, Value>) -> Map<String, Value> {
    settings
        .keys()
        .map(|key| (key.clone(), Value::String(REDACTED_VALUE.to_owned())))
        .collect()
}
