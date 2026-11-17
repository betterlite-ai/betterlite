#![cfg(test)]

use betterlite_models::decisions::Json;
use rstest::rstest;

use crate::api::video_contracts::config::BaseVideoConfig as _;
use crate::api::video_contracts::records::{VideoLogging, VideoReply};

use super::fixture::{AsyncHandle, FixtureError, Subject, raw};

#[rstest]
#[case::with_provider(Some("vertex_ai"), "vertex_ai")]
#[case::without_provider(None, "none")]
#[tokio::test]
async fn async_status_passes_the_provider_and_drops_the_client(
    #[case] provider: Option<&str>,
    #[case] expected_status: &str,
) {
    let response = raw(200, "video_1");
    let mut logging = VideoLogging::default();
    let reply = VideoReply {
        raw_response: &response,
        logging_obj: &mut logging,
        custom_llm_provider: provider,
    };
    let video = Subject
        .async_transform_video_status_retrieve_response(reply, Some(&AsyncHandle))
        .await
        .unwrap();
    assert_eq!(video.id, "video_1");
    assert_eq!(video.status, expected_status);
    assert_eq!(video.progress, Some(0));
    assert_eq!(
        logging.model_call_details.get("status_seen"),
        Some(&Json::Bool(true))
    );
}

#[tokio::test]
async fn async_status_keeps_the_sync_error() {
    let response = raw(502, "bad gateway");
    let mut logging = VideoLogging::default();
    let reply = VideoReply {
        raw_response: &response,
        logging_obj: &mut logging,
        custom_llm_provider: None,
    };
    let error = Subject
        .async_transform_video_status_retrieve_response(reply, None)
        .await
        .unwrap_err();
    assert!(matches!(error, FixtureError::Refused(502)));
}
