#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_logging::cache_observer::{ObservationParser, WireRequest};
use betterlite_models::decisions::Json;
use http::{HeaderMap, HeaderValue};
use rstest::rstest;
use url::Url;

use super::{ObservedCacheParser, parse_observed_cache};
use crate::cache_plan::parse_prompt;
use crate::cache_prediction_targets::scope::cache_scope;

const API_KEY: &str = "sk-ant-api03-key";
const NATIVE_URL: &str = "https://api.anthropic.com/v1/messages";
const BODY: &str = r#"{"model":"claude-3-5-sonnet","messages":[{"role":"user","content":[{"type":"text","text":"a","cache_control":{"type":"ephemeral"}}]}]}"#;
const ONE_HOUR_BODY: &str = r#"{"model":"claude-3-5-sonnet","messages":[{"role":"user","content":[{"type":"text","text":"a","cache_control":{"type":"ephemeral","ttl":"1h"}}]}]}"#;
const OPUS_BODY: &str = r#"{"model":"claude-3-opus","messages":[{"role":"user","content":[{"type":"text","text":"a","cache_control":{"type":"ephemeral"}}]}]}"#;

fn headers() -> HeaderMap {
    headers_with(API_KEY)
}

fn headers_with(key: &'static str) -> HeaderMap {
    let mut map = HeaderMap::new();
    map.insert("host", HeaderValue::from_static("api.anthropic.com"));
    map.insert("content-type", HeaderValue::from_static("application/json"));
    map.insert("x-api-key", HeaderValue::from_static(key));
    map.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
    map
}

fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn response(cached: i64, creation: i64, split: Option<(i64, i64)>) -> Json {
    let details = match split {
        Some((five, one)) => format!(
            r#","cache_creation_token_details":{{"ephemeral_5m_input_tokens":{five},"ephemeral_1h_input_tokens":{one}}}"#
        ),
        None => String::new(),
    };
    let encoded = format!(
        r#"{{"model":"claude-3-5-sonnet","usage":{{"prompt_tokens":1000,"prompt_tokens_details":{{"cached_tokens":{cached},"cache_creation_tokens":{creation}{details}}}}},"choices":[{{"finish_reason":"stop"}}]}}"#
    );
    json(&encoded)
}

fn hit() -> Json {
    response(100, 0, None)
}

fn wire_request<'wire>(
    url: &'wire Url,
    headers: &'wire HeaderMap,
    body: &'wire [u8],
) -> WireRequest<'wire> {
    WireRequest { url, headers, body }
}

#[test]
fn parse_observed_cache_returns_the_prefix_evidence() {
    let url = Url::parse(NATIVE_URL).unwrap();
    let headers = headers();
    let wire = WireRequest {
        url: &url,
        headers: &headers,
        body: BODY.as_bytes(),
    };
    let observed = parse_observed_cache(&wire, &hit(), "caller", "deployment").unwrap();
    let prefix = parse_prompt(BODY).unwrap();
    assert_eq!(observed.prefix.fingerprint, prefix.fingerprint);
    assert_eq!(observed.prefix.fingerprints, prefix.fingerprints);
    assert_eq!(observed.prefix.ttl_seconds, 300);
    assert_eq!(observed.cached_tokens, 100);
    assert_eq!(observed.cache_creation_tokens, 0);
    assert_eq!(
        observed.scope,
        cache_scope("caller", "deployment", API_KEY, "claude-3-5-sonnet")
    );
}

#[rstest]
#[case::default_scope("caller", "deployment", API_KEY)]
#[case::other_caller("other-caller", "deployment", API_KEY)]
#[case::other_deployment("caller", "other-deployment", API_KEY)]
#[case::other_key("caller", "deployment", "sk-ant-api03-other")]
fn parse_observed_cache_scopes_by_caller_key(
    #[case] caller: &str,
    #[case] deployment: &str,
    #[case] key: &'static str,
) {
    let url = Url::parse(NATIVE_URL).unwrap();
    let headers = headers_with(key);
    let wire = WireRequest {
        url: &url,
        headers: &headers,
        body: BODY.as_bytes(),
    };
    let observed = parse_observed_cache(&wire, &hit(), caller, deployment).unwrap();
    assert_eq!(
        observed.scope,
        cache_scope(caller, deployment, key, "claude-3-5-sonnet")
    );
}

