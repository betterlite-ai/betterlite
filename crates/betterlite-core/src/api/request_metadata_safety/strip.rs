use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::request_destination::RequestData;
use crate::api::request_metadata_safety::fields::{
    BLOCKED_CALLBACK_PARAMS, CLIENT_PRICING_METADATA_FIELDS, ROUTER_RESERVED_FIELDS,
    TRUSTED_CALLBACK_VARS_FIELD, UNTRUSTED_HEADER_CONTROLS, is_client_pricing_control,
};

const TURN_OFF_MESSAGE_LOGGING_KEY: &str = "turn_off_message_logging";
const LITELLM_PARAMS_KEY: &str = "litellm_params";
const METADATA_KEY: &str = "metadata";
const LITELLM_METADATA_KEY: &str = "litellm_metadata";
// iter_client_callback_metadata_dicts yields the nested params bucket before the two top-level slots
const CLIENT_CALLBACK_METADATA_SLOTS: [&str; 2] = [LITELLM_METADATA_KEY, METADATA_KEY];
const PRICING_METADATA_SLOTS: [&str; 2] = [METADATA_KEY, LITELLM_METADATA_KEY];

// ast-grep-ignore: no-long-names
pub fn strip_untrusted_request_header_controls<V>(
    headers: &mut IndexMap<String, V>,
    allow_opt_out: bool,
) {
    if allow_opt_out {
        return;
    }
    headers
        .retain(|name, _value| !UNTRUSTED_HEADER_CONTROLS.contains(&name.to_lowercase().as_str()));
}

// ast-grep-ignore: no-long-names
pub fn strip_client_message_redaction_opt_out(data: &mut RequestData) {
    let mut stripped: Vec<String> = Vec::new();
    if strip_false_like(data) {
        stripped.push(TURN_OFF_MESSAGE_LOGGING_KEY.to_owned());
    }
    if let Some(metadata) = nested_litellm_params_metadata(data) {
        strip_opt_out("litellm_params.metadata", metadata, &mut stripped);
    }
    for slot in CLIENT_CALLBACK_METADATA_SLOTS {
        if let Some(Json::Object(metadata)) = data.get_mut(slot) {
            strip_opt_out(slot, metadata, &mut stripped);
        }
    }
    if stripped.is_empty() {
        return;
    }
    let fields = stripped.join(", ");
    tracing::debug!(
        "stripped client-supplied message-redaction opt-out fields from request body: {fields}. Set `allow_client_message_redaction_opt_out: true` on the key or team metadata to keep these values."
    );
}

// ast-grep-ignore: no-long-names
pub fn strip_client_callback_credentials(data: &mut RequestData) {
    let mut stripped: Vec<String> = Vec::new();
    remove_callback_credentials("body", data, &mut stripped);
    if let Some(metadata) = nested_litellm_params_metadata(data) {
        remove_callback_credentials("litellm_params.metadata", metadata, &mut stripped);
    }
    for slot in CLIENT_CALLBACK_METADATA_SLOTS {
        if let Some(Json::Object(metadata)) = data.get_mut(slot) {
            remove_callback_credentials(slot, metadata, &mut stripped);
        }
    }
    data.shift_remove(TRUSTED_CALLBACK_VARS_FIELD);
    if stripped.is_empty() {
        return;
    }
    stripped.sort();
    let fields = stripped.join(", ");
    tracing::debug!(
        "stripped client-supplied callback credentials from request: {fields}. Configure these on the team or key callback settings instead."
    );
}

pub fn strip_client_pricing_overrides(data: &mut RequestData) {
    let mut stripped: Vec<String> = Vec::new();
    for name in data.keys() {
        if is_client_pricing_control(name) {
            stripped.push(name.clone());
        }
    }
    for name in &stripped {
        data.shift_remove(name);
    }
    for slot in PRICING_METADATA_SLOTS {
        let Some(Json::Object(metadata)) = data.get_mut(slot) else {
            continue;
        };
        for field in CLIENT_PRICING_METADATA_FIELDS {
            if metadata.shift_remove(field).is_some() {
                stripped.push(format!("{slot}.{field}"));
            }
        }
    }
    if stripped.is_empty() {
        return;
    }
    let fields = stripped.join(", ");
    tracing::debug!(
        "stripped client-supplied pricing fields from request body: {fields}. Set `allow_client_pricing_override: true` on the key or team metadata to keep these values."
    );
}

pub fn strip_router_reserved_metadata(data: &mut RequestData) {
    for slot in PRICING_METADATA_SLOTS {
        let Some(Json::Object(metadata)) = data.get_mut(slot) else {
            continue;
        };
        for field in ROUTER_RESERVED_FIELDS {
            if metadata.shift_remove(field).is_some() {
                tracing::debug!(
                    "stripped router-reserved metadata field from request body: {slot}.{field}"
                );
            }
        }
    }
}

fn strip_opt_out(label: &str, metadata: &mut IndexMap<String, Json>, stripped: &mut Vec<String>) {
    if strip_false_like(metadata) {
        stripped.push(format!("{label}.{TURN_OFF_MESSAGE_LOGGING_KEY}"));
    }
}

fn strip_false_like(metadata: &mut IndexMap<String, Json>) -> bool {
    let Some(value) = metadata.get(TURN_OFF_MESSAGE_LOGGING_KEY) else {
        return false;
    };
    if !is_false_like(value) {
        return false;
    }
    metadata.shift_remove(TURN_OFF_MESSAGE_LOGGING_KEY);
    true
}

// only an explicit false, or the spellings a client uses for it, opts out of message logging
fn is_false_like(value: &Json) -> bool {
    match value {
        Json::Bool(flag) => !*flag,
        // ast-grep-ignore: no-python-habits - LiteLLM's _is_false_like lowercases the text and matches these four spellings
        Json::Text(text) => matches!(
            text.trim().to_lowercase().as_str(),
            "false" | "0" | "no" | "off"
        ),
        Json::Null | Json::Number(_) | Json::List(_) | Json::Object(_) => false,
    }
}

fn nested_litellm_params_metadata(data: &mut RequestData) -> Option<&mut IndexMap<String, Json>> {
    let Some(Json::Object(litellm_params)) = data.get_mut(LITELLM_PARAMS_KEY) else {
        return None;
    };
    let Some(Json::Object(metadata)) = litellm_params.get_mut(METADATA_KEY) else {
        return None;
    };
    Some(metadata)
}

fn remove_callback_credentials(
    label: &str,
    container: &mut IndexMap<String, Json>,
    stripped: &mut Vec<String>,
) {
    for field in BLOCKED_CALLBACK_PARAMS {
        if container.shift_remove(field).is_some() {
            stripped.push(format!("{label}.{field}"));
        }
    }
}
