use crate::structured::{Payload, REDACTED, SensitiveDataMasker};

// proxy_server.py's _EXTRA_SECRET_GENERAL_SETTINGS_FIELDS
const EXTRA_SECRET_SETTINGS_FIELDS: [&str; 4] = [
    "database_url",
    "database_extra_connection_params",
    "pass_through_endpoints",
    "alert_to_webhook_url",
];
// proxy_server.py's _REDACT_SECRET_MAX_DEPTH
const SECRET_MAX_DEPTH: usize = 10;
const ENVIRONMENT_VARIABLES: &str = "environment_variables";

#[must_use]
pub fn is_secret_setting(name: &str) -> bool {
    if EXTRA_SECRET_SETTINGS_FIELDS.contains(&name) {
        return true;
    }
    SensitiveDataMasker::default().is_sensitive_key(name, &[])
}

#[must_use]
pub fn redact_secret_values(value: &Payload) -> Payload {
    match value {
        Payload::Object(_) | Payload::Array(_) => redact_at_depth(value, 0),
        Payload::Null | Payload::Bool(_) | Payload::Number(_) | Payload::String(_) => value.clone(),
    }
}

#[must_use]
pub fn redact_param_for_logging(name: Option<&str>, value: &Payload) -> Payload {
    if name == Some(ENVIRONMENT_VARIABLES) && value.is_object() {
        return redact_every_value(value);
    }
    redact_secret_values(value)
}

#[must_use]
pub fn redact_setting_value(name: &str, value: &Payload, is_full_admin: bool) -> Payload {
    if is_full_admin {
        return value.clone();
    }
    if is_secret_setting(name) {
        return Payload::String(REDACTED.to_owned());
    }
    redact_secret_values(value)
}

#[must_use]
pub fn dump_redacted_config(value: Option<&Payload>, redact_all: bool) -> Option<String> {
    let given = value?;
    let redacted = if redact_all {
        redact_every_value(given)
    } else {
        redact_secret_values(given)
    };
    serde_json::to_string(&redacted).ok()
}

// proxy_server.py's _redact_secret_values_in_obj: past the cap the whole subtree fails closed
fn redact_at_depth(value: &Payload, depth: usize) -> Payload {
    if depth >= SECRET_MAX_DEPTH {
        return Payload::String(REDACTED.to_owned());
    }
    match value {
        Payload::Object(fields) => Payload::Object(
            fields
                .iter()
                .map(|(key, child)| {
                    let redacted = if is_secret_setting(key) {
                        Payload::String(REDACTED.to_owned())
                    } else {
                        redact_at_depth(child, depth.saturating_add(1))
                    };
                    (key.clone(), redacted)
                })
                .collect(),
        ),
        Payload::Array(items) => Payload::Array(
            items
                .iter()
                .map(|item| redact_at_depth(item, depth.saturating_add(1)))
                .collect(),
        ),
        Payload::Null | Payload::Bool(_) | Payload::Number(_) | Payload::String(_) => value.clone(),
    }
}

// an object keeps every key and hides each value whatever the key looks like; anything else takes the secret walk
fn redact_every_value(value: &Payload) -> Payload {
    match value {
        Payload::Object(fields) => Payload::Object(
            fields
                .keys()
                .map(|key| (key.clone(), Payload::String(REDACTED.to_owned())))
                .collect(),
        ),
        Payload::Array(_)
        | Payload::Null
        | Payload::Bool(_)
        | Payload::Number(_)
        | Payload::String(_) => redact_secret_values(value),
    }
}

mod tests;
