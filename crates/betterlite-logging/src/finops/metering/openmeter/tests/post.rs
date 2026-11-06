#![cfg(test)]

use rstest::rstest;
use rust_decimal::Decimal;

use super::support::{API_KEY, CloudEventBody, http_logger, logger, logger_with};
use crate::finops::metering::openmeter::error::OpenMeterError;
use crate::finops::metering::support::{
    FakeSender, MODEL, Unreachable, assert_metering_request, event, serve_once,
};

#[tokio::test]
async fn post_sends_to_the_configured_endpoint_with_the_configured_key() {
    let sender = FakeSender::default();
    logger(sender.clone())
        .log_success_event(&event())
        .await
        .unwrap();
    let posted = sender.only_post();
    assert_eq!(posted.url, "https://openmeter.test/api/v1/events");
    assert_eq!(posted.content_type, "application/cloudevents+json");
    assert_eq!(posted.api_key, API_KEY);
}

#[tokio::test]
async fn post_sends_the_bearer_header_and_cloudevent_through_the_http_sender() {
    let (base, served) = serve_once(200, "{}").await;
    let logger = http_logger(&base);
    let call = event();
    let (result, received) = tokio::join!(logger.log_success_event(&call), served);
    result.unwrap();
    let content_type = "application/cloudevents+json";
    let body: CloudEventBody =
        serde_json::from_slice(assert_metering_request(&received, content_type, API_KEY)).unwrap();
    assert_eq!(body.source, "litellm-proxy");
    assert_eq!(body.subject, "end-user-1");
}

#[tokio::test]
async fn post_wraps_a_non_2xx_status_through_the_http_sender() {
    let (base, served) = serve_once(422, "unknown meter").await;
    let logger = http_logger(&base);
    let call = event();
    let (result, _received) = tokio::join!(logger.log_success_event(&call), served);
    assert_eq!(
        result.unwrap_err().to_string(),
        "OpenMeter logging error: unknown meter"
    );
}

#[rstest]
#[case::no_trailing_slash("https://openmeter.test", "https://openmeter.test/api/v1/events")]
#[case::one_trailing_slash("https://openmeter.test/", "https://openmeter.test/api/v1/events")]
#[case::with_a_path(
    "https://openmeter.test/metering",
    "https://openmeter.test/metering/api/v1/events"
)]
#[tokio::test]
async fn post_appends_the_events_path_once(
    #[case] endpoint: &'static str,
    #[case] expected: &'static str,
) {
    let sender = FakeSender::default();
    logger_with(sender.clone(), &[("OPENMETER_API_ENDPOINT", endpoint)])
        .log_success_event(&event())
        .await
        .unwrap();
    assert_eq!(sender.only_post().url, expected);
}

#[tokio::test]
async fn post_sends_the_cloud_event_json() {
    let sender = FakeSender::default();
    logger(sender.clone())
        .log_success_event(&event())
        .await
        .unwrap();
    let body: CloudEventBody = sender.only_post().parse();
    assert_eq!(body.specversion, "1.0");
    assert_eq!(body.event_type, "litellm_tokens");
    assert_eq!(body.id.as_deref(), Some("chatcmpl-response-1"));
    assert_eq!(body.time, "2030-01-02T03:04:05.000000+00:00");
    assert_eq!(body.subject, "end-user-1");
    assert_eq!(body.source, "litellm-proxy");
    assert_eq!(body.data.model.as_deref(), Some(MODEL));
    assert_eq!(body.data.cost, Some(Decimal::new(12, 4)));
    assert_eq!(body.data.prompt_tokens, Some(10));
    assert_eq!(body.data.completion_tokens, Some(20));
    assert_eq!(body.data.total_tokens, Some(30));
}

#[tokio::test]
async fn post_wraps_an_http_error_with_the_response_body() {
    let sender = FakeSender::answering(400, "quota exceeded");
    let error = logger(sender)
        .log_success_event(&event())
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "OpenMeter logging error: quota exceeded");
    assert!(matches!(error, OpenMeterError::Status { status: 400, .. }));
}

#[tokio::test]
async fn post_propagates_a_transport_error() {
    let error = logger(FakeSender::failing())
        .log_success_event(&event())
        .await
        .unwrap_err();
    assert!(matches!(error, OpenMeterError::Send(Unreachable)));
}

#[tokio::test]
async fn post_rejects_an_invalid_endpoint_without_sending() {
    let sender = FakeSender::default();
    let error = logger_with(sender.clone(), &[("OPENMETER_API_ENDPOINT", "not a url")])
        .log_success_event(&event())
        .await
        .unwrap_err();
    assert!(matches!(error, OpenMeterError::Url(_)));
    assert!(sender.posts().is_empty());
}

#[tokio::test]
async fn post_reports_a_missing_subject_without_sending() {
    let sender = FakeSender::default();
    let mut call = event();
    call.request_user = None;
    call.user_api_key_user_id = None;
    let error = logger(sender.clone())
        .log_success_event(&call)
        .await
        .unwrap_err();
    assert!(matches!(error, OpenMeterError::Payload(_)));
    assert!(sender.posts().is_empty());
}
