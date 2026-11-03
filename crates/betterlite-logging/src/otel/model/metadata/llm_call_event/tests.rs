#![cfg(test)]
use crate::otel::model::metadata::llm_call_event::LLMCallEvent;
use crate::otel::model::utils::coercion::OtelModelValue;

#[test]
fn parses_live_callback_fields() {
    let kwargs: OtelModelValue = serde_json::from_str(r#"{"litellm_call_id":"call","model":"model","call_type":"completion","api_call_start_time":10.0,"completion_start_time":10.5,"optional_params":{"stream":true},"standard_callback_dynamic_params":{"endpoint":"x"}}"#).expect("test JSON is valid");
    let event = LLMCallEvent::from_dict(&kwargs);
    assert_eq!(event.call_id.as_deref(), Some("call"));
    assert_eq!(event.provisional_span_name, "chat model");
    assert_eq!(event.time_to_first_chunk_seconds, Some(0.5_f64));
    assert!(event.upstream_started);
    assert!(event.dynamic_params.is_some());
}

#[test]
fn closed_payload_wins_and_rejected_calls_have_no_upstream() {
    let kwargs: OtelModelValue = serde_json::from_str(r#"{"litellm_call_id":"live","standard_logging_object":{"id":"closed","metadata":{"user_api_key_auth_metadata":{"tenant":"trusted"}}},"litellm_no_upstream_llm_call":true,"litellm_params":{"metadata":{"session_id":"conversation","trace_name":"caller"}}}"#).expect("test JSON is valid");
    let event = LLMCallEvent::from_dict(&kwargs);
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
