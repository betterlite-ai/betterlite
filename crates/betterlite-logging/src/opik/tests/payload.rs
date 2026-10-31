#![cfg(test)]

use betterlite_logging::opik::payload::{
    PayloadContent, PayloadWindow, SpanContext, SpanDetails, TraceContext, build_opik_payload,
    build_span_payload, build_trace_payload,
};
use betterlite_models::decisions::Json;
use chrono::{TimeZone as _, Utc};
use indexmap::IndexMap;

fn obj(text: &str) -> IndexMap<String, Json> {
    serde_json::from_str(text).expect("valid fixture JSON")
}

fn val(text: &str) -> Json {
    serde_json::from_str(text).expect("valid fixture JSON")
}

fn empty_content() -> PayloadContent {
    PayloadContent {
        input: val("{}"),
        output: val("{}"),
        metadata: IndexMap::new(),
        tags: vec![],
    }
}

fn sample_response() -> IndexMap<String, Json> {
    obj(
        r#"{"model":"m","object":"chat.completion","created":1,"usage":{"prompt_tokens":1,"completion_tokens":null}}"#,
    )
}

fn sample_window() -> PayloadWindow {
    PayloadWindow {
        start: Utc.timestamp_millis_opt(0).unwrap(),
        end: Utc.timestamp_millis_opt(1).unwrap(),
    }
}

#[test]
fn trace_payload_links_id_and_strips_nulls() {
    let response = sample_response();
    let scope = TraceContext {
        project_name: "p",
        trace_id: "trace",
        response: &response,
    };
    let trace = build_trace_payload(&scope, sample_window(), empty_content(), None);
    assert_eq!(trace["id"], val(r#""trace""#));
    assert!(!trace.contains_key("thread_id"));
}

#[test]
fn span_payload_links_trace_and_strips_nulls() {
    let response = sample_response();
    let scope = SpanContext {
        project_name: "p",
        trace_id: "trace",
        parent_span_id: None,
        response: &response,
    };
    let details = SpanDetails {
        cost: None,
        provider: Some("google_ai"),
    };
    let span = build_span_payload(scope, sample_window(), empty_content(), details);
    assert_eq!(span["trace_id"], val(r#""trace""#));
    assert!(!span.contains_key("parent_span_id"));
    assert_eq!(span["usage"], val(r#"{"prompt_tokens":1}"#));
}

fn sample_kwargs() -> IndexMap<String, Json> {
    obj(r#"{
            "custom_llm_provider": "gemini",
            "response_cost": 0.25,
            "litellm_params": {
                "metadata": {"opik": {"project_name": "request", "tags": ["request"]}},
                "proxy_server_request": {
                    "headers": {"opik_project_name": "header", "opik_tags": "[\"header-tag\"]"}
                }
            },
            "standard_logging_object": {
                "metadata": {
                    "user_api_key_auth_metadata": {"opik": {"project_name": "auth"}},
                    "requester_metadata": {"opik": {"thread_id": "thread"}}
                },
                "messages": [{"role": "user", "content": "hi"}],
                "response": {"answer": "ok"},
                "model": "m",
                "type": "completion",
                "status": "success"
            }
        }"#)
}

#[test]
fn orchestration_creates_trace_and_applies_metadata() {
    let kwargs = sample_kwargs();
    let response = obj(
        r#"{"model": "m", "object": "chat.completion", "created": 1, "usage": {"total_tokens": 2}}"#,
    );
    let (trace, span) = build_opik_payload(
        &kwargs,
        &response,
        Utc.timestamp_millis_opt(0).unwrap(),
        Utc.timestamp_millis_opt(1).unwrap(),
        "default",
    );
    let built_trace = trace.expect("new requests create a trace");
    assert_eq!(built_trace["project_name"], val(r#""header""#));
    assert_eq!(built_trace["thread_id"], val(r#""thread""#));
    assert_eq!(
        built_trace["tags"],
        val(r#"["request", "gemini", "header-tag"]"#)
    );
    let Json::Object(metadata) = &built_trace["metadata"] else {
        panic!("metadata must be an object")
    };
    assert_eq!(metadata["created_from"], val(r#""litellm""#));
    assert_eq!(span["trace_id"], built_trace["id"]);
    assert_eq!(span["provider"], val(r#""google_ai""#));
}
