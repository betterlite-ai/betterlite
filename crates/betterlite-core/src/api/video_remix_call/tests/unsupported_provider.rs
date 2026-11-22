#![cfg(test)]

use betterlite_error::error::class_message;

use super::support::{Bench, Case, Plan, VIDEO_BODY};
use crate::api::video_remix_call::VideoRemixCall;
use crate::api::video_remix_call::error::VideoCallError;

const REQUEST_TIMEOUT: f64 = 6000.0;

#[test]
fn unsupported_remix_provider_names_the_operation() {
    let mut bench = Bench::new(200, VIDEO_BODY);
    let call = VideoRemixCall {
        custom_llm_provider: Some("anthropic".to_owned()),
        ..VideoRemixCall::default()
    };
    let failure = bench
        .video_answer(Plan::seconds(Case::Remix(&call), REQUEST_TIMEOUT))
        .expect_err("anthropic has no video config");
    assert_eq!(
        failure.error.to_string(),
        "video remix is not supported for anthropic"
    );
    assert_eq!(failure.class.metadata().status_code, 500);
    assert!(bench.requests().is_empty());
}

#[test]
fn unsupported_remix_provider_maps_through_exception_type() {
    let mut bench = Bench::new(200, VIDEO_BODY);
    let call = VideoRemixCall {
        custom_llm_provider: Some("groq".to_owned()),
        ..VideoRemixCall::default()
    };
    let failure = bench
        .video_answer(Plan::seconds(Case::Remix(&call), REQUEST_TIMEOUT))
        .expect_err("groq has no video config");
    assert_eq!(
        class_message(&failure.class),
        "APIConnectionError: GroqException - video remix is not supported for groq"
    );
}

#[test]
fn edenai_selection_keeps_the_selection_message() {
    let mut bench = Bench::new(200, VIDEO_BODY);
    let call = VideoRemixCall {
        custom_llm_provider: Some("edenai".to_owned()),
        ..VideoRemixCall::default()
    };
    let failure = bench
        .video_answer(Plan::seconds(Case::Remix(&call), REQUEST_TIMEOUT))
        .expect_err("edenai is unbuilt");
    assert!(matches!(failure.error, VideoCallError::Selection(_)));
    assert_eq!(
        failure.error.to_string(),
        "no video config is installed for 'edenai'"
    );
    assert_eq!(failure.class.metadata().status_code, 500);
    assert!(bench.requests().is_empty());
}
