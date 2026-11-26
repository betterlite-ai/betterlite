use std::mem;

use betterlite_config::sections::settings::{JsonValue, Section};
use betterlite_pyhost::json_values::JsonObject;

use super::state::PASS_THROUGH_ENDPOINTS;

// _DB_OVERLAY_REMOTE_MODULE_STR_FIELDS and _LIST_FIELDS: the keys whose text reaches get_instance_fn
const LITELLM_MODULE_FIELDS: [&str; 1] = ["post_call_rules"];
const GENERAL_MODULE_FIELDS: [&str; 7] = [
    "custom_auth",
    "custom_key_generate",
    "custom_key_update",
    "custom_key_policy",
    "custom_team_metadata_validate",
    "custom_sso",
    "custom_ui_sso_sign_in_handler",
];
const LITELLM_MODULE_LISTS: [&str; 4] = [
    "callbacks",
    "success_callback",
    "failure_callback",
    "audit_log_callbacks",
];

// _scrub_db_overlay_remote_module_loads: the config file may name an s3:// or gcs:// module, a stored row may not
#[must_use]
pub fn scrub_remote_modules(section: Section, value: &JsonValue) -> JsonValue {
    let mut scrubbed = value.clone();
    let JsonValue::Object(fields) = &mut scrubbed else {
        return scrubbed;
    };
    match section {
        Section::LitellmSettings => {
            refuse_module_fields(section, fields, &LITELLM_MODULE_FIELDS);
            for name in LITELLM_MODULE_LISTS {
                refuse_list_modules(fields, name);
            }
            refuse_custom_handlers(fields);
            refuse_guardrail_modules(fields);
        }
        Section::GeneralSettings => {
            refuse_module_fields(section, fields, &GENERAL_MODULE_FIELDS);
            if let Some(validator) = take_remote(
                fields
                    .get_mut("litellm_jwtauth")
                    .and_then(|jwt| jwt.get_mut("custom_validate")),
            ) {
                tracing::warn!(
                    "Refused remote-URL custom_validate from DB-overlay general_settings.litellm_jwtauth: {validator}"
                );
            }
            refuse_pass_through_targets(fields);
        }
        Section::RouterSettings | Section::EnvironmentVariables | Section::UiSettings => {}
    }
    scrubbed
}

fn is_remote_module(value: &JsonValue) -> bool {
    value
        .as_str()
        .is_some_and(|text| text.starts_with("s3://") || text.starts_with("gcs://"))
}

// a refused module becomes null and the refused text comes back for the warning
fn take_remote(slot: Option<&mut JsonValue>) -> Option<JsonValue> {
    slot.filter(|value| is_remote_module(value))
        .map(|value| mem::replace(value, JsonValue::Null))
}

// the number of remote entries dropped from a list; anything but a list is left alone
fn drop_remote(slot: Option<&mut JsonValue>) -> usize {
    let Some(items) = slot.and_then(JsonValue::as_array_mut) else {
        return 0;
    };
    let before = items.len();
    items.retain(|item| !is_remote_module(item));
    before.saturating_sub(items.len())
}

fn refuse_module_fields(section: Section, fields: &mut JsonObject, names: &[&str]) {
    for name in names {
        if let Some(refused) = take_remote(fields.get_mut(*name)) {
            tracing::warn!(
                "Refused remote-URL value for DB-overlay {}.{name}={refused}; only config.yaml entries may reference s3:// / gcs:// modules.",
                section.name()
            );
        }
    }
}

fn refuse_list_modules(fields: &mut JsonObject, name: &str) {
    let refused = drop_remote(fields.get_mut(name));
    if refused > 0 {
        tracing::warn!(
            "Refused {refused} remote-URL entries from DB-overlay litellm_settings.{name}; only config.yaml entries may reference s3:// / gcs:// modules."
        );
    }
}

// the entry a refused module sits in comes back with the refused text, so the caller names it in its warning
fn refuse_item_modules(
    fields: &mut JsonObject,
    list: &str,
    module_field: &str,
) -> Vec<(JsonValue, JsonValue)> {
    let Some(JsonValue::Array(items)) = fields.get_mut(list) else {
        return Vec::new();
    };
    let mut refused = Vec::new();
    for item in items {
        if let Some(module) = take_remote(item.get_mut(module_field)) {
            refused.push((item.clone(), module));
        }
    }
    refused
}

fn refuse_custom_handlers(fields: &mut JsonObject) {
    for (_, handler) in refuse_item_modules(fields, "custom_provider_map", "custom_handler") {
        tracing::warn!(
            "Refused remote-URL custom_handler from DB-overlay litellm_settings.custom_provider_map: {handler}"
        );
    }
}

// v1 entries nest the guardrail under its name and v2 entries under litellm_params, so every mapping value of an entry is scrubbed
fn refuse_guardrail_modules(fields: &mut JsonObject) {
    let Some(JsonValue::Array(guardrails)) = fields.get_mut("guardrails") else {
        return;
    };
    for entry in guardrails {
        let JsonValue::Object(entry_fields) = entry else {
            continue;
        };
        for inner in entry_fields.values_mut() {
            let refused = drop_remote(inner.get_mut("callbacks"));
            if refused > 0 {
                tracing::warn!(
                    "Refused {refused} remote-URL entries from DB-overlay litellm_settings.guardrails[...].callbacks"
                );
            }
            if let Some(module) = take_remote(inner.get_mut("guardrail")) {
                tracing::warn!(
                    "Refused remote-URL guardrail module from DB-overlay litellm_settings.guardrails[...].guardrail: {module}"
                );
            }
        }
    }
}

fn refuse_pass_through_targets(fields: &mut JsonObject) {
    for (endpoint, target) in refuse_item_modules(fields, PASS_THROUGH_ENDPOINTS, "target") {
        let path = endpoint.get("path").cloned().unwrap_or_default();
        tracing::warn!(
            "Refused remote-URL target from DB-overlay general_settings.pass_through_endpoints (path={path}): {target}"
        );
    }
}
