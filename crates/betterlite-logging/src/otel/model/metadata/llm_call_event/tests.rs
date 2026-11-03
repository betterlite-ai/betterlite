#![cfg(test)]
use crate::otel::model::metadata::callback::LLMCallEvent;
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
