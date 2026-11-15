use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::HeadroomError;
use crate::guardrail_base::inputs::GuardrailLogTarget;
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
        Some(GuardrailValue::Text(text)) if !text.is_empty() => Some(text.as_str()),
        _ => None,
    }
}

pub(crate) fn call_id(request_data: &BTreeMap<String, GuardrailValue>) -> Option<&str> {
    match request_data.get(CALL_ID_FIELD) {
        Some(GuardrailValue::Text(text)) => Some(text.as_str()),
        _ => None,
    }
}

pub(crate) fn resolve_call_id(
    logging_obj: Option<&GuardrailLogTarget>,
    request_data: &BTreeMap<String, GuardrailValue>,
) -> Option<String> {
    if let Some(call_id) = logging_call_id(logging_obj) {
        return Some(call_id.to_owned());
    }
    call_id(request_data).map(str::to_owned)
}

fn logging_call_id(logging_obj: Option<&GuardrailLogTarget>) -> Option<&str> {
    let target = logging_obj?;
    [
        target.model_call_params.as_ref(),
        target.litellm_params.as_ref(),
    ]
    .into_iter()
    .flatten()
    .find_map(call_id_in)
}

fn call_id_in(params: &BTreeMap<String, GuardrailValue>) -> Option<&str> {
    match params.get(CALL_ID_FIELD) {
        Some(GuardrailValue::Text(call_id)) if !call_id.is_empty() => Some(call_id.as_str()),
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
