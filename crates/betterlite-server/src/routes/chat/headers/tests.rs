#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use axum::http::{HeaderMap, HeaderName, HeaderValue};
use betterlite_config::loader::ConfigLoader;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_openai::chat::deployment::{ChatDeployment, OpenAiDefaults};
use betterlite_openai::chat::execution::ChatError;
use rstest::rstest;

use super::{failure_headers, stream_headers};

#[rstest]
#[case::overhead(1500, "0.266")]
#[case::saturates(1000, "0.000")]
fn stream_timing_headers(#[case] total_micros: u64, #[case] overhead: &str) {
    let config = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("model_list:\n  - model_name: g\n    litellm_params:\n      model: openai/up\n")
    .unwrap();
    let deployment = ChatDeployment::resolve(
        config.model_list.first().unwrap(),
        &OpenAiDefaults::default(),
    )
    .unwrap();
    let headers = stream_headers(
        &HeaderMap::new(),
        &deployment,
        "stream-timing",
        Duration::from_micros(1234),
        Duration::from_micros(total_micros),
    );
    assert_eq!(
        headers.get("x-litellm-response-duration-ms").unwrap(),
        "1.234"
    );
    assert_eq!(
        headers.get("x-litellm-overhead-duration-ms").unwrap(),
        overhead
    );
}

#[test]
fn failure_headers_preserves_repeated_upstream_values() {
    let mut upstream = HeaderMap::new();
    upstream.append(
        "x-ratelimit-remaining-tokens",
        HeaderValue::from_static("1"),
    );
    upstream.append(
        "x-ratelimit-remaining-tokens",
        HeaderValue::from_static("2"),
    );
    let error = ChatError::Provider {
        response: Box::new(UpstreamResponse {
            status: 429,
            headers: upstream,
            body: Vec::new(),
        }),
    };
    let headers = failure_headers(Some(&error), None, "call-1", None);
    let forwarded: Vec<&HeaderValue> = headers
        .get_all("llm_provider-x-ratelimit-remaining-tokens")
        .iter()
        .collect();
    assert_eq!(forwarded, vec!["1", "2"]);
}

#[test]
fn failure_headers_drops_an_unsafe_prefixed_header() {
    let mut upstream = HeaderMap::new();
    upstream.insert("set-cookie", HeaderValue::from_static("a=b"));
    upstream.insert("x-request-id", HeaderValue::from_static("abc"));
    let error = ChatError::Provider {
        response: Box::new(UpstreamResponse {
            status: 500,
            headers: upstream,
            body: Vec::new(),
        }),
    };
    let headers = failure_headers(Some(&error), None, "call-3", None);
    assert!(headers.get("llm_provider-set-cookie").is_none());
    assert!(headers.get("set-cookie").is_none());
    assert_eq!(headers.get("llm_provider-x-request-id").unwrap(), "abc");
}

#[test]
fn failure_headers_forwards_a_non_utf8_value_losslessly() {
    let mut upstream = HeaderMap::new();
    upstream.insert(
        HeaderName::from_static("x-vendor-trace"),
        HeaderValue::from_bytes(&[0xC3, 0x28]).unwrap(),
    );
    let error = ChatError::Provider {
        response: Box::new(UpstreamResponse {
            status: 500,
            headers: upstream,
            body: Vec::new(),
        }),
    };
    let headers = failure_headers(Some(&error), None, "call-2", None);
    assert_eq!(
        headers
            .get("llm_provider-x-vendor-trace")
            .unwrap()
            .as_bytes(),
        &[0xC3, 0x28]
    );
}
