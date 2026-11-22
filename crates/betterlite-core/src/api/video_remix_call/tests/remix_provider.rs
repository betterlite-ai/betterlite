#![cfg(test)]

use betterlite_error::error::class_message;

use super::support::{Bench, Case, Plan, VIDEO_BODY, encoded_id, remix_call};
use crate::api::video_remix_call::error::VideoCallError;

const REQUEST_TIMEOUT: f64 = 6000.0;

#[test]
fn remix_provider_decodes_from_the_video_id() {
    let mut bench = Bench::new(200, VIDEO_BODY);
    let mut call = remix_call();
    call.video_id = encoded_id("video_1", "openai");
    bench
        .video_answer(Plan::seconds(Case::Remix(&call), REQUEST_TIMEOUT))
        .expect("the openai config answers");
    assert_eq!(bench.requests().len(), 1);
}

#[test]
fn remix_provider_decoded_groq_is_unsupported() {
    let mut bench = Bench::new(200, VIDEO_BODY);
    let mut call = remix_call();
    call.video_id = encoded_id("video_1", "groq");
    let failure = bench
        .video_answer(Plan::seconds(Case::Remix(&call), REQUEST_TIMEOUT))
        .expect_err("groq has no video config");
    let VideoCallError::Unsupported { provider, .. } = &failure.error else {
        panic!("the decoded provider is the unsupported one");
    };
    assert_eq!(provider, "groq");
    assert_eq!(
        class_message(&failure.class),
        "APIConnectionError: GroqException - video remix is not supported for groq"
    );
    assert!(bench.requests().is_empty());
}

#[test]
fn remix_provider_explicit_overrides_the_decoded_one() {
    let mut bench = Bench::new(200, VIDEO_BODY);
    let mut call = remix_call();
    call.video_id = encoded_id("video_1", "groq");
    call.custom_llm_provider = Some("openai".to_owned());
    bench
        .video_answer(Plan::seconds(Case::Remix(&call), REQUEST_TIMEOUT))
        .expect("the explicit provider is the one used");
    assert_eq!(bench.requests().len(), 1);
}

#[test]
fn remix_provider_undecodable_id_falls_back_to_openai() {
    let mut bench = Bench::new(200, VIDEO_BODY);
    let mut call = remix_call();
    call.video_id = "video_1".to_owned();
    bench
        .video_answer(Plan::seconds(Case::Remix(&call), REQUEST_TIMEOUT))
        .expect("an undecorated id defaults to openai");
    assert_eq!(bench.requests().len(), 1);
}

#[test]
fn remix_provider_explicit_empty_is_unknown() {
    let mut bench = Bench::new(200, VIDEO_BODY);
    let mut call = remix_call();
    call.custom_llm_provider = Some(String::new());
    let failure = bench
        .video_answer(Plan::seconds(Case::Remix(&call), REQUEST_TIMEOUT))
        .expect_err("an empty name parses");
    assert!(matches!(
        failure.error,
        VideoCallError::UnknownProvider { .. }
    ));
    assert_eq!(failure.class.metadata().status_code, 500);
    assert!(bench.requests().is_empty());
}

#[test]
fn remix_provider_unknown_name_is_unknown() {
    let mut bench = Bench::new(200, VIDEO_BODY);
    let mut call = remix_call();
    call.custom_llm_provider = Some("not-a-provider".to_owned());
    let failure = bench
        .video_answer(Plan::seconds(Case::Remix(&call), REQUEST_TIMEOUT))
        .expect_err("an unknown name is rejected");
    let VideoCallError::UnknownProvider { provider } = &failure.error else {
        panic!("the provider name is rejected before any config is built");
    };
    assert_eq!(provider, "not-a-provider");
    assert!(bench.requests().is_empty());
}
