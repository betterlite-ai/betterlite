use betterlite_error::error::class_message;

use crate::api::video_list_call::VideoListCall;
use crate::api::video_remix_call::error::VideoCallError;
use crate::api::video_remix_call::support::{Bench, LIST_BODY, run_list};

const REQUEST_TIMEOUT: f64 = 6000.0;

#[test]
fn unsupported_list_provider_names_the_operation() {
    let mut bench = Bench::new(200, LIST_BODY);
    let call = VideoListCall {
        custom_llm_provider: Some("anthropic".to_owned()),
        ..VideoListCall::default()
    };
    let failure =
        run_list(&mut bench, &call, REQUEST_TIMEOUT).expect_err("anthropic has no video config");
    assert_eq!(
        failure.error.to_string(),
        "video list is not supported for anthropic"
    );
    assert_eq!(failure.class.metadata().status_code, 500);
    assert!(bench.requests().is_empty());
}

#[test]
fn video_list_groq_maps_through_exception_type() {
    let mut bench = Bench::new(200, LIST_BODY);
    let call = VideoListCall {
        custom_llm_provider: Some("groq".to_owned()),
        ..VideoListCall::default()
    };
    let failure =
        run_list(&mut bench, &call, REQUEST_TIMEOUT).expect_err("groq has no video config");
    assert_eq!(
        class_message(&failure.class),
        "APIConnectionError: GroqException - video list is not supported for groq"
    );
}

#[test]
fn video_list_unknown_provider_is_reported() {
    let mut bench = Bench::new(200, LIST_BODY);
    let call = VideoListCall {
        custom_llm_provider: Some("not-a-provider".to_owned()),
        ..VideoListCall::default()
    };
    let failure =
        run_list(&mut bench, &call, REQUEST_TIMEOUT).expect_err("an unknown name is rejected");
    assert!(matches!(
        failure.error,
        VideoCallError::UnknownProvider { .. }
    ));
    assert!(bench.requests().is_empty());
}
