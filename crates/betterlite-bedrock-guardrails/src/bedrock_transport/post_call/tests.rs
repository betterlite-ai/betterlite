#![cfg(test)]

use std::time::Duration;

use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;

use crate::bedrock_post::sign_and_post;
use crate::bedrock_transport::BedrockGuardrail;
use crate::bedrock_transport::test_support::{
    RecordingLog, RecordingTransport, guardrail_init, guardrail_with, prepared_request,
};

fn header<'text>(headers: &'text [(String, String)], name: &str) -> Option<&'text str> {
    headers
        .iter()
        .find(|(existing, _)| existing.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

#[tokio::test]
async fn post_call_sends_the_prepared_request() {
    let guardrail = guardrail_with(OpaqueParams::default());
    let transport = RecordingTransport::replying(200, "{}");
    let log = RecordingLog::default();
    let response = sign_and_post(&guardrail, &prepared_request(), &transport, &log, true)
        .await
        .expect("the response");
    assert_eq!(response.status, 200);
    let sent = transport.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].method, "POST");
    assert_eq!(sent[0].url, prepared_request().url);
    assert_eq!(sent[0].body, prepared_request().body);
    assert_eq!(
        header(&sent[0].headers, "Content-Type"),
        Some("application/json")
    );
    assert!(header(&sent[0].headers, "Authorization").is_some());
    assert!(log.details().is_empty());
}

#[tokio::test]
async fn post_call_uses_the_guardrail_timeout() {
    let mut init = guardrail_init();
    init.timeout = Some(Duration::from_secs(30));
    let guardrail = BedrockGuardrail::__init__(init).expect("the guardrail");
    let transport = RecordingTransport::replying(200, "{}");
    let log = RecordingLog::default();
    sign_and_post(&guardrail, &prepared_request(), &transport, &log, true)
        .await
        .expect("the response");
    assert_eq!(transport.sent()[0].deadline, Duration::from_secs(30));
}

#[tokio::test]
async fn post_call_defaults_the_timeout_to_six_hundred_seconds() {
    let guardrail = guardrail_with(OpaqueParams::default());
    let transport = RecordingTransport::replying(200, "{}");
    let log = RecordingLog::default();
    sign_and_post(&guardrail, &prepared_request(), &transport, &log, true)
        .await
        .expect("the response");
    assert_eq!(transport.sent()[0].deadline, Duration::from_secs(600));
}

#[tokio::test]
async fn post_call_returns_the_response_body() {
    let guardrail = guardrail_with(OpaqueParams::default());
    let transport = RecordingTransport::replying(200, r#"{"action":"NONE"}"#);
    let log = RecordingLog::default();
    let response = sign_and_post(&guardrail, &prepared_request(), &transport, &log, true)
        .await
        .expect("the response");
    assert_eq!(response.body, br#"{"action":"NONE"}"#.to_vec());
}

#[tokio::test]
async fn post_call_marks_only_the_authorization_header_sensitive() {
    let guardrail = guardrail_with(OpaqueParams::default());
    let transport = RecordingTransport::replying(200, "{}");
    let log = RecordingLog::default();
    sign_and_post(&guardrail, &prepared_request(), &transport, &log, true)
        .await
        .expect("the response");
    assert_eq!(transport.sent()[0].sensitive_headers, vec!["authorization"]);
}
