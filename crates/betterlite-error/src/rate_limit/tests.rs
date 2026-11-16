#![cfg(test)]

use std::collections::BTreeMap;

use axum::http::StatusCode;
use axum::http::header::HeaderValue;
use axum::response::IntoResponse as _;
use betterlite_redact::structured::Payload;
use rstest::rstest;
use serde_json::json;

use super::{
    ProxyRateLimitError, ProxyRateLimitErrorInit, RateLimitCategory, RateLimitDimension,
    coerce_message,
};

#[rstest]
#[case::vendor(RateLimitCategory::VendorRateLimit, "vendor_rate_limit")]
#[case::vendor_batch(RateLimitCategory::VendorBatchRateLimit, "vendor_batch_rate_limit")]
#[case::litellm(RateLimitCategory::LitellmRateLimit, "litellm_rate_limit")]
#[case::litellm_batch(RateLimitCategory::LitellmBatchRateLimit, "litellm_batch_rate_limit")]
fn rate_limit_category_tests_behavior(#[case] category: RateLimitCategory, #[case] wire: &str) {
    assert_eq!(serde_json::to_value(category).unwrap(), wire);
}

#[rstest]
#[case::requests(RateLimitDimension::Requests, "requests")]
#[case::tokens(RateLimitDimension::Tokens, "tokens")]
#[case::concurrent(RateLimitDimension::ConcurrentRequests, "concurrent_requests")]
#[case::budget(RateLimitDimension::Budget, "budget")]
#[case::max_iterations(RateLimitDimension::MaxIterations, "max_iterations")]
fn rate_limit_dimension_tests_behavior(#[case] dimension: RateLimitDimension, #[case] wire: &str) {
    assert_eq!(serde_json::to_value(dimension).unwrap(), wire);
}

#[rstest]
#[case::none(json!(null), "")]
#[case::text(json!("plain rate limit"), "plain rate limit")]
#[case::error_text(json!({"error": "flat"}), "flat")]
#[case::message_text(json!({"message": "slow down"}), "slow down")]
#[case::nested_error(json!({"error": {"message": "inner"}}), "inner")]
#[case::nested_message(json!({"message": {"message": "deep"}}), "deep")]
#[case::error_before_message(json!({"error": "first", "message": "second"}), "first")]
#[case::non_text_error_falls_through(json!({"error": 42, "message": "fallback"}), "fallback")]
#[case::object_json(json!({"foo": "bar"}), "{\"foo\":\"bar\"}")]
#[case::number(json!(7), "7")]
fn coerce_message_tests_behavior(#[case] detail: Payload, #[case] expected: &str) {
    assert_eq!(coerce_message(&detail), expected);
}

fn error_with_headers(headers: BTreeMap<String, Payload>) -> ProxyRateLimitError {
    ProxyRateLimitError::new(ProxyRateLimitErrorInit {
        detail: Payload::Null,
        headers: Some(headers),
        category: None,
        rate_limit_type: None,
        model: None,
        llm_provider: None,
    })
}

#[test]
fn new_defaults_match_litellm() {
    let error = ProxyRateLimitError::new(ProxyRateLimitErrorInit::default());
    assert_eq!(error.message, "");
    assert_eq!(error.headers, None);
    assert_eq!(error.category, RateLimitCategory::LitellmRateLimit);
    assert_eq!(error.rate_limit_type, None);
    assert_eq!(error.model, "");
    assert_eq!(error.llm_provider, "litellm_proxy");
    assert_eq!(error.detail, json!(null));
    assert_eq!(error.to_string(), "");
    assert_eq!(
        ProxyRateLimitError::PROMETHEUS_EXCEPTION_CLASS,
        "HTTPException"
    );
}

#[test]
fn new_keeps_given_fields() {
    let init = ProxyRateLimitErrorInit {
        detail: json!({"error": {"message": "slow down"}}),
        headers: Some(BTreeMap::from([
            (String::from("retry-after"), json!(5)),
            (String::from("x-bucket"), json!("a")),
        ])),
        category: Some(RateLimitCategory::LitellmBatchRateLimit),
        rate_limit_type: Some(RateLimitDimension::Budget),
        model: Some(String::from("gpt-4o")),
        llm_provider: Some(String::from("openai")),
    };
    let error = ProxyRateLimitError::new(init);
    assert_eq!(error.message, "slow down");
    assert_eq!(
        error.headers,
        Some(BTreeMap::from([
            (String::from("retry-after"), String::from("5")),
            (String::from("x-bucket"), String::from("a")),
        ]))
    );
    assert_eq!(error.category, RateLimitCategory::LitellmBatchRateLimit);
    assert_eq!(error.rate_limit_type, Some(RateLimitDimension::Budget));
    assert_eq!(error.model, "gpt-4o");
    assert_eq!(error.llm_provider, "openai");
    assert_eq!(error.to_string(), "slow down");
}

#[rstest]
#[case::text(json!("z"), "z")]
#[case::integer(json!(5), "5")]
#[case::float(json!(1.5), "1.5")]
#[case::empty_text(json!(""), "")]
fn header_values_are_stringified(#[case] value: Payload, #[case] expected: &str) {
    let error = error_with_headers(BTreeMap::from([(String::from("x-test"), value)]));
    assert_eq!(
        error.headers,
        Some(BTreeMap::from([(
            String::from("x-test"),
            expected.to_owned()
        )]))
    );
}

#[test]
fn debug_hides_header_values() {
    let error = error_with_headers(BTreeMap::from([(
        String::from("authorization"),
        json!("sk-secret"),
    )]));
    let rendered = format!("{error:?}");
    assert!(rendered.contains("authorization"));
    assert!(!rendered.contains("sk-secret"));
}

#[tokio::test]
async fn into_response_is_a_429_with_detail_and_headers() {
    let init = ProxyRateLimitErrorInit {
        detail: json!({"error": {"message": "slow down"}}),
        headers: Some(BTreeMap::from([
            (String::from("retry-after"), json!(5)),
            (String::from("x-bucket"), json!("a")),
        ])),
        category: None,
        rate_limit_type: None,
        model: None,
        llm_provider: None,
    };
    let response = ProxyRateLimitError::new(init).into_response();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        response.headers().get("retry-after"),
        Some(&HeaderValue::from_static("5"))
    );
    assert_eq!(
        response.headers().get("x-bucket"),
        Some(&HeaderValue::from_static("a"))
    );
    let body = axum::body::to_bytes(response.into_body(), 1024)
        .await
        .expect("the detail body is small");
    assert_eq!(
        serde_json::from_slice::<Payload>(&body).expect("the body is JSON"),
        json!({"detail": {"error": {"message": "slow down"}}})
    );
}

#[tokio::test]
async fn into_response_drops_headers_http_cannot_carry() {
    let error = error_with_headers(BTreeMap::from([(
        String::from("bad header"),
        json!("value"),
    )]));
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(response.headers().get("bad header").is_none());
}
