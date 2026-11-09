use std::collections::BTreeMap;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use uuid::Uuid;

use crate::fields::payload::StandardLoggingPayload;
use crate::newrelic_extraction::content::ContentPolicy;
use crate::newrelic_extraction::ids::IdSource;
use crate::newrelic_extraction::sources::{CallKwargs, EventSources};

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

pub(super) struct FixedIds(Uuid);

impl IdSource for FixedIds {
    fn next_id(&self) -> Uuid {
        self.0
    }
}

pub(super) fn ids(value: u128) -> FixedIds {
    FixedIds(Uuid::from_u128(value))
}

pub(super) fn payload() -> StandardLoggingPayload {
    serde_json::from_str(STANDARD_PAYLOAD).unwrap()
}

pub(super) fn object(encoded: &str) -> IndexMap<String, Json> {
    match serde_json::from_str(encoded).unwrap() {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("expected a JSON object: {encoded}")
        }
    }
}

pub(super) fn json(encoded: &str) -> Json {
    serde_json::from_str(encoded).unwrap()
}

pub(super) fn call_kwargs<'event>() -> CallKwargs<'event> {
    CallKwargs {
        messages: None,
        model: None,
        litellm_call_id: None,
        custom_llm_provider: None,
        headers: None,
        optional_params: None,
        llm_api_duration_ms: None,
        start_time: None,
        end_time: None,
    }
}

pub(super) fn sources<'event>(
    kwargs: &'event CallKwargs<'event>,
    response: &'event IndexMap<String, Json>,
    payload: Option<&'event StandardLoggingPayload>,
) -> EventSources<'event> {
    EventSources {
        kwargs,
        response,
        standard_logging_object: payload,
    }
}

pub(super) fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn content_policy(environment: &dyn EnvironmentLookup) -> ContentPolicy<'_> {
    ContentPolicy {
        turn_off_message_logging: false,
        redact_message_logging: false,
        environment,
    }
}
