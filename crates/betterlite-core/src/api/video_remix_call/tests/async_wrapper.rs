#![cfg(test)]

use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::openai_video_identifiers::ids::encode_video_id_with_provider;

use super::support::{Bench, Case, Plan, VIDEO_BODY, Wire, remix_call};
use crate::api::video_remix_list_transport::client::VideoClient;

const REQUEST_TIMEOUT: f64 = 6000.0;

#[tokio::test]
async fn avideo_remix_uses_the_async_path() {
    let mut bench = Bench::new(200, VIDEO_BODY);
    let call = remix_call();
    let video = bench
        .avideo_answer(Plan::seconds(Case::Remix(&call), REQUEST_TIMEOUT))
        .await
        .expect("the remix answers");
    assert_eq!(
        video.id,
        encode_video_id_with_provider("video_123", "openai", None)
    );
    assert!(bench.source.sync_calls().is_empty());
    assert_eq!(bench.source.async_calls(), vec![(ProviderId::Openai, None)]);
    assert_eq!(bench.requests().len(), 1);
}

#[tokio::test]
async fn avideo_remix_uses_a_supplied_async_client() {
    let mut bench = Bench::new(200, VIDEO_BODY);
    let wire = Wire::replying(200, VIDEO_BODY);
    let call = remix_call();
    let video = bench
        .avideo_answer(Plan::client(
            Case::Remix(&call),
            VideoClient::Asynchronous(&wire),
        ))
        .await
        .expect("the supplied client answers");
    assert_eq!(
        video.id,
        encode_video_id_with_provider("video_123", "openai", None)
    );
    assert_eq!(wire.requests().len(), 1);
    assert!(bench.requests().is_empty());
    assert!(bench.source.async_calls().is_empty());
}

#[test]
fn remix_uses_a_supplied_sync_client() {
    let mut bench = Bench::new(200, VIDEO_BODY);
    let wire = Wire::replying(200, VIDEO_BODY);
    let call = remix_call();
    let video = bench
        .video_answer(Plan::client(Case::Remix(&call), VideoClient::Sync(&wire)))
        .expect("the supplied client answers");
    assert_eq!(
        video.id,
        encode_video_id_with_provider("video_123", "openai", None)
    );
    assert_eq!(wire.requests().len(), 1);
    assert!(bench.requests().is_empty());
    assert!(bench.source.sync_calls().is_empty());
}
