use std::collections::BTreeMap;

use betterlite_core::agents::json::AgentJson;
use betterlite_core::agents::provider::a2a::{IntegerId, JsonRpcId};

use crate::agent_card::ProtocolVersion;

pub mod version_conversion;

pub type JsonObject = BTreeMap<String, AgentJson>;

#[must_use]
pub fn request_id(value: &AgentJson) -> Option<JsonRpcId> {
    match value {
        AgentJson::String(text) => Some(JsonRpcId::String(text.clone())),
        AgentJson::Number(number) => IntegerId::try_from(number.clone())
            .ok()
            .map(JsonRpcId::Integer),
        AgentJson::Null | AgentJson::Bool(_) | AgentJson::Array(_) | AgentJson::Object(_) => None,
    }
}

#[must_use]
pub fn normalize_jsonrpc_response(
    content: JsonObject,
    target: ProtocolVersion,
    method: &str,
) -> JsonObject {
    convert_payload(content, |result| {
        version_conversion::result_for_method(result, target, method)
    })
}

#[must_use]
pub fn normalize_stream_event(
    event: JsonObject,
    target: ProtocolVersion,
    request_id: Option<&JsonRpcId>,
) -> JsonObject {
    convert_payload(event, |result| {
        version_conversion::stream_for(result, target, request_id)
    })
}

#[must_use]
pub fn normalize_request_params(
    params: JsonObject,
    served: ProtocolVersion,
    method: &str,
) -> JsonObject {
    if served == ProtocolVersion::V0_3 {
        return params;
    }
    let converted = version_conversion::request_for(
        &AgentJson::Object(params.clone().into_iter().collect()),
        method,
    );
    converted.unwrap_or(params)
}

#[must_use]
pub fn normalize_agent_card(card: JsonObject, target: ProtocolVersion) -> JsonObject {
    if !version_conversion::card_needs_conversion(&card, target) {
        return card;
    }
    version_conversion::convert_card(&card, target).unwrap_or(card)
}

// Errors and non-object results pass through untouched, and a failed conversion keeps the payload
fn convert_payload<F>(payload: JsonObject, convert: F) -> JsonObject
where
    F: FnOnce(&AgentJson) -> Option<JsonObject>,
{
    if payload.get("error").is_some_and(|error| !error.is_null()) {
        return payload;
    }
    let Some(result) = payload.get("result").filter(|result| result.is_object()) else {
        return payload;
    };
    match convert(result) {
        Some(converted) => {
            let mut updated = payload;
            updated.insert(
                "result".to_owned(),
                AgentJson::Object(converted.into_iter().collect()),
            );
            updated
        }
        None => payload,
    }
}
