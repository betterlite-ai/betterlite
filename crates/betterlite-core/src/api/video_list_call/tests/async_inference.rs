#![cfg(test)]

use betterlite_error::error::class_message;
use betterlite_models::identity::provider_id::ProviderId;

use crate::api::video_list_call::{VideoListCall, VideoListReply};
use crate::api::video_remix_call::error::VideoCallError;
use crate::api::video_remix_call::support::{Bench, Case, LIST_BODY, Plan, json};

const REQUEST_TIMEOUT: f64 = 6000.0;

#[tokio::test]
async fn avideo_list_without_a_provider_is_a_bad_request() {
    let mut bench = Bench::new(200, LIST_BODY);
    let failure = bench
        .alist_answer(Plan::seconds(
            Case::List(&VideoListCall::default()),
            REQUEST_TIMEOUT,
        ))
        .await
        .expect_err("an empty model names no provider");
    assert!(matches!(failure.error, VideoCallError::Resolution(_)));
    assert_eq!(failure.class.metadata().status_code, 400);
    assert_eq!(
        class_message(&failure.class),
        "Exception - LLM Provider NOT provided. Pass in the LLM provider you are trying to call. You passed model="
    );
    assert!(bench.requests().is_empty());
}

#[tokio::test]
async fn avideo_list_infers_before_the_mock_response() {
    let mut bench = Bench::new(200, LIST_BODY);
    let call = VideoListCall {
        mock_response: Some(json(
            r#"[{"id":"video_1","object":"video","status":"queued"}]"#,
        )),
        ..VideoListCall::default()
    };
    let failure = bench
        .alist_answer(Plan::seconds(Case::List(&call), REQUEST_TIMEOUT))
        .await
        .expect_err("the inference runs before the mock");
    assert!(matches!(failure.error, VideoCallError::Resolution(_)));
    assert!(bench.requests().is_empty());
}

#[tokio::test]
async fn avideo_list_with_a_provider_calls_the_provider() {
    let mut bench = Bench::new(200, LIST_BODY);
    let call = VideoListCall {
        custom_llm_provider: Some("openai".to_owned()),
        ..VideoListCall::default()
    };
    let reply = bench
        .alist_answer(Plan::seconds(Case::List(&call), REQUEST_TIMEOUT))
        .await
        .expect("the list answers");
    assert!(matches!(reply, VideoListReply::Page(_)));
    assert_eq!(bench.source.async_calls(), vec![(ProviderId::Openai, None)]);
    assert!(bench.source.sync_calls().is_empty());
    assert_eq!(bench.requests().len(), 1);
}

#[tokio::test]
async fn avideo_list_with_a_provider_skips_the_inference_for_a_mock() {
    let mut bench = Bench::new(200, LIST_BODY);
    let call = VideoListCall {
        custom_llm_provider: Some("openai".to_owned()),
        mock_response: Some(json(
            r#"[{"id":"video_1","object":"video","status":"queued"}]"#,
        )),
        ..VideoListCall::default()
    };
    let reply = bench
        .alist_answer(Plan::seconds(Case::List(&call), REQUEST_TIMEOUT))
        .await
        .expect("the mock answers");
    let VideoListReply::Objects(objects) = reply else {
        panic!("a mock list is objects");
    };
    assert_eq!(objects.len(), 1);
    assert!(bench.requests().is_empty());
}
