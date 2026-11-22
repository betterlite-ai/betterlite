#![cfg(test)]

use betterlite_error::error::ErrorClass;

use super::support::{Bench, Case, Fixture, Plan, VIDEO_BODY, remix_call};
use crate::api::video_remix_call::error::{VideoCallError, failure};
use crate::api::video_remix_list_transport::error::VideoRemixListTransportError;

const REQUEST_TIMEOUT: f64 = 6000.0;

#[test]
fn unsupported_remix_provider_names_the_operation() {
    let mut bench = Bench::new(200, VIDEO_BODY);
    let mut call = remix_call();
    call.custom_llm_provider = Some("anthropic".to_owned());
    let failure = bench
        .video_answer(Plan::seconds(Case::Remix(&call), REQUEST_TIMEOUT))
        .expect_err("anthropic has no video config");
    assert_eq!(
        failure.error.to_string(),
        "video remix is not supported for anthropic"
    );
    let ErrorClass::BadRequest(details) = &failure.class else {
        panic!(
            "an unsupported provider answers 400, got {:?}",
            failure.class
        );
    };
    assert_eq!(failure.class.metadata().status_code, 400);
    assert_eq!(
        details.message,
        "video remix is not supported for anthropic"
    );
    assert_eq!(details.model.as_deref(), Some(""));
    assert_eq!(details.provider.as_deref(), Some("anthropic"));
    assert!(bench.requests().is_empty());
}

#[test]
fn unsupported_remix_provider_is_a_bad_request() {
    let mut bench = Bench::new(200, VIDEO_BODY);
    let mut call = remix_call();
    call.custom_llm_provider = Some("groq".to_owned());
    let failure = bench
        .video_answer(Plan::seconds(Case::Remix(&call), REQUEST_TIMEOUT))
        .expect_err("groq has no video config");
    let ErrorClass::BadRequest(details) = &failure.class else {
        panic!(
            "an unsupported provider answers 400, got {:?}",
            failure.class
        );
    };
    assert_eq!(details.message, "video remix is not supported for groq");
    assert_eq!(details.provider.as_deref(), Some("groq"));
}

// the transport's async client parses the provider again, and its own failure answers 400 for it
#[test]
fn transport_unknown_provider_answers_400() {
    let fixture = Fixture::new();
    let error = VideoCallError::Transport(VideoRemixListTransportError::UnknownProvider {
        provider: "nope".to_owned(),
    });
    let mapped = failure(error, "nope", fixture.env.redactor());
    let ErrorClass::BadRequest(details) = &mapped.class else {
        panic!(
            "an unknown transport provider answers 400, got {:?}",
            mapped.class
        );
    };
    assert_eq!(mapped.class.metadata().status_code, 400);
    assert_eq!(
        details.message,
        "\"nope\" is not a provider betterlite knows"
    );
    assert_eq!(details.model.as_deref(), Some(""));
    assert_eq!(details.provider.as_deref(), Some("nope"));
    assert_eq!(
        mapped.error.to_string(),
        "\"nope\" is not a provider betterlite knows"
    );
}

#[test]
fn edenai_selection_answers_400_with_its_message() {
    let mut bench = Bench::new(200, VIDEO_BODY);
    let mut call = remix_call();
    call.custom_llm_provider = Some("edenai".to_owned());
    let failure = bench
        .video_answer(Plan::seconds(Case::Remix(&call), REQUEST_TIMEOUT))
        .expect_err("edenai is unbuilt");
    assert!(matches!(failure.error, VideoCallError::Selection(_)));
    assert_eq!(
        failure.error.to_string(),
        "no video config is installed for 'edenai'"
    );
    let ErrorClass::BadRequest(details) = &failure.class else {
        panic!("a selection failure answers 400, got {:?}", failure.class);
    };
    assert_eq!(failure.class.metadata().status_code, 400);
    assert_eq!(details.message, "no video config is installed for 'edenai'");
    assert_eq!(details.provider.as_deref(), Some("edenai"));
    assert!(bench.requests().is_empty());
}
