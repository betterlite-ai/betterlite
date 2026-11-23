#![cfg(test)]

use betterlite_error::error::ErrorClass;

use crate::api::video_remix_call::error::VideoCallError;
use crate::api::video_remix_call::support::{Bench, Case, LIST_BODY, Plan, list_call};

const REQUEST_TIMEOUT: f64 = 6000.0;

#[test]
fn unsupported_list_provider_names_the_operation() {
    let mut bench = Bench::new(200, LIST_BODY);
    let mut call = list_call();
    call.custom_llm_provider = Some("anthropic".to_owned());
    let failure = bench
        .list_answer(Plan::seconds(Case::List(&call), REQUEST_TIMEOUT))
        .expect_err("anthropic has no video config");
    assert_eq!(
        failure.error.to_string(),
        "video list is not supported for anthropic"
    );
    let ErrorClass::BadRequest(details) = &failure.class else {
        panic!(
            "an unsupported provider answers 400, got {:?}",
            failure.class
        );
    };
    assert_eq!(failure.class.metadata().status_code, 400);
    assert_eq!(details.message, "video list is not supported for anthropic");
    assert_eq!(details.model.as_deref(), Some(""));
    assert_eq!(details.provider.as_deref(), Some("anthropic"));
    assert!(bench.requests().is_empty());
}

#[test]
fn video_list_groq_is_a_bad_request() {
    let mut bench = Bench::new(200, LIST_BODY);
    let mut call = list_call();
    call.custom_llm_provider = Some("groq".to_owned());
    let failure = bench
        .list_answer(Plan::seconds(Case::List(&call), REQUEST_TIMEOUT))
        .expect_err("groq has no video config");
    let ErrorClass::BadRequest(details) = &failure.class else {
        panic!(
            "an unsupported provider answers 400, got {:?}",
            failure.class
        );
    };
    assert_eq!(details.message, "video list is not supported for groq");
    assert_eq!(details.provider.as_deref(), Some("groq"));
}

#[test]
fn video_list_unknown_provider_is_reported() {
    let mut bench = Bench::new(200, LIST_BODY);
    let mut call = list_call();
    call.custom_llm_provider = Some("not-a-provider".to_owned());
    let failure = bench
        .list_answer(Plan::seconds(Case::List(&call), REQUEST_TIMEOUT))
        .expect_err("an unknown name is rejected");
    assert!(matches!(
        failure.error,
        VideoCallError::UnknownProvider { .. }
    ));
    assert_eq!(failure.class.metadata().status_code, 400);
    assert!(bench.requests().is_empty());
}
