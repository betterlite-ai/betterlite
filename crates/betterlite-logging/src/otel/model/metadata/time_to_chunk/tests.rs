#![cfg(test)]
use crate::otel::model::metadata::callback::time_to_first_chunk_seconds;
use crate::otel::model::utils::coercion::OtelModelValue;

#[test]
fn latency_is_streaming_only() {
    let non_streaming: OtelModelValue =
        serde_json::from_str(r#"{"api_call_start_time":2.0,"completion_start_time":3.5}"#)
            .expect("test JSON is valid");
    let streaming: OtelModelValue = serde_json::from_str(r#"{"optional_params":{"stream":true},"api_call_start_time":2.0,"completion_start_time":3.5}"#).expect("test JSON is valid");
    assert_eq!(time_to_first_chunk_seconds(&non_streaming), None);
    assert_eq!(time_to_first_chunk_seconds(&streaming), Some(1.5_f64));
}
