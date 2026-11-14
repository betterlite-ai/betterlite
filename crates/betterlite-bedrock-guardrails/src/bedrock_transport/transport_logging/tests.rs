#![cfg(test)]

use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;

use crate::bedrock_post::sign_and_post;
use crate::bedrock_transport::test_support::{
    RecordingLog, RecordingTransport, capture, guardrail_with, prepared_request,
};

#[tokio::test]
async fn transport_logging_logs_an_http_error_when_enabled() {
    let guardrail = guardrail_with(OpaqueParams::default());
    let transport = RecordingTransport::replying(400, r#"{"message":"bad input"}"#);
    let log = RecordingLog::default();
    let _error = sign_and_post(&guardrail, &prepared_request(), &transport, &log, true)
        .await
        .expect_err("the HTTP error");
    assert_eq!(log.details(), vec!["bad input".to_owned()]);
}

#[tokio::test]
async fn transport_logging_suppresses_an_http_error_when_disabled() {
    let guardrail = guardrail_with(OpaqueParams::default());
    let transport = RecordingTransport::replying(400, r#"{"message":"bad input"}"#);
    let log = RecordingLog::default();
    let _error = sign_and_post(&guardrail, &prepared_request(), &transport, &log, false)
        .await
        .expect_err("the HTTP error");
    assert!(log.details().is_empty());
}

#[tokio::test]
async fn transport_logging_logs_a_connection_failure_even_when_disabled() {
    let (captured, _guard) = capture();
    let guardrail = guardrail_with(OpaqueParams::default());
    let transport = RecordingTransport::failing();
    let log = RecordingLog::default();
    let _error = sign_and_post(&guardrail, &prepared_request(), &transport, &log, false)
        .await
        .expect_err("the transport error");
    assert_eq!(log.details().len(), 1);
    let errors = captured.errors();
    assert_eq!(errors.len(), 1);
    assert!(errors[0].contains("failed to make guardrail request"));
}