#[rstest]
#[case::explicit_default_port("https://api.anthropic.com:443/v1/messages")]
#[case::empty_query("https://api.anthropic.com/v1/messages?")]
fn parse_observed_cache_accepts_native_recipients(#[case] recipient: &str) {
    let url = Url::parse(recipient).unwrap();
    let headers = headers();
    let wire = wire_request(&url, &headers, BODY.as_bytes());
    assert!(parse_observed_cache(&wire, &hit(), "caller", "deployment").is_some());
}

#[rstest]
#[case::plain_http("http://api.anthropic.com/v1/messages")]
#[case::other_host("https://api.example.com/v1/messages")]
#[case::other_path("https://api.anthropic.com/v1/complete")]
#[case::query("https://api.anthropic.com/v1/messages?beta=1")]
#[case::other_port("https://api.anthropic.com:8443/v1/messages")]
fn parse_observed_cache_rejects_non_native_recipients(#[case] recipient: &str) {
    let url = Url::parse(recipient).unwrap();
    let headers = headers();
    let wire = wire_request(&url, &headers, BODY.as_bytes());
    assert!(parse_observed_cache(&wire, &hit(), "caller", "deployment").is_none());
}

#[rstest]
#[case::anthropic_beta("anthropic-beta", "prompt-caching-2024-07-31")]
#[case::foreign_header("x-request-id", "abc")]
#[case::other_version("anthropic-version", "2023-01-01")]
fn parse_observed_cache_rejects_foreign_headers(
    #[case] name: &'static str,
    #[case] value: &'static str,
) {
    let url = Url::parse(NATIVE_URL).unwrap();
    let mut headers = headers();
    headers.insert(name, HeaderValue::from_static(value));
    let wire = WireRequest {
        url: &url,
        headers: &headers,
        body: BODY.as_bytes(),
    };
    assert!(parse_observed_cache(&wire, &hit(), "caller", "deployment").is_none());
}

#[test]
fn parse_observed_cache_requires_the_native_version() {
    let url = Url::parse(NATIVE_URL).unwrap();
    let mut headers = headers();
    headers.remove("anthropic-version");
    let wire = WireRequest {
        url: &url,
        headers: &headers,
        body: BODY.as_bytes(),
    };
    assert!(parse_observed_cache(&wire, &hit(), "caller", "deployment").is_none());
}

#[test]
fn parse_observed_cache_joins_repeated_versions_like_httpx() {
    let url = Url::parse(NATIVE_URL).unwrap();
    let mut headers = headers();
    headers.append("anthropic-version", HeaderValue::from_static("2023-06-01"));
    let wire = WireRequest {
        url: &url,
        headers: &headers,
        body: BODY.as_bytes(),
    };
    assert!(parse_observed_cache(&wire, &hit(), "caller", "deployment").is_none());
}

#[test]
fn parse_observed_cache_joins_repeated_keys_in_the_scope() {
    let url = Url::parse(NATIVE_URL).unwrap();
    let mut headers = headers();
    headers.append("x-api-key", HeaderValue::from_static("sk-ant-api03-other"));
    let wire = WireRequest {
        url: &url,
        headers: &headers,
        body: BODY.as_bytes(),
    };
    let observed = parse_observed_cache(&wire, &hit(), "caller", "deployment").unwrap();
    assert_eq!(
        observed.scope,
        cache_scope(
            "caller",
            "deployment",
            "sk-ant-api03-key, sk-ant-api03-other",
            "claude-3-5-sonnet"
        )
    );
}

#[rstest]
#[case::missing(None)]
#[case::empty(Some(""))]
#[case::oauth(Some("sk-ant-oat01-token"))]
fn parse_observed_cache_rejects_unsupported_provider_keys(#[case] key: Option<&'static str>) {
    let url = Url::parse(NATIVE_URL).unwrap();
    let mut headers = headers();
    match key {
        Some(value) => {
            headers.insert("x-api-key", HeaderValue::from_static(value));
        }
        None => {
            headers.remove("x-api-key");
        }
    }
    let wire = WireRequest {
        url: &url,
        headers: &headers,
        body: BODY.as_bytes(),
    };
    assert!(parse_observed_cache(&wire, &hit(), "caller", "deployment").is_none());
}

#[test]
fn parse_observed_cache_rejects_a_model_mismatch() {
    let url = Url::parse(NATIVE_URL).unwrap();
    let headers = headers();
    let wire = WireRequest {
        url: &url,
        headers: &headers,
        body: OPUS_BODY.as_bytes(),
    };
    assert!(parse_observed_cache(&wire, &hit(), "caller", "deployment").is_none());
}

