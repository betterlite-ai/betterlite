#![cfg(test)]

use betterlite_models::decisions::Json;

use crate::api::video_contracts::config::BaseVideoConfig as _;
use crate::api::video_contracts::records::VideoLogging;

use super::fixture::{FixtureError, Subject, raw};

#[tokio::test]
async fn async_content_returns_the_sync_bytes_and_forwards_logging() {
    let mut logging = VideoLogging::default();
    let content = Subject
        .async_transform_video_content_response(&raw(200, "video bytes"), &mut logging)
        .await
        .unwrap();
    assert_eq!(content, b"video bytes");
    assert_eq!(
        logging.model_call_details.get("content_seen"),
        Some(&Json::Bool(true))
    );
}

#[tokio::test]
async fn async_content_keeps_the_sync_error() {
    let mut logging = VideoLogging::default();
    let error = Subject
        .async_transform_video_content_response(&raw(500, "down"), &mut logging)
        .await
        .unwrap_err();
    assert!(matches!(error, FixtureError::Refused(500)));
    assert!(logging.model_call_details.is_empty());
}
