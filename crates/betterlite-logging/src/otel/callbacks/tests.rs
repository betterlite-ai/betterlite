#![cfg(test)]

use std::sync::Arc;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::callback_logger::event::LoggerEvent;

pub(crate) const SUCCESS_RESPONSE: &str = r#"{"id": "chatcmpl-response-1", "model": "gpt-4o-2024-08-06", "usage": {"total_tokens": 11, "completion_tokens": 3, "prompt_tokens": 8}, "choices": [{"index": 0, "finish_reason": "stop", "message": {"role": "assistant", "content": "Hi there"}}]}"#;

pub(crate) const DEFAULT_MESSAGES: &str = r#"[{"role": "user", "content": "Hello"}]"#;

pub(crate) const DEFAULT_MODEL_PARAMETERS: &str = r#"{"temperature": 0.2, "max_tokens": 16, "stop": ["END"], "top_p": 0.9, "frequency_penalty": 0.1, "presence_penalty": 0.3, "user": "end-user-1", "functions": ["lookup"]}"#;

// the success payload the logging pipeline builds, with the parts a vendor callback reads swapped per case
const TEMPLATE: &str = r#"{
    "id": "chatcmpl-1",
    "trace_id": "trace-1",
    "session_id": "session-1",
    "litellm_call_id": "call-1",
    "call_type": "completion",
    "stream": false,
    "response_cost": 0.0000125,
    "status": "success",
    "status_fields": {"llm_api_status": "success", "guardrail_status": "not_run"},
    "total_tokens": 11,
    "prompt_tokens": 8,
    "completion_tokens": 3,
    "startTime": 1704067200.0,
    "endTime": 1704067201.5,
    "completionStartTime": 1704067200.5,
    "response_time": 1.5,
    "model_map_information": {},
    "model": "gpt-4o",
    "api_base": "https://api.openai.com",
    "metadata": {"user_api_key_user_id": "user-1"},
    "end_user": "end-user-1",
    "saved_cache_cost": 0.0,
    "request_tags": [],
    "model_parameters": MODEL_PARAMETERS,
    "hidden_params": {},
    "messages": MESSAGES,
    "response": RESPONSE
}"#;

pub(crate) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the fixture JSON decodes")
}

pub(crate) fn payload(response: &str, messages: &str) -> String {
    payload_with(response, messages, DEFAULT_MODEL_PARAMETERS)
}

pub(crate) fn payload_with(response: &str, messages: &str, model_parameters: &str) -> String {
    TEMPLATE
        .replace("MODEL_PARAMETERS", model_parameters)
        .replace("MESSAGES", messages)
        .replace("RESPONSE", response)
}

pub(crate) fn from_payload(text: &str) -> LoggerEvent {
    event_with_metadata(text, IndexMap::new())
}

pub(crate) fn event() -> LoggerEvent {
    from_payload(&payload(SUCCESS_RESPONSE, DEFAULT_MESSAGES))
}

pub(crate) fn event_with_metadata(
    text: &str,
    litellm_metadata: IndexMap<String, Json>,
) -> LoggerEvent {
    LoggerEvent {
        payload: Arc::new(serde_json::from_str(text).expect("the fixture payload decodes")),
        dynamic_params: Arc::new(Json::Object(IndexMap::new())),
        litellm_metadata: Arc::new(litellm_metadata),
        stream: false,
        guardrail_snapshot: None,
    }
}
