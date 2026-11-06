#![cfg(test)]

use rstest::rstest;
use rust_decimal::Decimal;
use uuid::Uuid;

use super::support::{API_KEY, EVENT_CODE, LagoBody, http_logger, logger, logger_with};
use crate::finops::metering::lago::error::LagoError;
use crate::finops::metering::support::{
    FakeSender, MODEL, Unreachable, assert_metering_request, capture_debug, event, serve_once,
};

#[tokio::test]
async fn post_sends_to_the_base_with_the_events_path_and_the_configured_key() {
    let sender = FakeSender::default();
    logger(sender.clone())
        .log_success_event(&event())
        .await
        .unwrap();
    let posted = sender.only_post();
    assert_eq!(posted.url, "https://lago.test/api/v1/events");
    assert_eq!(posted.content_type, "application/json");
    assert_eq!(posted.api_key, API_KEY);
}

#[tokio::test]
async fn post_sends_the_bearer_header_and_json_body_through_the_http_sender() {
    let (base, served) = serve_once(200, "{}").await;
    let logger = http_logger(&base);
    let call = event();
    let (result, received) = tokio::join!(logger.log_success_event(&call), served);
    result.unwrap();
    let body: LagoBody = serde_json::from_slice(assert_metering_request(
        &received,
        "application/json",
        API_KEY,
    ))
    .unwrap();
    assert_eq!(body.event.code, EVENT_CODE);
    assert_eq!(body.event.external_subscription_id, "end-user-1");
}

#[tokio::test]
async fn post_returns_a_non_2xx_status_through_the_http_sender() {
    let (base, served) = serve_once(422, "unknown subscription").await;
    let logger = http_logger(&base);
    let call = event();
    let (result, received) = tokio::join!(logger.log_success_event(&call), served);
    let error = result.unwrap_err();
    assert!(matches!(error, LagoError::Status { status: 422 }));
    assert_eq!(received.path, "/api/v1/events");
}

#[rstest]
#[case::no_trailing_slash("https://lago.test", "https://lago.test/api/v1/events")]
#[case::one_trailing_slash("https://lago.test/", "https://lago.test/api/v1/events")]
#[case::with_a_path("https://lago.test/billing", "https://lago.test/billing/api/v1/events")]
#[tokio::test]
async fn post_appends_the_events_path_once(
    #[case] base: &'static str,
    #[case] expected: &'static str,
) {
    let sender = FakeSender::default();
    logger_with(sender.clone(), &[("LAGO_API_BASE", base)])
        .log_success_event(&event())
        .await
        .unwrap();
    assert_eq!(sender.only_post().url, expected);
}

#[tokio::test]
async fn post_sends_the_lago_event_json() {
    let sender = FakeSender::default();
    logger(sender.clone())
        .log_success_event(&event())
        .await
        .unwrap();
    let body: LagoBody = sender.only_post().parse();
    assert_eq!(body.event.code, EVENT_CODE);
    assert_eq!(body.event.external_subscription_id, "end-user-1");
    assert_eq!(body.event.properties.model.as_deref(), Some(MODEL));
    assert_eq!(
        body.event.properties.response_cost,
        Some(Decimal::new(12, 4))
    );
    assert_eq!(body.event.properties.prompt_tokens, Some(10));
    assert_eq!(body.event.properties.completion_tokens, Some(20));
    assert_eq!(body.event.properties.total_tokens, Some(30));
    Uuid::parse_str(&body.event.transaction_id).unwrap();
}

#[tokio::test]
async fn post_logs_the_built_event_at_debug() {
    let (captured, _guard) = capture_debug();
    let sender = FakeSender::default();
    logger(sender).log_success_event(&event()).await.unwrap();
    let logs = captured.text();
    assert!(logs.contains("Logged Lago Object"), "{logs}");
    assert!(logs.contains(EVENT_CODE), "{logs}");
}

#[tokio::test]
async fn post_logs_the_rejected_response_body_and_returns_the_status() {
    let sender = FakeSender::answering(422, "subscription is unknown");
    let (captured, _guard) = capture_debug();
    let error = logger(sender)
        .log_success_event(&event())
        .await
        .unwrap_err();
    assert!(matches!(error, LagoError::Status { status: 422 }));
    assert!(captured.text().contains("subscription is unknown"));
}

#[tokio::test]
async fn post_propagates_a_transport_error() {
    let error = logger(FakeSender::failing())
        .log_success_event(&event())
        .await
        .unwrap_err();
    assert!(matches!(error, LagoError::Send(Unreachable)));
}

#[tokio::test]
async fn post_rejects_an_invalid_base_url_without_sending() {
    let sender = FakeSender::default();
    let error = logger_with(sender.clone(), &[("LAGO_API_BASE", "not a url")])
        .log_success_event(&event())
        .await
        .unwrap_err();
    assert!(matches!(error, LagoError::Url(_)));
    assert!(sender.posts().is_empty());
}

#[tokio::test]
async fn post_reports_a_missing_subscription_without_sending() {
    let sender = FakeSender::default();
    let mut call = event();
    call.request_user = None;
    let error = logger(sender.clone())
        .log_success_event(&call)
        .await
        .unwrap_err();
    assert!(matches!(error, LagoError::Payload(_)));
    assert!(sender.posts().is_empty());
}
