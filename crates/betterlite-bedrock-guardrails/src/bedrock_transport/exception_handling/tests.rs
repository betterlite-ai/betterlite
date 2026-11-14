#![cfg(test)]

use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use betterlite_http::proxy::transport_error::TransportError;

use crate::bedrock_post::{PostError, sign_and_post};
use crate::bedrock_transport::test_support::{
    RecordingLog, RecordingTransport, guardrail_with, prepared_request,
};

#[tokio::test]
async fn exception_handling_parses_a_client_error() {
    let guardrail = guardrail_with(OpaqueParams::default());
    let transport = RecordingTransport::replying(400, r#"{"message":"bad input"}"#);
    let log = RecordingLog::default();
    let error = sign_and_post(&guardrail, &prepared_request(), &transport, &log, true)
        .await
        .expect_err("the HTTP error");
    match error {
        PostError::Http { status, detail } => {
            assert_eq!(status, 400);
            assert_eq!(detail, "bad input");
        }
        other => panic!("expected the HTTP error, got {other}"),
    }
}

#[tokio::test]
async fn exception_handling_parses_a_server_error_nested_shape() {
    let guardrail = guardrail_with(OpaqueParams::default());
    let transport = RecordingTransport::replying(500, r#"{"error":{"message":"boom"}}"#);
    let log = RecordingLog::default();
    let error = sign_and_post(&guardrail, &prepared_request(), &transport, &log, true)
        .await
        .expect_err("the HTTP error");
    match error {
        PostError::Http { status, detail } => {
            assert_eq!(status, 500);
            assert_eq!(detail, "boom");
        }
        other => panic!("expected the HTTP error, got {other}"),
    }
}

#[tokio::test]
async fn exception_handling_returns_the_transport_error() {
    let guardrail = guardrail_with(OpaqueParams::default());
    let transport = RecordingTransport::failing();
    let log = RecordingLog::default();
    let error = sign_and_post(&guardrail, &prepared_request(), &transport, &log, true)
        .await
        .expect_err("the transport error");
    match error {
        PostError::Transport { source } => {
            assert!(matches!(source, TransportError::Closed));
        }
        other => panic!("expected the transport error, got {other}"),
    }
}

#[tokio::test]
async fn exception_handling_rejects_a_redirect_response() {
    let guardrail = guardrail_with(OpaqueParams::default());
    let transport = RecordingTransport::replying(302, "");
    let log = RecordingLog::default();
    let error = sign_and_post(&guardrail, &prepared_request(), &transport, &log, true)
        .await
        .expect_err("the HTTP error");
    match error {
        PostError::Http { status, detail } => {
            assert_eq!(status, 302);
            assert_eq!(detail, "Bedrock guardrail request failed");
        }
        other => panic!("expected the HTTP error, got {other}"),
    }
}

#[tokio::test]
async fn exception_handling_rejects_a_not_modified_response() {
    let guardrail = guardrail_with(OpaqueParams::default());
    let transport = RecordingTransport::replying(304, "");
    let log = RecordingLog::default();
    let error = sign_and_post(&guardrail, &prepared_request(), &transport, &log, true)
        .await
        .expect_err("the HTTP error");
    assert!(matches!(error, PostError::Http { status: 304, .. }));
}

#[tokio::test]
async fn exception_handling_accepts_the_last_success_status() {
    let guardrail = guardrail_with(OpaqueParams::default());
    let transport = RecordingTransport::replying(299, "{}");
    let log = RecordingLog::default();
    let response = sign_and_post(&guardrail, &prepared_request(), &transport, &log, true)
        .await
        .expect("a 2xx reply is a success");
    assert_eq!(response.status, 299);
}

#[tokio::test]
async fn exception_handling_accepts_the_first_success_status() {
    let guardrail = guardrail_with(OpaqueParams::default());
    let transport = RecordingTransport::replying(200, "{}");
    let log = RecordingLog::default();
    let response = sign_and_post(&guardrail, &prepared_request(), &transport, &log, true)
        .await
        .expect("a 2xx reply is a success");
    assert_eq!(response.status, 200);
}

#[tokio::test]
async fn exception_handling_rejects_an_informational_response() {
    let guardrail = guardrail_with(OpaqueParams::default());
    let transport = RecordingTransport::replying(199, "");
    let log = RecordingLog::default();
    let error = sign_and_post(&guardrail, &prepared_request(), &transport, &log, true)
        .await
        .expect_err("the HTTP error");
    assert!(matches!(error, PostError::Http { status: 199, .. }));
}
