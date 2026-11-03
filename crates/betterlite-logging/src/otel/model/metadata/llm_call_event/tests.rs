#![cfg(test)]
use crate::otel::model::metadata::llm_call_event::LLMCallEvent;
use crate::otel::model::metadata::wire::{callback::Callback, decode};
use rust_decimal::Decimal;

#[test]
fn parses_live_callback_fields() {
    let kwargs: Callback = serde_json::from_str(r#"{"litellm_call_id":"call","model":"model","call_type":"completion","api_call_start_time":10.0,"completion_start_time":10.5,"optional_params":{"stream":true},"standard_callback_dynamic_params":{"endpoint":"x"}}"#).expect("test JSON is valid");
    let event = LLMCallEvent::from_dict(&kwargs).unwrap();
    assert_eq!(event.call_id.as_deref(), Some("call"));
    assert_eq!(event.provisional_span_name, "chat model");
    assert_eq!(event.time_to_first_chunk_seconds, Some(Decimal::new(5, 1)));
    assert!(event.upstream_started);
    assert!(event.dynamic_params.is_some());
}

#[test]
fn closed_payload_wins_and_rejected_calls_have_no_upstream() {
    let kwargs: Callback = serde_json::from_str(r#"{"litellm_call_id":"live","standard_logging_object":{"id":"closed","metadata":{"user_api_key_auth_metadata":{"tenant":"trusted"}}},"litellm_no_upstream_llm_call":true,"litellm_params":{"metadata":{"session_id":"conversation","trace_name":"caller"}}}"#).expect("test JSON is valid");
    let event = LLMCallEvent::from_dict(&kwargs).unwrap();
    assert_eq!(event.call_id.as_deref(), Some("closed"));
    assert!(event.payload.is_some());
    assert!(event.is_no_upstream_call);
    assert!(!event.upstream_started);
    assert_eq!(event.time_to_first_chunk_seconds, None);
    assert_eq!(event.session_id.as_deref(), Some("conversation"));
    assert_eq!(event.trace.name.as_deref(), Some("caller"));
    assert_eq!(
        event
            .auth_metadata
            .unwrap()
            .get("tenant")
            .map(String::as_str),
        Some("trusted")
    );
}

#[rstest::rstest]
#[case::stream(r#"{"optional_params":{"stream":"yes"}}"#, "optional_params.stream")]
#[case::payload(
    r#"{"standard_logging_object":{"model":false}}"#,
    "standard_logging_object.model"
)]
#[case::timestamp(r#"{"api_call_start_time":"invalid"}"#, "api_call_start_time")]
#[case::session(
    r#"{"litellm_params":{"metadata":{"session_id":42}}}"#,
    "litellm_params.metadata.session_id"
)]
fn rejects_malformed_stream(#[case] source: &str, #[case] field: &str) {
    assert!(
        decode::<Callback>(source)
            .unwrap_err()
            .to_string()
            .contains(field)
    );
}

#[test]
fn null_callback_fields_take_defaults() {
    let kwargs: Callback = decode(r#"{"litellm_params":null,"optional_params":{"stream":null},"litellm_no_upstream_llm_call":null}"#).unwrap();
    let event = LLMCallEvent::from_dict(&kwargs).unwrap();
    assert!(!event.is_no_upstream_call);
    assert!(!event.upstream_started);
    assert_eq!(event.time_to_first_chunk_seconds, None);
}

#[test]
fn serialized_latency_is_an_exact_json_number() {
    let kwargs: Callback = decode(r#"{"optional_params":{"stream":true},"api_call_start_time":0,"completion_start_time":0.123456789012345678}"#).unwrap();
    let event = LLMCallEvent::from_dict(&kwargs).unwrap();
    let serialized = serde_json::to_string(&event).unwrap();
    let decoded: Latency = serde_json::from_str(&serialized).unwrap();
    assert_eq!(
        decoded.time_to_first_chunk_seconds.to_string(),
        "0.123456789012345678"
    );
}

#[derive(serde::Deserialize)]
struct Latency {
    time_to_first_chunk_seconds: serde_json::Number,
}
