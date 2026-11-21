#![cfg(test)]

use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_models::decisions::Json;

use crate::limiter_records::RateLimitType;
use crate::post_call_settlement::header_merge::merge_rate_limit_headers;
use crate::post_call_settlement::tests::support::{headers, number, status};

#[test]
fn writes_a_remaining_and_limit_pair_for_one_status() {
    let merged = merge_rate_limit_headers(
        &AdditionalHeaders::new(),
        &[status("user", RateLimitType::Tokens, 1000, 750)],
    );
    assert_eq!(
        merged.get("x-ratelimit-user-remaining-tokens"),
        Some(&number(750))
    );
    assert_eq!(
        merged.get("x-ratelimit-user-limit-tokens"),
        Some(&number(1000))
    );
}

#[test]
fn keeps_the_headers_the_caller_already_held() {
    let existing = headers(&[
        ("x-request-id", Json::Text("abc".to_owned())),
        ("x-litellm-model", Json::Text("gpt-4".to_owned())),
    ]);
    let merged =
        merge_rate_limit_headers(&existing, &[status("user", RateLimitType::Requests, 10, 9)]);
    assert_eq!(
        merged.get("x-request-id"),
        Some(&Json::Text("abc".to_owned()))
    );
    assert_eq!(
        merged.get("x-litellm-model"),
        Some(&Json::Text("gpt-4".to_owned()))
    );
    assert_eq!(merged.len(), 4);
}

#[test]
fn leaves_the_input_map_untouched() {
    let existing = headers(&[("x-request-id", Json::Text("abc".to_owned()))]);
    let merged =
        merge_rate_limit_headers(&existing, &[status("user", RateLimitType::Requests, 10, 9)]);
    assert_eq!(existing.len(), 1);
    assert_eq!(existing.get("x-ratelimit-user-remaining-requests"), None);
    assert_eq!(merged.len(), 3);
}

#[test]
fn uses_each_status_own_descriptor_and_rate_limit_type() {
    let merged = merge_rate_limit_headers(
        &AdditionalHeaders::new(),
        &[
            status("api_key", RateLimitType::Requests, 60, 59),
            status("api_key", RateLimitType::Tokens, 5000, 4000),
            status("model", RateLimitType::MaxParallelRequests, 4, 3),
        ],
    );
    let mut keys: Vec<&str> = merged.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "x-ratelimit-api_key-limit-requests",
            "x-ratelimit-api_key-limit-tokens",
            "x-ratelimit-api_key-remaining-requests",
            "x-ratelimit-api_key-remaining-tokens",
            "x-ratelimit-model-limit-max_parallel_requests",
            "x-ratelimit-model-remaining-max_parallel_requests",
        ]
    );
    assert_eq!(
        merged.get("x-ratelimit-api_key-remaining-tokens"),
        Some(&number(4000))
    );
    assert_eq!(
        merged.get("x-ratelimit-model-limit-max_parallel_requests"),
        Some(&number(4))
    );
}

#[test]
fn overwrites_a_stale_generated_header() {
    let existing = headers(&[(
        "x-ratelimit-user-remaining-tokens",
        Json::Text("stale".to_owned()),
    )]);
    let merged = merge_rate_limit_headers(
        &existing,
        &[status("user", RateLimitType::Tokens, 1000, 750)],
    );
    assert_eq!(
        merged.get("x-ratelimit-user-remaining-tokens"),
        Some(&number(750))
    );
    assert_eq!(merged.len(), 2);
}

#[test]
fn writes_nothing_for_an_empty_status_list() {
    let existing = headers(&[("x-request-id", Json::Text("abc".to_owned()))]);
    let merged = merge_rate_limit_headers(&existing, &[]);
    assert_eq!(merged, existing);
}
