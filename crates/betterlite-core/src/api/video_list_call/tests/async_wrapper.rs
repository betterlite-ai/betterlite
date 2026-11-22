#![cfg(test)]

use betterlite_models::identity::provider_id::ProviderId;

use crate::api::video_list_call::{VideoListCall, VideoListReply};
use crate::api::video_remix_call::support::{Bench, Case, LIST_BODY, Plan, Wire};
use crate::api::video_remix_list_transport::client::VideoClient;

const REQUEST_TIMEOUT: f64 = 6000.0;

#[tokio::test]
async fn avideo_list_uses_the_async_path() {
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
    assert!(bench.source.sync_calls().is_empty());
    assert_eq!(bench.source.async_calls(), vec![(ProviderId::Openai, None)]);
    assert_eq!(bench.requests().len(), 1);
}

#[tokio::test]
async fn avideo_list_uses_a_supplied_async_client() {
    let mut bench = Bench::new(200, LIST_BODY);
    let wire = Wire::replying(200, LIST_BODY);
    let call = VideoListCall {
        custom_llm_provider: Some("openai".to_owned()),
        ..VideoListCall::default()
    };
    let reply = bench
        .alist_answer(Plan::client(
            Case::List(&call),
            VideoClient::Asynchronous(&wire),
        ))
        .await
        .expect("the supplied client answers");
    assert!(matches!(reply, VideoListReply::Page(_)));
    assert_eq!(wire.requests().len(), 1);
    assert!(bench.requests().is_empty());
    assert!(bench.source.async_calls().is_empty());
}
