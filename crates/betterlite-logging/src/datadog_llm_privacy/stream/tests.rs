#![cfg(test)]

use crate::datadog_llm_privacy::tests::support;

use betterlite_models::decisions::Json;
use serde_json::Number;

use crate::datadog_llm_privacy::stream::is_streamed_request;

#[test]
fn stream_detection_reads_the_top_level_flag_first() {
    let mut payload = support::payload();
    payload.stream = Some(true);
    payload
        .model_parameters
        .insert("stream".to_owned(), Json::Bool(false));
    assert!(is_streamed_request(&payload));
}

#[test]
fn stream_detection_falls_back_to_the_request_parameters() {
    let mut payload = support::payload();
    payload.stream = None;
    payload
        .model_parameters
        .insert("stream".to_owned(), Json::Bool(true));
    assert!(is_streamed_request(&payload));
}

#[test]
fn stream_detection_needs_the_boolean_true() {
    let mut payload = support::payload();
    payload.stream = Some(false);
    payload
        .model_parameters
        .insert("stream".to_owned(), Json::Text("true".to_owned()));
    assert!(!is_streamed_request(&payload));
    payload
        .model_parameters
        .insert("stream".to_owned(), Json::Number(Number::from(1_u8)));
    assert!(!is_streamed_request(&payload));
}

#[test]
fn a_request_without_stream_fields_is_not_streamed() {
    let mut payload = support::payload();
    payload.stream = None;
    assert!(!is_streamed_request(&payload));
}
