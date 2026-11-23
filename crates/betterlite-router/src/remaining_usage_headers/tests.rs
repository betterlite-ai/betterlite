#![cfg(test)]

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_models::decisions::Json;
use serde_json::Number;

use super::{RemainingUsage, apply_remaining_usage_headers, build_io_token_rate_limit_headers};
use crate::io_token_reservations::limits::IoLimits;

const INPUT_LIMIT: &str = "x-ratelimit-limit-input-tokens";
const INPUT_REMAINING: &str = "x-ratelimit-remaining-input-tokens";
const INPUT_RESET: &str = "x-ratelimit-reset-input-tokens";
const OUTPUT_LIMIT: &str = "x-ratelimit-limit-output-tokens";
const OUTPUT_REMAINING: &str = "x-ratelimit-remaining-output-tokens";
const OUTPUT_RESET: &str = "x-ratelimit-reset-output-tokens";

#[test]
fn io_headers_both_limits_floor_input_remaining() {
    let moment = UNIX_EPOCH.checked_add(Duration::from_secs(60)).unwrap();
    let headers = build_io_token_rate_limit_headers(
        IoLimits {
            itpm: Some(100),
            otpm: Some(50),
        },
        Some(120),
        None,
        moment,
    );
    let actual: Vec<(String, i64)> = headers
        .iter()
        .map(|(name, value)| (name.clone(), *value))
        .collect();
    assert_eq!(
        actual,
        vec![
            (INPUT_LIMIT.to_owned(), 100),
            (INPUT_REMAINING.to_owned(), 0),
            (INPUT_RESET.to_owned(), 60),
            (OUTPUT_LIMIT.to_owned(), 50),
            (OUTPUT_REMAINING.to_owned(), 50),
            (OUTPUT_RESET.to_owned(), 60),
        ]
    );
}

#[test]
fn io_headers_input_only() {
    let moment = UNIX_EPOCH.checked_add(Duration::from_secs(90)).unwrap();
    let headers = build_io_token_rate_limit_headers(
        IoLimits {
            itpm: Some(10),
            otpm: None,
        },
        Some(4),
        Some(9),
        moment,
    );
    let actual: Vec<(String, i64)> = headers
        .iter()
        .map(|(name, value)| (name.clone(), *value))
        .collect();
    assert_eq!(
        actual,
        vec![
            (INPUT_LIMIT.to_owned(), 10),
            (INPUT_REMAINING.to_owned(), 6),
            (INPUT_RESET.to_owned(), 30),
        ]
    );
}

#[test]
fn io_headers_output_only() {
    let moment = UNIX_EPOCH.checked_add(Duration::from_secs(61)).unwrap();
    let headers = build_io_token_rate_limit_headers(
        IoLimits {
            itpm: None,
            otpm: Some(7),
        },
        Some(100),
        Some(2),
        moment,
    );
    let actual: Vec<(String, i64)> = headers
        .iter()
        .map(|(name, value)| (name.clone(), *value))
        .collect();
    assert_eq!(
        actual,
        vec![
            (OUTPUT_LIMIT.to_owned(), 7),
            (OUTPUT_REMAINING.to_owned(), 5),
            (OUTPUT_RESET.to_owned(), 59),
        ]
    );
}

#[test]
fn io_headers_no_limits() {
    let moment = UNIX_EPOCH.checked_add(Duration::from_secs(60)).unwrap();
    let headers = build_io_token_rate_limit_headers(
        IoLimits {
            itpm: None,
            otpm: None,
        },
        Some(3),
        Some(4),
        moment,
    );
    assert!(headers.is_empty());
}

#[test]
fn io_headers_zero_limit() {
    let moment = UNIX_EPOCH.checked_add(Duration::from_secs(60)).unwrap();
    let headers = build_io_token_rate_limit_headers(
        IoLimits {
            itpm: Some(0),
            otpm: Some(0),
        },
        Some(5),
        None,
        moment,
    );
    let actual: Vec<(String, i64)> = headers
        .iter()
        .map(|(name, value)| (name.clone(), *value))
        .collect();
    assert_eq!(
        actual,
        vec![
            (INPUT_LIMIT.to_owned(), 0),
            (INPUT_REMAINING.to_owned(), 0),
            (INPUT_RESET.to_owned(), 60),
            (OUTPUT_LIMIT.to_owned(), 0),
            (OUTPUT_REMAINING.to_owned(), 0),
            (OUTPUT_RESET.to_owned(), 60),
        ]
    );
}

#[test]
fn io_headers_share_one_reset_at_second_zero() {
    let moment = UNIX_EPOCH.checked_add(Duration::from_secs(60)).unwrap();
    let headers = build_io_token_rate_limit_headers(
        IoLimits {
            itpm: Some(10),
            otpm: Some(10),
        },
        Some(1),
        Some(1),
        moment,
    );
    assert_eq!(headers[INPUT_RESET], 60);
    assert_eq!(headers[OUTPUT_RESET], 60);
}