#[test]
fn parse_observed_cache_rejects_a_non_utf8_body() {
    let url = Url::parse(NATIVE_URL).unwrap();
    let headers = headers();
    let body = [0xff_u8, 0xfe];
    let wire = WireRequest {
        url: &url,
        headers: &headers,
        body: &body,
    };
    assert!(parse_observed_cache(&wire, &hit(), "caller", "deployment").is_none());
}

#[rstest]
#[case::no_breakpoint(
    r#"{"model":"claude-3-5-sonnet","messages":[{"role":"user","content":[{"type":"text","text":"a"}]}]}"#
)]
#[case::two_breakpoints(
    r#"{"model":"claude-3-5-sonnet","messages":[{"role":"user","content":[{"type":"text","text":"a","cache_control":{"type":"ephemeral"}}]},{"role":"assistant","content":[{"type":"text","text":"b","cache_control":{"type":"ephemeral"}}]}]}"#
)]
#[case::thinking_block(
    r#"{"model":"claude-3-5-sonnet","messages":[{"role":"assistant","content":[{"type":"thinking","thinking":"t","signature":"s"},{"type":"text","text":"a","cache_control":{"type":"ephemeral"}}]}]}"#
)]
fn parse_observed_cache_rejects_unsupported_bodies(#[case] body: &str) {
    let url = Url::parse(NATIVE_URL).unwrap();
    let headers = headers();
    let wire = wire_request(&url, &headers, body.as_bytes());
    assert!(parse_observed_cache(&wire, &hit(), "caller", "deployment").is_none());
}

#[rstest]
#[case::no_cache_tokens(BODY, 0, 0, None)]
#[case::negative_cached(BODY, -1, 200, None)]
#[case::negative_cached_without_creation(BODY, -1, 0, None)]
#[case::over_prompt(BODY, 900, 200, None)]
#[case::over_prompt_with_reads(BODY, 1_001, 0, None)]
#[case::creation_without_split(BODY, 0, 100, None)]
#[case::split_mismatch(BODY, 0, 100, Some((50, 40)))]
#[case::split_mismatch_without_one_hour(BODY, 0, 100, Some((50, 0)))]
#[case::five_minute_prefix_with_one_hour_split(BODY, 0, 100, Some((0, 100)))]
#[case::one_hour_prefix_with_five_minute_split(ONE_HOUR_BODY, 0, 100, Some((100, 0)))]
#[case::negative_one_hour_split(BODY, 0, 100, Some((101, -1)))]
#[case::negative_five_minute_split(ONE_HOUR_BODY, 0, 499, Some((-1, 500)))]
fn parse_observed_cache_rejects_inconsistent_cache_counts(
    #[case] body: &str,
    #[case] cached: i64,
    #[case] creation: i64,
    #[case] split: Option<(i64, i64)>,
) {
    let url = Url::parse(NATIVE_URL).unwrap();
    let headers = headers();
    let wire = WireRequest {
        url: &url,
        headers: &headers,
        body: body.as_bytes(),
    };
    let observed = response(cached, creation, split);
    assert!(parse_observed_cache(&wire, &observed, "caller", "deployment").is_none());
}

#[test]
fn parse_observed_cache_sums_reads_and_creation() {
    let url = Url::parse(NATIVE_URL).unwrap();
    let headers = headers();
    let wire = WireRequest {
        url: &url,
        headers: &headers,
        body: BODY.as_bytes(),
    };
    let observed = parse_observed_cache(&wire, &response(100, 400, Some((400, 0))), "caller", "d");
    let found = observed.unwrap();
    assert_eq!(found.cached_tokens, 500);
    assert_eq!(found.cache_creation_tokens, 400);
}

#[test]
fn parse_observed_cache_accepts_a_creation_only_hit() {
    let url = Url::parse(NATIVE_URL).unwrap();
    let headers = headers();
    let wire = WireRequest {
        url: &url,
        headers: &headers,
        body: ONE_HOUR_BODY.as_bytes(),
    };
    let observed = parse_observed_cache(&wire, &response(0, 500, Some((0, 500))), "caller", "d");
    let found = observed.unwrap();
    assert_eq!(found.prefix.ttl_seconds, 3_600);
    assert_eq!(found.cached_tokens, 500);
    assert_eq!(found.cache_creation_tokens, 500);
}

