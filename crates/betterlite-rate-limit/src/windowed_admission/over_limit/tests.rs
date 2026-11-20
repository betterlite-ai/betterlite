#![cfg(test)]

use crate::limiter_records::{
    CacheCounterValue, RateLimitResponse, RateLimitStatus, RateLimitType, WindowKeyMetadata,
};
use crate::windowed_admission::WindowedAdmissionError;
use crate::windowed_admission::over_limit::is_cache_list_over_limit;
use crate::windowed_admission::tests::support::{int, key_list, metadata, metadata_map, text};

fn window_key() -> String {
    "{api-key:k1}:window".to_owned()
}

fn requests_key() -> String {
    "{api-key:k1}:requests".to_owned()
}

fn tokens_key() -> String {
    "{api-key:k1}:tokens".to_owned()
}

fn check(
    keys: &[&str],
    values: &[Option<CacheCounterValue>],
    metadata: &[(&str, WindowKeyMetadata)],
    read_only: bool,
) -> Result<RateLimitResponse, WindowedAdmissionError> {
    is_cache_list_over_limit(
        &key_list(keys),
        &values.to_vec(),
        &metadata_map(metadata),
        read_only,
    )
}

#[test]
fn a_counter_over_its_limit_is_over_limit() {
    let window = window_key();
    let requests = requests_key();
    let keys = [window.as_str(), requests.as_str()];
    let values = [Some(text("1700000000")), Some(int(11))];
    let metadata = [(window.as_str(), metadata(Some(10), None, 60))];
    let response = check(&keys, &values, &metadata, false).unwrap();
    assert_eq!(response.overall_code, "OVER_LIMIT");
    assert_eq!(
        response.statuses,
        vec![RateLimitStatus {
            code: "OVER_LIMIT".to_owned(),
            current_limit: 10,
            limit_remaining: -1,
            rate_limit_type: RateLimitType::Requests,
            descriptor_key: "api-key".to_owned(),
            descriptor_value: Some("k1".to_owned()),
        }]
    );
    assert!(response.reservation_windows.is_none());
}

#[test]
fn a_counter_at_its_limit_is_ok_when_the_check_incremented() {
    let window = window_key();
    let requests = requests_key();
    let keys = [window.as_str(), requests.as_str()];
    let values = [Some(text("1700000000")), Some(int(10))];
    let metadata = [(window.as_str(), metadata(Some(10), None, 60))];
    let response = check(&keys, &values, &metadata, false).unwrap();
    assert_eq!(response.overall_code, "OK");
    assert_eq!(response.statuses[0].limit_remaining, 0);
}

#[test]
fn an_absent_counter_is_ok_with_the_full_limit_remaining() {
    let window = window_key();
    let requests = requests_key();
    let keys = [window.as_str(), requests.as_str()];
    let values = [Some(text("1700000000")), None];
    let metadata = [(window.as_str(), metadata(Some(10), None, 60))];
    let response = check(&keys, &values, &metadata, false).unwrap();
    assert_eq!(response.overall_code, "OK");
    assert_eq!(response.statuses[0].current_limit, 10);
    assert_eq!(response.statuses[0].limit_remaining, 10);
}

#[test]
fn read_only_counts_one_pending_request() {
    let window = window_key();
    let requests = requests_key();
    let keys = [window.as_str(), requests.as_str()];
    let values = [Some(text("1700000000")), Some(int(10))];
    let metadata = [(window.as_str(), metadata(Some(10), None, 60))];
    let read_only = check(&keys, &values, &metadata, true).unwrap();
    assert_eq!(read_only.overall_code, "OVER_LIMIT");
    assert_eq!(read_only.statuses[0].code, "OVER_LIMIT");
    let incrementing = check(&keys, &values, &metadata, false).unwrap();
    assert_eq!(incrementing.overall_code, "OK");
}

#[test]
fn a_float_counter_is_cut_toward_zero() {
    let window = window_key();
    let requests = requests_key();
    let keys = [window.as_str(), requests.as_str()];
    let values = [
        Some(text("1700000000")),
        Some(CacheCounterValue::Float(9.9)),
    ];
    let metadata = [(window.as_str(), metadata(Some(10), None, 60))];
    let response = check(&keys, &values, &metadata, false).unwrap();
    assert_eq!(response.overall_code, "OK");
    assert_eq!(response.statuses[0].limit_remaining, 1);
}

