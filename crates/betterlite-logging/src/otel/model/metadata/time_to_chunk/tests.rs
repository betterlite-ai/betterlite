#![cfg(test)]
use crate::otel::model::metadata::time_to_chunk::time_to_first_chunk_seconds;
use crate::otel::model::metadata::wire::callback::Callback;
use rust_decimal::Decimal;

#[test]
fn latency_is_streaming_only() {
    let non_streaming: Callback =
        serde_json::from_str(r#"{"api_call_start_time":2.0,"completion_start_time":3.5}"#)
            .expect("test JSON is valid");
    let streaming: Callback = serde_json::from_str(r#"{"optional_params":{"stream":true},"api_call_start_time":2.0,"completion_start_time":3.5}"#).expect("test JSON is valid");
    assert_eq!(time_to_first_chunk_seconds(&non_streaming), None);
    assert_eq!(
        time_to_first_chunk_seconds(&streaming),
        Some(Decimal::new(15, 1))
    );
}

#[rstest::rstest]
#[case(r#"{"optional_params":{"stream":true},"api_call_start_time":1}"#)]
#[case(r#"{"optional_params":{"stream":true},"completion_start_time":2}"#)]
#[case(
    r#"{"optional_params":{"stream":true},"api_call_start_time":null,"completion_start_time":2}"#
)]
fn streaming_latency_requires_both_timestamps(#[case] source: &str) {
    let kwargs: Callback = serde_json::from_str(source).unwrap();
    assert_eq!(time_to_first_chunk_seconds(&kwargs), None);
}

#[test]
fn streaming_latency_retains_decimal_precision() {
    let kwargs: Callback = serde_json::from_str(r#"{"optional_params":{"stream":true},"api_call_start_time":0,"completion_start_time":0.123456789012345678}"#).unwrap();
    assert_eq!(
        time_to_first_chunk_seconds(&kwargs)
            .map(|value| value.to_string())
            .as_deref(),
        Some("0.123456789012345678")
    );
}

#[test]
fn formatted_timestamps_decode_to_decimal_seconds() {
    let kwargs: Callback = serde_json::from_str(r#"{"optional_params":{"stream":true},"api_call_start_time":"1970-01-01 00:00:00","completion_start_time":"1970-01-01 00:00:00.125"}"#).unwrap();
    assert_eq!(
        time_to_first_chunk_seconds(&kwargs),
        Some(Decimal::new(125, 3))
    );
}