#[test]
fn parse_observed_cache_accepts_the_full_prompt_budget() {
    let url = Url::parse(NATIVE_URL).unwrap();
    let headers = headers();
    let wire = WireRequest {
        url: &url,
        headers: &headers,
        body: BODY.as_bytes(),
    };
    let observed = parse_observed_cache(&wire, &response(1_000, 0, None), "caller", "d");
    let found = observed.unwrap();
    assert_eq!(found.cached_tokens, 1_000);
    assert_eq!(found.cache_creation_tokens, 0);
}

#[test]
fn parse_observed_cache_ignores_fields_outside_the_response_model() {
    let url = Url::parse(NATIVE_URL).unwrap();
    let headers = headers();
    let wire = WireRequest {
        url: &url,
        headers: &headers,
        body: BODY.as_bytes(),
    };
    let observed = json(
        r#"{"id":"msg_1","object":"chat.completion","model":"claude-3-5-sonnet","usage":{"prompt_tokens":1000,"completion_tokens":10,"prompt_tokens_details":{"cached_tokens":100,"cache_creation_tokens":0,"vendor_detail":true}},"choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":"hi"}}]}"#,
    );
    let found = parse_observed_cache(&wire, &observed, "caller", "deployment").unwrap();
    assert_eq!(found.cached_tokens, 100);
}

#[rstest]
#[case::no_choices(
    r#"{"model":"claude-3-5-sonnet","usage":{"prompt_tokens":1000,"prompt_tokens_details":{"cached_tokens":100}},"choices":[]}"#
)]
#[case::blank_finish_reason(
    r#"{"model":"claude-3-5-sonnet","usage":{"prompt_tokens":1000,"prompt_tokens_details":{"cached_tokens":100}},"choices":[{"finish_reason":""}]}"#
)]
#[case::missing_cache_details(
    r#"{"model":"claude-3-5-sonnet","usage":{"prompt_tokens":1000},"choices":[{"finish_reason":"stop"}]}"#
)]
#[case::negative_prompt_tokens(
    r#"{"model":"claude-3-5-sonnet","usage":{"prompt_tokens":-1,"prompt_tokens_details":{"cached_tokens":100}},"choices":[{"finish_reason":"stop"}]}"#
)]
#[case::missing_model(
    r#"{"usage":{"prompt_tokens":1000,"prompt_tokens_details":{"cached_tokens":100}},"choices":[{"finish_reason":"stop"}]}"#
)]
#[case::not_an_object("[]")]
fn parse_observed_cache_rejects_incomplete_responses(#[case] response: &str) {
    let url = Url::parse(NATIVE_URL).unwrap();
    let headers = headers();
    let wire = wire_request(&url, &headers, BODY.as_bytes());
    assert!(parse_observed_cache(&wire, &json(response), "caller", "deployment").is_none());
}

#[test]
fn parse_observed_cache_parser_maps_the_ported_evidence() {
    let url = Url::parse(NATIVE_URL).unwrap();
    let headers = headers();
    let wire = WireRequest {
        url: &url,
        headers: &headers,
        body: BODY.as_bytes(),
    };
    let parser = ObservedCacheParser;
    let carried = parser.parse(&wire, &hit(), "caller", "deployment").unwrap();
    let prefix = parse_prompt(BODY).unwrap();
    assert_eq!(carried.prefix.fingerprint, prefix.fingerprint);
    assert_eq!(carried.prefix.fingerprints, prefix.fingerprints);
    assert_eq!(carried.ttl_seconds, 300);
    assert_eq!(carried.cached_tokens, 100);
    assert_eq!(carried.cache_creation_tokens, 0);
    assert_eq!(
        carried.scope,
        cache_scope("caller", "deployment", API_KEY, "claude-3-5-sonnet")
    );
}

#[test]
fn replays_the_observed_cache_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/observed_cache_decoder");
    let outcomes: Vec<bool> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| {
            let bytes = fs::read(entry.unwrap().path()).unwrap();
            let url = Url::parse(NATIVE_URL).unwrap();
            let headers = headers();
            let wire = wire_request(&url, &headers, &bytes);
            parse_observed_cache(&wire, &hit(), "caller", "deployment").is_some()
        })
        .collect();
    assert!(outcomes.contains(&true));
    assert!(outcomes.contains(&false));
}