#[test]
fn counter_text_that_is_not_a_whole_number_is_an_error() {
    let window = window_key();
    let requests = requests_key();
    let keys = [window.as_str(), requests.as_str()];
    let values = [Some(text("1700000000")), Some(text("many"))];
    let metadata = [(window.as_str(), metadata(Some(10), None, 60))];
    let error = check(&keys, &values, &metadata, false).unwrap_err();
    assert!(matches!(error, WindowedAdmissionError::Counter(_)));
}

#[test]
fn a_key_that_is_not_a_requests_or_tokens_counter_gives_no_status() {
    let window = window_key();
    let other = "{api-key:k1}:other".to_owned();
    let keys = [window.as_str(), other.as_str()];
    let values = [Some(text("1700000000")), Some(int(11))];
    let metadata = [(window.as_str(), metadata(Some(10), Some(10), 60))];
    let response = check(&keys, &values, &metadata, false).unwrap();
    assert_eq!(response.overall_code, "OK");
    assert!(response.statuses.is_empty());
}

#[test]
fn a_limit_that_is_none_gives_no_status() {
    let window = window_key();
    let tokens = tokens_key();
    let keys = [window.as_str(), tokens.as_str()];
    let values = [Some(text("1700000000")), Some(int(11))];
    let metadata = [(window.as_str(), metadata(Some(10), None, 60))];
    let response = check(&keys, &values, &metadata, false).unwrap();
    assert!(response.statuses.is_empty());
}

#[test]
fn both_limits_report_one_status_each_in_key_order() {
    let window = window_key();
    let requests = requests_key();
    let tokens = tokens_key();
    let keys = [
        window.as_str(),
        requests.as_str(),
        window.as_str(),
        tokens.as_str(),
    ];
    let values = [
        Some(text("1700000000")),
        Some(int(5)),
        Some(text("1700000000")),
        Some(int(25)),
    ];
    let metadata = [(window.as_str(), metadata(Some(10), Some(20), 60))];
    let response = check(&keys, &values, &metadata, false).unwrap();
    assert_eq!(response.overall_code, "OVER_LIMIT");
    assert_eq!(response.statuses.len(), 2);
    assert_eq!(
        response.statuses[0].rate_limit_type,
        RateLimitType::Requests
    );
    assert_eq!(response.statuses[0].code, "OK");
    assert_eq!(response.statuses[0].limit_remaining, 5);
    assert_eq!(response.statuses[1].rate_limit_type, RateLimitType::Tokens);
    assert_eq!(response.statuses[1].code, "OVER_LIMIT");
    assert_eq!(response.statuses[1].limit_remaining, -5);
}

#[test]
fn keys_and_values_of_different_lengths_are_an_error() {
    let window = window_key();
    let requests = requests_key();
    let tokens = tokens_key();
    let keys = [
        window.as_str(),
        requests.as_str(),
        window.as_str(),
        tokens.as_str(),
    ];
    let values = [Some(text("1700000000")), Some(int(1))];
    let metadata = [(window.as_str(), metadata(Some(10), Some(10), 60))];
    let error = check(&keys, &values, &metadata, false).unwrap_err();
    assert!(matches!(
        error,
        WindowedAdmissionError::WindowPairCount { keys: 4, values: 2 }
    ));
}

#[test]
fn an_odd_number_of_values_is_an_error() {
    let window = window_key();
    let requests = requests_key();
    let keys = [window.as_str(), requests.as_str(), window.as_str()];
    let values = [
        Some(text("1700000000")),
        Some(int(1)),
        Some(text("1700000000")),
    ];
    let metadata = [(window.as_str(), metadata(Some(10), None, 60))];
    let error = check(&keys, &values, &metadata, false).unwrap_err();
    assert!(matches!(
        error,
        WindowedAdmissionError::WindowPairCount { keys: 3, values: 3 }
    ));
}

#[test]
fn a_window_key_without_metadata_is_an_error() {
    let window = window_key();
    let requests = requests_key();
    let keys = [window.as_str(), requests.as_str()];
    let values = [Some(text("1700000000")), Some(int(1))];
    let error = check(&keys, &values, &[], false).unwrap_err();
    assert!(matches!(
        error,
        WindowedAdmissionError::MissingWindowMetadata { .. }
    ));
}
