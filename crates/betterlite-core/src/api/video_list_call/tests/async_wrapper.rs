use betterlite_models::identity::provider_id::ProviderId;

use crate::api::video_list_call::{VideoListCall, VideoListReply};
use crate::api::video_remix_call::support::{
    Bench, LIST_BODY, Resolver, Wire, run_avideo_list, run_avideo_list_with_client,
};
use crate::api::video_remix_list_transport::client::VideoClient;

const REQUEST_TIMEOUT: f64 = 6000.0;

#[tokio::test]
async fn avideo_list_uses_the_async_path() {
    let mut bench = Bench::new(200, LIST_BODY);
    let resolver = Resolver::new();
    let call = VideoListCall {
        custom_llm_provider: Some("openai".to_owned()),
        ..VideoListCall::default()
    };
    let reply = run_avideo_list(&mut bench, &resolver, &call, REQUEST_TIMEOUT)
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
    let resolver = Resolver::new();
    let wire = Wire::replying(200, LIST_BODY);
    let call = VideoListCall {
        custom_llm_provider: Some("openai".to_owned()),
        ..VideoListCall::default()
    };
    let reply = run_avideo_list_with_client(
        &mut bench,
        &resolver,
        &call,
        VideoClient::Asynchronous(&wire),
    )
    .await
    .expect("the supplied client answers");
    assert!(matches!(reply, VideoListReply::Page(_)));
    assert_eq!(wire.requests().len(), 1);
    assert!(bench.requests().is_empty());
    assert!(bench.source.async_calls().is_empty());
}
