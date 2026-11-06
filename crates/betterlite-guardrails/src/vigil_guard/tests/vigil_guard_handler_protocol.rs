use std::time::Duration;

use betterlite_guardrails::vigil_guard::DEFAULT_TIMEOUT;
use betterlite_guardrails::vigil_guard::handler_protocol::AsyncPostHandler as _;
use reqwest::StatusCode;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};

use crate::support::{FakeHandler, json_body, payload};

#[tokio::test]
async fn handler_method_signature() {
    let handler = FakeHandler::new(vec![Ok(json_body("{}"))]);
    let headers = HeaderMap::new();
    let payload = payload();
    let response = handler
        .post(
            "https://vigil.example/v1/guard/analyze",
            &headers,
            &payload,
            DEFAULT_TIMEOUT,
        )
        .await
        .unwrap();
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.body, b"{}".to_vec());
}

#[tokio::test]
async fn post_with_url_headers_json() {
    let handler = FakeHandler::new(vec![Ok(json_body("{}"))]);
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer sk-test"));
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    handler
        .post(
            "https://vigil.example/v1/guard/analyze",
            &headers,
            &payload(),
            DEFAULT_TIMEOUT,
        )
        .await
        .unwrap();
    let calls = handler.calls();
    let call = &calls[0];
    assert_eq!(call.url, "https://vigil.example/v1/guard/analyze");
    assert_eq!(
        call.headers.get(AUTHORIZATION).unwrap().to_str().unwrap(),
        "Bearer sk-test"
    );
    assert_eq!(
        call.headers.get(CONTENT_TYPE).unwrap().to_str().unwrap(),
        "application/json"
    );
    assert_eq!(call.payload.text, "hello");
    assert_eq!(call.timeout.total, Duration::from_secs(10));
    assert_eq!(call.timeout.connect, Duration::from_secs(5));
    assert_eq!(call.timeout, DEFAULT_TIMEOUT);
}
