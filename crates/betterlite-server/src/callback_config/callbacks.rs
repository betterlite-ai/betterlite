pub(crate) mod available;

use std::collections::BTreeMap;

use betterlite_config::sections::settings::JsonValue;
use serde::Serialize;

use self::available::AVAILABLE_CALLBACKS;
use super::error::CallbackConfigError;

pub(crate) const LITELLM_SETTINGS: &str = "litellm_settings";
pub(crate) const GENERAL_SETTINGS: &str = "general_settings";
pub(crate) const ENVIRONMENT_VARIABLES: &str = "environment_variables";
pub(crate) const SUCCESS_CALLBACK: &str = "success_callback";
pub(crate) const FAILURE_CALLBACK: &str = "failure_callback";
pub(crate) const CALLBACKS: &str = "callbacks";
pub(crate) const SUCCESS_PATH: &str = "litellm_settings.success_callback";
pub(crate) const FAILURE_PATH: &str = "litellm_settings.failure_callback";
pub(crate) const CALLBACKS_PATH: &str = "litellm_settings.callbacks";
pub(crate) const ALERTING_PATH: &str = "general_settings.alerting";

#[derive(Clone, Debug, Serialize)]
pub(crate) struct CallbackRow {
    pub(crate) name: String,
    pub(crate) variables: JsonValue,
    #[serde(rename = "type")]
    pub(crate) callback_type: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) read_only: Option<bool>,
}

// _normalize_callback_alias: the four names the settings page and the runtime know by different strings
pub(crate) fn normalize_alias(name: &str) -> &str {
    match name {
        "opentelemetry" => "otel",
        "s3_v2" => "s3",
        "aws_sqs" => "sqs",
        "custom_callback_api" => "generic_api",
        other => other,
    }
}

// process_callback: the name, its environment variables from the stored section and then the process, and its type
pub(crate) fn process_callback(
    name: &str,
    callback_type: &'static str,
    stored: &BTreeMap<String, JsonValue>,
    environment: &BTreeMap<String, String>,
) -> CallbackRow {
    let mut variables = BTreeMap::new();
    for variable in available::callback_env_vars(name) {
        let value = match stored.get(*variable) {
            Some(JsonValue::Null) | None => environment
                .get(*variable)
                .map_or(JsonValue::Null, |text| JsonValue::String(text.clone())),
            Some(value) => value.clone(),
        };
        variables.insert((*variable).to_owned(), value);
    }
    CallbackRow {
        name: name.to_owned(),
        variables: JsonValue::Object(variables.into_iter().collect()),
        callback_type,
        read_only: None,
    }
}

fn invalid_config(field: &str, expected: &'static str) -> CallbackConfigError {
    CallbackConfigError::InvalidConfig {
        field: field.to_owned(),
        expected,
    }
}

// Whether a bare string in the stored value counts as one entry: the settings page reads it that way, the delete path does not
#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum SingleEntry {
    Allowed,
    Rejected,
}

// A stored callback list read as its entries: null is absent, and any other non-list is rejected with the field named
pub(crate) fn entry_list(
    value: Option<&JsonValue>,
    field: &str,
    single: SingleEntry,
) -> Result<Vec<String>, CallbackConfigError> {
    match value {
        None | Some(JsonValue::Null) => Ok(Vec::new()),
        Some(JsonValue::String(name)) if single == SingleEntry::Allowed => Ok(vec![name.clone()]),
        Some(JsonValue::Array(items)) => string_items(items, field),
        Some(
            JsonValue::String(_) | JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Object(_),
        ) => Err(invalid_config(field, "a list of strings")),
    }
}

fn string_items(items: &[JsonValue], field: &str) -> Result<Vec<String>, CallbackConfigError> {
    let mut names = Vec::with_capacity(items.len());
    for item in items {
        let JsonValue::String(name) = item else {
            return Err(CallbackConfigError::InvalidConfig {
                field: field.to_owned(),
                expected: "a list of strings",
            });
        };
        names.push(name.clone());
    }
    Ok(names)
}

// A stored section read as its fields: anything but a mapping is rejected with the section named
pub(crate) fn section_object(
    value: Option<&JsonValue>,
    section: &str,
) -> Result<BTreeMap<String, JsonValue>, CallbackConfigError> {
    let Some(present) = value.filter(|entry| !entry.is_null()) else {
        return Ok(BTreeMap::new());
    };
    let JsonValue::Object(entries) = present else {
        return Err(invalid_config(section, "an object"));
    };
    Ok(entries
        .iter()
        .map(|(key, entry)| (key.clone(), entry.clone()))
        .collect())
}

pub(crate) fn available_callbacks() -> BTreeMap<&'static str, available::UiCallback> {
    let mut callbacks = BTreeMap::new();
    for callback in &AVAILABLE_CALLBACKS {
        callbacks.insert(
            callback.key,
            available::UiCallback {
                litellm_callback_name: callback.litellm_callback_name,
                litellm_callback_params: callback.params,
                ui_name: callback.ui_callback_name,
            },
        );
    }
    callbacks
}
