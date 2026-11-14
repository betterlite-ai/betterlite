use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::error::HeadroomError;
use crate::headroom_settings::BYPASS_HEADER;
use crate::primitives::json_value::JsonValue;

pub(crate) const BACKGROUND_FIELD: &str = "background";
pub(crate) const CALL_ID_FIELD: &str = "litellm_call_id";
pub(crate) const MESSAGES_FIELD: &str = "messages";
pub(crate) const MODEL_FIELD: &str = "model";
const HEADERS_FIELD: &str = "headers";
const PROXY_SERVER_REQUEST_FIELD: &str = "proxy_server_request";

pub(crate) fn bypass(request_data: &BTreeMap<String, GuardrailValue>) -> bool {
    let Some(GuardrailValue::Object(request)) = request_data.get(PROXY_SERVER_REQUEST_FIELD) else {
        return false;
    };
    let Some(GuardrailValue::Object(headers)) = request.get(HEADERS_FIELD) else {
        return false;
    };
    match headers.get(BYPASS_HEADER) {
        Some(GuardrailValue::Text(value)) => value.as_bytes().eq_ignore_ascii_case(b"true"),
        _ => false,
    }
}

pub(crate) fn background(request_data: &BTreeMap<String, GuardrailValue>) -> bool {
    match request_data.get(BACKGROUND_FIELD) {
        Some(GuardrailValue::Boolean(flag)) => *flag,
        _ => false,
    }
}

pub(crate) fn model(request_data: &BTreeMap<String, GuardrailValue>) -> Option<&str> {
    match request_data.get(MODEL_FIELD) {
        Some(GuardrailValue::Text(text)) => Some(text.as_str()),
        _ => None,
    }
}

pub(crate) fn call_id(request_data: &BTreeMap<String, GuardrailValue>) -> Option<&str> {
    match request_data.get(CALL_ID_FIELD) {
        Some(GuardrailValue::Text(text)) => Some(text.as_str()),
        _ => None,
    }
}

pub(crate) fn untranslated_messages(
    request_data: &BTreeMap<String, GuardrailValue>,
) -> Result<Option<JsonValue>, HeadroomError> {
    let Some(messages) = request_data.get(MESSAGES_FIELD) else {
        return Ok(None);
    };
    let encoded = serde_json::to_value(messages)?;
    let decoded = serde_json::from_value(encoded)?;
    Ok(Some(decoded))
}
