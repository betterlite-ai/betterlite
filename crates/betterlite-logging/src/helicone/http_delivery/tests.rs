#![cfg(test)]

use std::time::Duration;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;
use tokio::join;

use super::{DeliveryError, post};
use crate::helicone::envelope;
use crate::helicone::test_support::{COMPLETION, Received, event, json, logger, serve_once};

async fn deliver(model: &str, key: &str, status: u16) -> (Result<u16, DeliveryError>, Received) {
    let (base, server) = serve_once(status, "ok").await;
    let logger = logger(&base, Some(key));
    let secret = SecretString::from(key);
    let assembled = envelope::assemble(&logger.settings, &secret, &event(model)).unwrap();
    let (result, received) = join!(
        post(&logger.client, &secret, assembled, Duration::from_secs(5)),
        server
    );
    (result.map(|response| response.status), received)
}

fn body(received: &Received) -> Json {
    serde_json::from_slice(&received.body).unwrap()
}

#[tokio::test]
async fn http_delivery_posts_json_with_a_bearer_token() {
    let (result, received) = deliver("gpt-4o", "sk-helicone", 200).await;
    assert_eq!(result.unwrap(), 200);
    assert_eq!(received.request_line, "POST /oai/v1/log HTTP/1.1");
    assert_eq!(received.headers["authorization"], "Bearer sk-helicone");
    assert_eq!(received.headers["content-type"], "application/json");
}

#[tokio::test]
async fn http_delivery_sends_the_whole_envelope() {
    let (_result, received) = deliver("gpt-4o", "sk-helicone", 200).await;
    let expected = json(&format!(
        r#"{{
            "providerRequest": {{
                "url": "https://api.openai.com/v1",
                "json": {{"model": "gpt-4o", "messages": [{{"role": "user", "content": "hi"}}]}},
                "meta": {{"Helicone-Auth": "Bearer sk-helicone"}}
            }},
            "providerResponse": {{
                "json": {COMPLETION},
                "headers": {{"openai-version": "2020-10-01"}},
                "status": 200
            }},
            "timing": {{
                "startTime": {{"seconds": 1700000000, "milliseconds": 0}},
                "endTime": {{"seconds": 1700000001, "milliseconds": 299}}
            }}
        }}"#
    ));
    assert_eq!(body(&received), expected);
}

#[tokio::test]
async fn http_delivery_sends_claude_models_to_the_anthropic_endpoint() {
    let (_result, received) = deliver("claude-3-opus", "k", 200).await;
    assert_eq!(received.request_line, "POST /anthropic/v1/log HTTP/1.1");
    let sent = body(&received);
    let Json::Object(envelope) = sent else {
        panic!("the body is not an object");
    };
    let expected_request = json(
        r#"{"url": "https://api.anthropic.com/v1/messages", "json": {"model": "claude-3-opus", "messages": [{"role": "user", "content": "hi"}]}, "meta": {"Helicone-Auth": "Bearer k"}}"#,
    );
    assert_eq!(envelope["providerRequest"], expected_request);
    let expected_response = json(
        r#"{"json": {"id": "chatcmpl-1", "type": "message", "role": "assistant", "model": "claude-3-opus", "content": [{"type": "text", "text": "hello"}], "stop_reason": "stop", "stop_sequence": null, "usage": {"input_tokens": 3, "output_tokens": 5}}, "headers": {"openai-version": "2020-10-01"}, "status": 200}"#,
    );
    assert_eq!(envelope["providerResponse"], expected_response);
}

#[tokio::test]
async fn http_delivery_sends_gemini_models_to_the_custom_endpoint() {
    let (_result, received) = deliver("gemini-pro", "k", 200).await;
    assert_eq!(received.request_line, "POST /custom/v1/log HTTP/1.1");
}

#[tokio::test]
async fn http_delivery_sends_metadata_without_the_otel_span() {
    let (base, server) = serve_once(200, "ok").await;
    let logger = logger(&base, Some("k"));
    let mut sent = event("gpt-4o");
    sent.litellm_params.metadata = IndexMap::from([
        ("team".to_owned(), Json::Text("search".to_owned())),
        (
            "litellm_parent_otel_span".to_owned(),
            Json::Text("span".to_owned()),
        ),
    ]);
    let secret = SecretString::from("k");
    let assembled = envelope::assemble(&logger.settings, &secret, &sent).unwrap();
    let (result, received) = join!(
        post(&logger.client, &secret, assembled, Duration::from_secs(5)),
        server
    );
    result.unwrap();
    let Json::Object(envelope) = body(&received) else {
        panic!("the body is not an object");
    };
    let Json::Object(request) = &envelope["providerRequest"] else {
        panic!("providerRequest is not an object");
    };
    assert_eq!(
        request["meta"],
        json(r#"{"Helicone-Auth": "Bearer k", "team": "search"}"#)
    );
}

#[tokio::test]
async fn http_delivery_returns_a_non_200_status_to_the_caller() {
    let (result, _received) = deliver("gpt-4o", "k", 500).await;
    assert_eq!(result.unwrap(), 500);
}

#[tokio::test]
async fn http_delivery_reports_an_unreachable_host() {
    let logger = logger("http://127.0.0.1:1", Some("k"));
    let secret = SecretString::from("k");
    let assembled = envelope::assemble(&logger.settings, &secret, &event("gpt-4o")).unwrap();
    let result = post(&logger.client, &secret, assembled, Duration::from_secs(5)).await;
    assert!(matches!(result, Err(DeliveryError::Post(_))));
}
