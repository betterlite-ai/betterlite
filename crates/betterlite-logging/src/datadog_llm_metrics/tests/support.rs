use betterlite_models::decisions::Json;

use crate::datadog_llm_metrics::metrics::span_metrics;
use crate::datadog_llm_obs::values::Object;
use crate::datadog_llm_privacy::record_object;
use crate::fields::payload::StandardLoggingPayload;
use crate::fields::payload::records::ErrorInformation;

const PAYLOAD_JSON: &str = r#"{
  "id": "chatcmpl-1",
  "trace_id": "trace-1",
  "session_id": "session-1",
  "call_type": "completion",
  "response_cost": 0.5,
  "status": "success",
  "status_fields": {"llm_api_status": "success", "guardrail_status": "not_run"},
  "total_tokens": 10,
  "prompt_tokens": 4,
  "completion_tokens": 6,
  "startTime": 1704067200.0,
  "endTime": 1704067201.5,
  "completionStartTime": 1704067200.5,
  "response_time": 1.5,
  "model_map_information": {},
  "model": "gpt-4o",
  "model_group": "gpt-4o",
  "api_base": "https://api.openai.com",
  "metadata": {},
  "saved_cache_cost": 0.0,
  "request_tags": [],
  "model_parameters": {},
  "hidden_params": {},
  "standard_built_in_tools_params": null
}"#;

pub(crate) fn payload() -> StandardLoggingPayload {
    serde_json::from_str(PAYLOAD_JSON).expect("the payload fixture parses")
}

pub(crate) fn error_information(text: &str) -> ErrorInformation {
    serde_json::from_str(text).expect("the error information fixture parses")
}

pub(crate) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the fixture is JSON")
}

pub(crate) fn object(text: &str) -> Object {
    match json(text) {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("the fixture is an object")
        }
    }
}

pub(crate) fn metrics(payload: &StandardLoggingPayload, tool_output_tokens: Option<f64>) -> Object {
    let metrics = span_metrics(payload, tool_output_tokens).expect("the metrics build");
    record_object("metrics", &metrics).expect("the metrics encode")
}

pub(crate) fn assert_fields(actual: &Object, expected_json: &str) {
    let Json::Object(expected) = json(expected_json) else {
        panic!("the expected fixture is an object");
    };
    for (field, value) in expected {
        assert_eq!(actual.get(&field), Some(&value), "field {field}");
    }
}

pub(crate) fn assert_absent(actual: &Object, field: &str) {
    assert!(!actual.contains_key(field), "field {field} is present");
}

pub(crate) fn assert_all_absent(actual: &Object, fields: &[&str]) {
    for field in fields {
        assert_absent(actual, field);
    }
}
