#![cfg(test)]

use chrono::{TimeZone, Utc};
use serde_json::json;

#[test]
fn payloads_link_trace_and_strip_nulls() {
    let response = json!({"model":"m","object":"chat.completion","created":1,"usage":{"prompt_tokens":1,"completion_tokens":null}});
    let trace = betterlite_logging::opik::build_trace_payload(
        "p",
        "trace",
        response.as_object().unwrap(),
        Utc.timestamp_millis_opt(0).unwrap(),
        Utc.timestamp_millis_opt(1).unwrap(),
        json!({}),
        json!({}),
        serde_json::Map::new(),
        vec![],
        None,
    );
    assert_eq!(trace["id"], "trace");
    assert!(!trace.contains_key("thread_id"));
    let span = betterlite_logging::opik::build_span_payload(
        "p",
        "trace",
        None,
        response.as_object().unwrap(),
        Utc.timestamp_millis_opt(0).unwrap(),
        Utc.timestamp_millis_opt(1).unwrap(),
        json!({}),
        json!({}),
        serde_json::Map::new(),
        vec![],
        None,
        Some("google_ai"),
    );
    assert_eq!(span["trace_id"], "trace");
    assert!(!span.contains_key("parent_span_id"));
    assert_eq!(span["usage"], json!({"prompt_tokens":1}));
}

#[test]
fn orchestration_creates_trace_and_applies_metadata() {
    let kwargs = json!({
        "custom_llm_provider": "gemini",
        "response_cost": 0.25,
        "litellm_params": {"metadata": {"opik": {"project_name": "request", "tags": ["request"]}}, "proxy_server_request": {"headers": {"opik_project_name": "header", "opik_tags": "[\"header-tag\"]"}}},
        "standard_logging_object": {
            "metadata": {"user_api_key_auth_metadata": {"opik": {"project_name": "auth"}}, "requester_metadata": {"opik": {"thread_id": "thread"}}},
            "messages": [{"role": "user", "content": "hi"}],
            "response": {"answer": "ok"},
            "model": "m",
            "type": "completion",
            "status": "success"
        }
    });
    let response = json!({"model": "m", "object": "chat.completion", "created": 1, "usage": {"total_tokens": 2}});
    let (trace, span) = betterlite_logging::opik::build_opik_payload(
        kwargs.as_object().unwrap(),
        response.as_object().unwrap(),
        Utc.timestamp_millis_opt(0).unwrap(),
        Utc.timestamp_millis_opt(1).unwrap(),
        "default",
    );
    let trace = trace.expect("new requests create a trace");
    assert_eq!(trace["project_name"], "header");
    assert_eq!(trace["thread_id"], "thread");
    assert_eq!(trace["tags"], json!(["request", "gemini", "header-tag"]));
    assert_eq!(trace["metadata"]["created_from"], "litellm");
    assert_eq!(span["trace_id"], trace["id"]);
    assert_eq!(span["provider"], "google_ai");
}
