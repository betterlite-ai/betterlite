#![cfg(test)]

use super::support::{CloudEventBody, logger};
use crate::finops::metering::openmeter::error::OpenMeterError;
use crate::finops::metering::support::{FakeSender, Unreachable, event};

#[tokio::test]
async fn async_log_success_event_sends_the_same_payload_and_auth_as_the_sync_path() {
    let sender = FakeSender::default();
    let logger = logger(sender.clone());
    logger.log_success_event(&event()).await.unwrap();
    logger.async_log_success_event(&event()).await.unwrap();
    let posts = sender.posts();
    assert_eq!(posts.len(), 2);
    assert_eq!(posts[0].url, posts[1].url);
    assert_eq!(posts[0].api_key, posts[1].api_key);
    assert_eq!(posts[0].content_type, posts[1].content_type);
    let sync_body: CloudEventBody = posts[0].parse();
    let async_body: CloudEventBody = posts[1].parse();
    assert_eq!(sync_body, async_body);
}

#[tokio::test]
async fn async_log_success_event_wraps_an_http_error_with_the_response_body() {
    let sender = FakeSender::answering(503, "sink unavailable");
    let error = logger(sender)
        .async_log_success_event(&event())
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "OpenMeter logging error: sink unavailable"
    );
    assert!(matches!(error, OpenMeterError::Status { status: 503, .. }));
}

#[tokio::test]
async fn async_log_success_event_propagates_a_transport_error() {
    let error = logger(FakeSender::failing())
        .async_log_success_event(&event())
        .await
        .unwrap_err();
    assert!(matches!(error, OpenMeterError::Send(Unreachable)));
}
