#![no_main]

use betterlite_anthropic::observation_parsing::parse_observed_cache;
use betterlite_logging::cache_observer::WireRequest;
use betterlite_models::decisions::Json;
use http::{HeaderMap, HeaderValue};
use libfuzzer_sys::fuzz_target;
use url::Url;

const RECIPIENT: &str = "https://api.anthropic.com/v1/messages";
const RESPONSE: &str = r#"{"model":"claude-3-5-sonnet","usage":{"prompt_tokens":1000,"prompt_tokens_details":{"cached_tokens":100,"cache_creation_tokens":0}},"choices":[{"finish_reason":"stop"}]}"#;

fuzz_target!(|body: &[u8]| {
    let Ok(url) = Url::parse(RECIPIENT) else {
        return;
    };
    let Ok(response) = serde_json::from_str::<Json>(RESPONSE) else {
        return;
    };
    let mut headers = HeaderMap::new();
    headers.insert("host", HeaderValue::from_static("api.anthropic.com"));
    headers.insert("content-type", HeaderValue::from_static("application/json"));
    headers.insert("x-api-key", HeaderValue::from_static("sk-ant-api03-key"));
    headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
    let wire = WireRequest {
        url: &url,
        headers: &headers,
        body,
    };
    let _evidence = parse_observed_cache(&wire, &response, "caller", "deployment");
});
