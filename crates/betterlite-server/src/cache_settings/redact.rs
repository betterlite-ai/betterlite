use betterlite_redact::structured::SensitiveDataMasker;
use serde_json::{Map, Value};

use super::{CACHE_SENSITIVE_FIELDS, REDACTED_VALUE};

// `url` can embed a password (redis://:secret@host:6379/1) but has no sensitive-looking name, so it stays in the explicit set.
#[must_use]
pub fn is_credential_field(key: &str) -> bool {
    CACHE_SENSITIVE_FIELDS.contains(&key)
        || SensitiveDataMasker::default().is_sensitive_key(key, &[])
}

#[must_use]
pub fn redact_credentials(settings: &Map<String, Value>) -> Map<String, Value> {
    settings
        .iter()
        .map(|(key, value)| {
            let redacted = !value.is_null() && is_credential_field(key);
            let next = if redacted {
                Value::String(REDACTED_VALUE.to_owned())
            } else {
                value.clone()
            };
            (key.clone(), next)
        })
        .collect()
}

#[must_use]
pub fn redact_settings(settings: Option<&Map<String, Value>>) -> Map<String, Value> {
    let Some(present) = settings.filter(|map| !map.is_empty()) else {
        return Map::new();
    };
    present
        .keys()
        .map(|key| (key.clone(), Value::String(REDACTED_VALUE.to_owned())))
        .collect()
}
