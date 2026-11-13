use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::anthropic_headers::Settings as HeaderConfig;
use http::{HeaderMap, HeaderValue};

use crate::headers::catalog::{
    ModelRecord, create_anthropic_model_list_response, model_created_at,
};
use crate::headers::response::process_anthropic_headers;

pub(super) fn response_header_prefixes_case() {
    let mut headers = HeaderMap::new();
    headers.insert(
        "anthropic-ratelimit-requests-limit",
        HeaderValue::from_static("10"),
    );
    headers.insert(
        "anthropic-ratelimit-requests-remaining",
        HeaderValue::from_static("9"),
    );
    headers.insert(
        "anthropic-ratelimit-tokens-limit",
        HeaderValue::from_static("1000"),
    );
    headers.insert(
        "anthropic-ratelimit-tokens-remaining",
        HeaderValue::from_static("999"),
    );
    headers.insert("request-id", HeaderValue::from_static("req_1"));
    headers.insert("content-type", HeaderValue::from_static("application/json"));
    let projected = process_anthropic_headers(&headers);
    assert_eq!(
        projected.get("llm_provider-request-id").map(String::as_str),
        Some("req_1")
    );
    assert_eq!(
        projected
            .get("llm_provider-anthropic-ratelimit-requests-limit")
            .map(String::as_str),
        Some("10")
    );
    assert_eq!(
        projected
            .get("x-ratelimit-limit-requests")
            .map(String::as_str),
        Some("10")
    );
    assert_eq!(
        projected
            .get("x-ratelimit-remaining-requests")
            .map(String::as_str),
        Some("9")
    );
    assert_eq!(
        projected
            .get("x-ratelimit-limit-tokens")
            .map(String::as_str),
        Some("1000")
    );
    assert_eq!(
        projected
            .get("x-ratelimit-remaining-tokens")
            .map(String::as_str),
        Some("999")
    );

    let mut without_limits = HeaderMap::new();
    without_limits.insert("request-id", HeaderValue::from_static("req_2"));
    let projected = process_anthropic_headers(&without_limits);
    assert_eq!(
        projected.get("llm_provider-request-id").map(String::as_str),
        Some("req_2")
    );
    assert!(!projected.contains_key("x-ratelimit-limit-requests"));
    assert!(!projected.contains_key("x-ratelimit-remaining-requests"));
    assert!(!projected.contains_key("x-ratelimit-limit-tokens"));
    assert!(!projected.contains_key("x-ratelimit-remaining-tokens"));

    assert!(process_anthropic_headers(&HeaderMap::new()).is_empty());

    let mut repeated = HeaderMap::new();
    repeated.append("x-repeated", HeaderValue::from_static("one"));
    repeated.append("x-repeated", HeaderValue::from_static("two"));
    repeated.append(
        "anthropic-ratelimit-requests-limit",
        HeaderValue::from_static("5"),
    );
    repeated.append(
        "anthropic-ratelimit-requests-limit",
        HeaderValue::from_static("6"),
    );
    let projected = process_anthropic_headers(&repeated);
    assert_eq!(
        projected.get("llm_provider-x-repeated").map(String::as_str),
        Some("one, two")
    );
    assert_eq!(
        projected
            .get("x-ratelimit-limit-requests")
            .map(String::as_str),
        Some("5, 6")
    );
}

pub(super) fn catalog_created_at_default_case() {
    let lookup = BTreeMap::<String, String>::new();
    let default = ConfigLoader { lookup }.load("").unwrap();
    let settings: HeaderConfig = default.sections.anthropic_headers;
    assert_eq!(settings.default_model_created_at_time, 1_677_610_602);
    assert_eq!(
        model_created_at(settings.default_model_created_at_time).unwrap(),
        "2023-02-28T20:16:42Z"
    );

    let lookup = BTreeMap::from([("DEFAULT_MODEL_CREATED_AT_TIME".to_owned(), "42".to_owned())]);
    let configured = ConfigLoader { lookup }
        .load("")
        .unwrap()
        .sections
        .anthropic_headers;
    assert_eq!(configured.default_model_created_at_time, 42);
    assert_eq!(
        model_created_at(configured.default_model_created_at_time).unwrap(),
        "1970-01-01T00:00:42Z"
    );

    let lookup = BTreeMap::from([("DEFAULT_MODEL_CREATED_AT_TIME".to_owned(), String::new())]);
    let empty = ConfigLoader { lookup }
        .load("")
        .unwrap()
        .sections
        .anthropic_headers;
    assert_eq!(empty.default_model_created_at_time, 1_677_610_602);

    let lookup = BTreeMap::from([(
        "DEFAULT_MODEL_CREATED_AT_TIME".to_owned(),
        "not-a-number".to_owned(),
    )]);
    let error = ConfigLoader { lookup }.load("").unwrap_err();
    assert_eq!(error.category(), "environment");
    assert!(error.to_string().contains("DEFAULT_MODEL_CREATED_AT_TIME"));

    let models = vec![ModelRecord {
        id: "claude-sonnet-4".to_owned(),
        max_input_tokens: None,
        max_output_tokens: None,
    }];
    let response = create_anthropic_model_list_response(
        &models,
        &BTreeMap::new(),
        &BTreeMap::new(),
        &model_created_at(settings.default_model_created_at_time).unwrap(),
    );
    assert_eq!(
        response.data.first().map(|entry| entry.created_at.as_str()),
        Some("2023-02-28T20:16:42Z")
    );
}
