use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::alert_settings::types::AlertType;
use betterlite_config::sections::alert_settings::webhooks::WebhookUrl;
use betterlite_config::sections::settings::JsonValue;
use betterlite_config::sections::settings::settings_resolution::{
    EmptyDbValue, FieldDescriptor, ResolvedField, resolve_fields,
};
use betterlite_redact::structured::REDACTED;
use betterlite_redact::structured::redact::mask_sensitive_keys;
use betterlite_secrets::callback_vars::is_sensitive_callback_key;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;

// proxy_server.py's _EXTRA_SECRET_CALLBACK_ENV_VARS
const EXTRA_SECRET_CALLBACK_ENV_VARS: [&str; 6] = [
    "ALERTING_WEBHOOK_URL",
    "GALILEO_USERNAME",
    "GENERIC_LOGGER_HEADERS",
    "OTEL_HEADERS",
    "SLACK_WEBHOOK_URL",
    "SMTP_USERNAME",
];

// proxy_server.py's _ALERTING_SENSITIVE_VARS
const ALERTING_SENSITIVE_VARS: [&str; 3] =
    ["ALERTING_WEBHOOK_URL", "SLACK_WEBHOOK_URL", "SMTP_PASSWORD"];

const MS_TEAMS_WEBHOOK_URL: &str = "MS_TEAMS_WEBHOOK_URL";

// The three key sets the settings page applies; Teams' webhook URL is betterlite's masking deviation, since it carries a token
pub(crate) struct SecretKeys {
    pub(crate) callback: BTreeSet<String>,
    pub(crate) alerting: BTreeSet<String>,
    pub(crate) admin: BTreeSet<String>,
}

pub(crate) fn secret_keys() -> SecretKeys {
    let callback: BTreeSet<String> = EXTRA_SECRET_CALLBACK_ENV_VARS
        .iter()
        .map(|key| (*key).to_owned())
        .collect();
    let mut alerting = callback.clone();
    alerting.insert(MS_TEAMS_WEBHOOK_URL.to_owned());
    let mut admin: BTreeSet<String> = ALERTING_SENSITIVE_VARS
        .iter()
        .map(|key| (*key).to_owned())
        .collect();
    admin.insert(MS_TEAMS_WEBHOOK_URL.to_owned());
    SecretKeys {
        callback,
        alerting,
        admin,
    }
}

#[derive(Serialize)]
pub(crate) struct AlertDestination {
    pub(crate) name: &'static str,
    pub(crate) variables: JsonValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) active_alerts: Option<JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) alerts_to_webhook: Option<JsonValue>,
}

// resolve_fields over the stored environment then the process: admins see short values masked, viewers REDACTED
pub(crate) fn alert_variables(
    descriptors: &[FieldDescriptor],
    stored: &BTreeMap<String, JsonValue>,
    environment: &BTreeMap<String, String>,
    is_full_admin: bool,
    keys: &SecretKeys,
) -> JsonValue {
    let mut process_environment = BTreeMap::new();
    for descriptor in descriptors {
        if let Some(value) = environment.get(descriptor.env_var) {
            process_environment.insert(
                descriptor.env_var.to_owned(),
                SecretString::from(value.clone()),
            );
        }
    }
    let fields = resolve_fields(
        descriptors,
        stored,
        &process_environment,
        EmptyDbValue::Cleared,
    );
    let variables = variables_value(&fields);
    if is_full_admin {
        return mask(&variables, &keys.admin);
    }
    redact(&variables, &keys.alerting)
}

fn variables_value(fields: &[ResolvedField]) -> JsonValue {
    let mut variables = BTreeMap::new();
    for field in fields {
        let value = field.value.as_ref().map_or(JsonValue::Null, |text| {
            JsonValue::String(text.expose_secret().clone())
        });
        variables.insert(field.field_name.to_owned(), value);
    }
    JsonValue::Object(variables.into_iter().collect())
}

// _redact_callback_env_vars: a None stays None, and a sensitive key's value becomes the shared marker
pub(crate) fn redact(variables: &JsonValue, keys: &BTreeSet<String>) -> JsonValue {
    let Some(entries) = variables.as_object() else {
        return variables.clone();
    };
    let mut redacted = BTreeMap::new();
    for (key, value) in entries {
        let shown = if !value.is_null() && is_sensitive_callback_key(key, keys) {
            JsonValue::String(REDACTED.to_owned())
        } else {
            value.clone()
        };
        redacted.insert(key.clone(), shown);
    }
    JsonValue::Object(redacted.into_iter().collect())
}

fn mask(variables: &JsonValue, keys: &BTreeSet<String>) -> JsonValue {
    let Some(entries) = variables.as_object() else {
        return variables.clone();
    };
    mask_sensitive_keys(entries, keys)
}

pub(crate) fn active_alerts(alert_types: &[AlertType]) -> JsonValue {
    let mut entries = Vec::with_capacity(alert_types.len());
    for alert in alert_types {
        entries.push(JsonValue::String(alert.to_string()));
    }
    JsonValue::Array(entries)
}

// _apply_webhook_role_gate: every configured alert type keeps its key, and a viewer sees only the marker
pub(crate) fn webhook_map(
    webhooks: &BTreeMap<AlertType, WebhookUrl>,
    is_full_admin: bool,
) -> JsonValue {
    let mut entries = BTreeMap::new();
    for (alert_type, url) in webhooks {
        let value = if is_full_admin {
            webhook_value(url)
        } else {
            JsonValue::String(REDACTED.to_owned())
        };
        entries.insert(alert_type.to_string(), value);
    }
    JsonValue::Object(entries.into_iter().collect())
}

fn webhook_value(url: &WebhookUrl) -> JsonValue {
    match url {
        WebhookUrl::Single(secret) => JsonValue::String(secret.expose_secret().clone()),
        WebhookUrl::Multiple(urls) => JsonValue::Array(
            urls.iter()
                .map(|secret| JsonValue::String(secret.expose_secret().clone()))
                .collect(),
        ),
    }
}
