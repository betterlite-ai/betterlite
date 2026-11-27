use betterlite_core::agents::json::AgentJson;
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_logging::prometheus_latency::input_sequence_length::{
    InputLengthInputs, UsageMapping,
};
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Object as JsonMap;
use indexmap::IndexMap;
use rust_decimal::Decimal;

pub(crate) fn as_object(value: &Json) -> Option<&IndexMap<String, Json>> {
    match value {
        Json::Object(fields) => Some(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

pub(crate) fn as_text(value: &Json) -> Option<&str> {
    match value {
        Json::Text(text) => Some(text.as_str()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

pub(crate) fn request_json(request_data: &JsonMap) -> Json {
    serde_json::from_value(AgentJson::Object(request_data.clone()))
        .unwrap_or_else(|_error| Json::Object(IndexMap::new()))
}

pub(crate) fn metadata_text(
    metadata: Option<&IndexMap<String, Json>>,
    key: &str,
) -> Option<String> {
    // a metadata value that is not text reads as absent rather than as an error
    metadata?.get(key).and_then(as_text).map(str::to_owned)
}

fn integer_value(value: Option<&Json>) -> Option<i64> {
    let Json::Number(number) = value? else {
        return None;
    };
    number.as_i64()
}

// litellm_params.metadata.queue_time_seconds, added to the total latency when a producer recorded it
pub(crate) fn queue_time_seconds(metadata: &IndexMap<String, Json>) -> Option<Decimal> {
    let Json::Number(number) = metadata.get("queue_time_seconds")? else {
        return None;
    };
    let text = number.to_string();
    text.parse::<Decimal>()
        .ok()
        .or_else(|| Decimal::from_scientific(&text).ok())
}

// the six inputs of B3878's input-sequence-length reader, from the typed payload instead of the kwargs response object
pub(crate) fn input_length_inputs(payload: &StandardLoggingPayload) -> InputLengthInputs {
    let reported = payload
        .response
        .as_ref()
        .and_then(as_object)
        .and_then(|fields| fields.get("usage"));
    let metadata_usage = payload
        .metadata
        .usage_object
        .as_ref()
        .and_then(as_object)
        .map_or(UsageMapping::Absent, |fields| UsageMapping::Mapping {
            prompt_tokens: integer_value(fields.get("prompt_tokens")),
        });
    let response_object = reported
        .and_then(as_object)
        .map_or(UsageMapping::Absent, |fields| UsageMapping::Mapping {
            prompt_tokens: integer_value(fields.get("prompt_tokens")),
        });
    let response_has_usage = matches!(reported, Some(value) if !matches!(value, Json::Null));
    InputLengthInputs {
        prompt_tokens: Some(payload.prompt_tokens),
        combined_usage: None,
        response_has_usage,
        metadata_usage,
        response_object,
    }
}
