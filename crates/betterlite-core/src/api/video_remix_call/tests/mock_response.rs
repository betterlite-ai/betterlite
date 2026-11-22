#![cfg(test)]

use betterlite_models::decisions::Json;

use super::support::{Bench, Case, Plan, VIDEO_BODY, json, remix_call};
use crate::api::video_remix_call::error::VideoCallError;

const REQUEST_TIMEOUT: f64 = 6000.0;

#[test]
fn mock_response_json_string_returns_the_video() {
    let mut bench = Bench::new(200, VIDEO_BODY);
    let mut call = remix_call();
    call.mock_response = Some(json(
        r#"{"id":"video_9","object":"video","status":"queued"}"#,
    ));
    let video = bench
        .video_answer(Plan::seconds(Case::Remix(&call), REQUEST_TIMEOUT))
        .expect("the mock answers");
    assert_eq!(video.id, "video_9");
    assert_eq!(video.status, "queued");
    assert_eq!(bench.fixture.state.call_type, "");
    assert!(bench.requests().is_empty());
}

#[test]
fn mock_response_object_returns_the_video() {
    let mut bench = Bench::new(200, VIDEO_BODY);
    let mut call = remix_call();
    call.mock_response = Some(json(
        r#"{"id":"video_8","object":"video","status":"completed","seconds":"2"}"#,
    ));
    let video = bench
        .video_answer(Plan::seconds(Case::Remix(&call), REQUEST_TIMEOUT))
        .expect("the mock answers");
    assert_eq!(video.id, "video_8");
    assert_eq!(video.seconds.as_deref(), Some("2"));
    assert!(bench.requests().is_empty());
}

#[test]
fn mock_response_invalid_json_is_a_value_error() {
    let mut bench = Bench::new(200, VIDEO_BODY);
    let mut call = remix_call();
    call.mock_response = Some(Json::Text("not json".to_owned()));
    let failure = bench
        .video_answer(Plan::seconds(Case::Remix(&call), REQUEST_TIMEOUT))
        .expect_err("the mock text is JSON");
    assert!(matches!(failure.error, VideoCallError::Mock(_)));
    assert_eq!(failure.class.metadata().status_code, 500);
    assert!(bench.requests().is_empty());
}

#[test]
fn mock_response_null_is_no_mock_response() {
    let mut bench = Bench::new(200, VIDEO_BODY);
    let mut call = remix_call();
    call.mock_response = Some(Json::Null);
    let video = bench
        .video_answer(Plan::seconds(Case::Remix(&call), REQUEST_TIMEOUT))
        .expect("the provider answers");
    assert_eq!(video.id, "video_123");
    assert_eq!(bench.requests().len(), 1);
}

#[test]
fn mock_response_wins_over_an_unsupported_provider() {
    let mut bench = Bench::new(200, VIDEO_BODY);
    let mut call = remix_call();
    call.custom_llm_provider = Some("groq".to_owned());
    call.mock_response = Some(json(
        r#"{"id":"video_7","object":"video","status":"queued"}"#,
    ));
    let video = bench
        .video_answer(Plan::seconds(Case::Remix(&call), REQUEST_TIMEOUT))
        .expect("the mock answers first");
    assert_eq!(video.id, "video_7");
    assert!(bench.requests().is_empty());
}
