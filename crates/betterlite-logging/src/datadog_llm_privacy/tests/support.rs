use betterlite_models::decisions::Json;

use crate::datadog_llm_obs::values::Object;
use crate::fields::payload::StandardLoggingPayload;

const PAYLOAD_JSON: &str = r#"{
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
  "endTime": 1704067201.0,
  "completionStartTime": 1704067200.5,
  "response_time": 1.0,
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

pub(crate) fn assert_fields(actual: &Object, expected_json: &str) {
    let Json::Object(expected) = json(expected_json) else {
        panic!("the expected fixture is an object");
    };
    for (field, value) in expected {
        assert_eq!(actual.get(&field), Some(&value), "field {field}");
    }
}
