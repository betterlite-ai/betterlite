#![cfg(test)]

use super::support::{LagoBody, logger};
use crate::finops::metering::lago::error::LagoError;
use crate::finops::metering::support::{FakeSender, Unreachable, capture_debug, event};

#[tokio::test]
async fn async_log_success_event_sends_the_same_payload_and_auth_as_the_sync_path() {
    let sender = FakeSender::default();
    let logger = logger(sender.clone());
    logger.log_success_event(&event()).await.unwrap();
    logger.async_log_success_event(&event()).await.unwrap();
    let posts = sender.posts();
    assert_eq!(posts.len(), 2);
    assert_eq!(posts[0].url, posts[1].url);
    assert_eq!(posts[0].authorization, posts[1].authorization);
    assert_eq!(posts[0].content_type, posts[1].content_type);
    let sync_body: LagoBody = posts[0].parse();
    let async_body: LagoBody = posts[1].parse();
    assert_eq!(sync_body.event.code, async_body.event.code);
    assert_eq!(
        sync_body.event.external_subscription_id,
        async_body.event.external_subscription_id
    );
    assert_eq!(sync_body.event.properties, async_body.event.properties);
    assert_ne!(
        sync_body.event.transaction_id,
        async_body.event.transaction_id
    );
}

#[tokio::test]
async fn async_log_success_event_logs_the_accepted_event() {
    let (captured, _guard) = capture_debug();
    let sender = FakeSender::answering(200, "lago accepted the event");
    logger(sender)
        .async_log_success_event(&event())
        .await
        .unwrap();
    assert!(captured.text().contains("lago accepted the event"));
}

#[tokio::test]
async fn async_log_success_event_returns_the_http_error_after_logging_the_body() {
    let (captured, _guard) = capture_debug();
    let sender = FakeSender::answering(500, "lago is down");
    let error = logger(sender)
        .async_log_success_event(&event())
        .await
        .unwrap_err();
    assert!(matches!(error, LagoError::Status { status: 500 }));
    assert!(captured.text().contains("lago is down"));
}

#[tokio::test]
async fn async_log_success_event_propagates_a_transport_error() {
    let error = logger(FakeSender::failing())
        .async_log_success_event(&event())
        .await
        .unwrap_err();
    assert!(matches!(error, LagoError::Send(Unreachable)));
}
