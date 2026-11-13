use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset};

use crate::datadog::payloads::legacy::{LegacyLoggingEvent, LegacyLoggingKwargs};
use crate::datadog::payloads::status::DataDogStatus;
use crate::datadog::payloads::types::DatadogPayload;
use crate::datadog::tagging::Environment;
use crate::fields::payload::StandardLoggingPayload;

const STANDARD_PAYLOAD: &str = r#"{
    "id": "chatcmpl-1",
    "trace_id": "trace-1",
    "session_id": "session-1",
    "call_type": "completion",
    "response_cost": 0.5,
    "status": "success",
    "status_fields": {"llm_api_status": "success", "guardrail_status": "not_run"},
    "total_tokens": 5,
    "prompt_tokens": 2,
    "completion_tokens": 3,
    "startTime": 1704067200.0,
    "endTime": 1704067201.5,
    "completionStartTime": 1704067200.5,
    "response_time": 1.0,
    "model_map_information": {"model_map_key": "", "model_map_value": null},
    "model": "gpt-4o",
    "api_base": "https://api.openai.com",
    "metadata": {},
    "saved_cache_cost": 0.0,
    "request_tags": [],
    "model_parameters": {},
    "hidden_params": {}
}"#;

const TAGGED_PAYLOAD: &str = r#"{
    "id": "chatcmpl-1",
    "trace_id": "trace-1",
    "session_id": "session-1",
    "call_type": "completion",
    "response_cost": 0.5,
    "status": "success",
    "status_fields": {"llm_api_status": "success", "guardrail_status": "not_run"},
    "total_tokens": 5,
    "prompt_tokens": 2,
    "completion_tokens": 3,
    "startTime": 1704067200.0,
    "endTime": 1704067201.5,
    "completionStartTime": 1704067200.5,
    "response_time": 1.0,
    "model_map_information": {"model_map_key": "", "model_map_value": null},
    "model": "gpt-4o",
    "api_base": "https://api.openai.com",
    "metadata": {"user_api_key_team_alias": "Team One", "team_id": "ignored-team"},
    "saved_cache_cost": 0.0,
    "request_tags": ["Prod A", 7],
    "model_parameters": {},
    "hidden_params": {}
}"#;

pub(crate) fn environment(pairs: &[(&str, &str)]) -> Environment {
    let values = pairs
        .iter()
        .map(|(name, value)| (String::from(*name), String::from(*value)));
    Environment::from_lookup(&BTreeMap::from_iter(values)).unwrap()
}

pub(crate) fn default_environment() -> Environment {
    Environment::from_lookup(&BTreeMap::new()).unwrap()
}

pub(crate) fn standard_payload() -> StandardLoggingPayload {
    serde_json::from_str(STANDARD_PAYLOAD).unwrap()
}

pub(crate) fn tagged_payload() -> StandardLoggingPayload {
    serde_json::from_str(TAGGED_PAYLOAD).unwrap()
}

pub(crate) fn message(payload: &DatadogPayload) -> Json {
    serde_json::from_str(&payload.message).unwrap()
}

pub(crate) fn json(encoded: &str) -> Json {
    serde_json::from_str(encoded).unwrap()
}

pub(crate) fn field<'json>(value: &'json Json, name: &str) -> &'json Json {
    match value {
        Json::Object(fields) => fields.get(name).unwrap_or_else(|| panic!("missing {name}")),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("expected a JSON object")
        }
    }
}

pub(crate) fn has_key(value: &Json, name: &str) -> bool {
    match value {
        Json::Object(fields) => fields.contains_key(name),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => false,
    }
}

pub(crate) fn moment(encoded: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(encoded).unwrap()
}

pub(crate) fn default_kwargs() -> LegacyLoggingKwargs {
    serde_json::from_str("{}").unwrap()
}

pub(crate) fn nested_json(depth: usize) -> Json {
    let mut value = Json::Text("deep".to_owned());
    for _ in 0..depth {
        value = Json::List(vec![value]);
    }
    value
}

pub(crate) fn legacy_event<'event>(
    kwargs: &'event LegacyLoggingKwargs,
    response_obj: &'event Json,
) -> LegacyLoggingEvent<'event> {
    LegacyLoggingEvent {
        kwargs,
        response_obj,
        start_time: moment("2024-01-01T00:00:00+00:00"),
        end_time: moment("2024-01-01T00:00:01.500+00:00"),
        status: DataDogStatus::Info,
        generated_id: "generated-id",
    }
}