#[test]
fn io_headers_share_one_reset_at_second_fifty_nine() {
    let moment = UNIX_EPOCH.checked_add(Duration::from_secs(119)).unwrap();
    let headers = build_io_token_rate_limit_headers(
        IoLimits {
            itpm: Some(10),
            otpm: Some(10),
        },
        Some(1),
        Some(1),
        moment,
    );
    assert_eq!(headers[INPUT_RESET], 1);
    assert_eq!(headers[OUTPUT_RESET], 1);
}

#[test]
fn io_headers_follow_litellm_order() {
    let moment = UNIX_EPOCH.checked_add(Duration::from_secs(60)).unwrap();
    let headers = build_io_token_rate_limit_headers(
        IoLimits {
            itpm: Some(10),
            otpm: Some(20),
        },
        Some(1),
        Some(2),
        moment,
    );
    let names: Vec<&str> = headers.keys().map(String::as_str).collect();
    assert_eq!(
        names,
        vec![
            INPUT_LIMIT,
            INPUT_REMAINING,
            INPUT_RESET,
            OUTPUT_LIMIT,
            OUTPUT_REMAINING,
            OUTPUT_RESET,
        ]
    );
}

#[test]
fn io_headers_extreme_limits_saturate() {
    let moment = UNIX_EPOCH.checked_add(Duration::from_secs(60)).unwrap();
    let headers = build_io_token_rate_limit_headers(
        IoLimits {
            itpm: Some(i64::MIN),
            otpm: Some(i64::MAX),
        },
        Some(1),
        Some(-1),
        moment,
    );
    let actual: Vec<(String, i64)> = headers
        .iter()
        .map(|(name, value)| (name.clone(), *value))
        .collect();
    assert_eq!(
        actual,
        vec![
            (INPUT_LIMIT.to_owned(), i64::MIN),
            (INPUT_REMAINING.to_owned(), 0),
            (INPUT_RESET.to_owned(), 60),
            (OUTPUT_LIMIT.to_owned(), i64::MAX),
            (OUTPUT_REMAINING.to_owned(), i64::MAX),
            (OUTPUT_RESET.to_owned(), 60),
        ]
    );
}

#[test]
fn merge_keeps_present_value() {
    let mut additional = AdditionalHeaders::new();
    additional.insert(INPUT_LIMIT.to_owned(), Json::Number(Number::from(9_i64)));
    let mut remaining = RemainingUsage::new();
    remaining.insert(INPUT_LIMIT.to_owned(), 100);
    remaining.insert(INPUT_REMAINING.to_owned(), 42);
    apply_remaining_usage_headers(&mut additional, &remaining);
    let actual: Vec<(String, Json)> = additional
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    assert_eq!(
        actual,
        vec![
            (INPUT_LIMIT.to_owned(), Json::Number(Number::from(9_i64))),
            (
                INPUT_REMAINING.to_owned(),
                Json::Number(Number::from(42_i64))
            ),
        ]
    );
}

#[test]
fn merge_keeps_present_null() {
    let mut additional = AdditionalHeaders::new();
    additional.insert(INPUT_RESET.to_owned(), Json::Null);
    let mut remaining = RemainingUsage::new();
    remaining.insert(INPUT_RESET.to_owned(), 60);
    apply_remaining_usage_headers(&mut additional, &remaining);
    let actual: Vec<(String, Json)> = additional
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    assert_eq!(actual, vec![(INPUT_RESET.to_owned(), Json::Null)]);
}

#[test]
fn merge_inserts_absent_keys_in_remaining_order() {
    let mut additional = AdditionalHeaders::new();
    additional.insert(OUTPUT_LIMIT.to_owned(), Json::Text("kept".to_owned()));
    let mut remaining = RemainingUsage::new();
    remaining.insert(INPUT_LIMIT.to_owned(), 100);
    remaining.insert(INPUT_REMAINING.to_owned(), 42);
    remaining.insert(OUTPUT_LIMIT.to_owned(), 50);
    remaining.insert(INPUT_RESET.to_owned(), 60);
    apply_remaining_usage_headers(&mut additional, &remaining);
    let actual: Vec<(String, Json)> = additional
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    assert_eq!(
        actual,
        vec![
            (OUTPUT_LIMIT.to_owned(), Json::Text("kept".to_owned())),
            (INPUT_LIMIT.to_owned(), Json::Number(Number::from(100_i64))),
            (
                INPUT_REMAINING.to_owned(),
                Json::Number(Number::from(42_i64))
            ),
            (INPUT_RESET.to_owned(), Json::Number(Number::from(60_i64))),
        ]
    );
}
