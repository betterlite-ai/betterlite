use rstest::rstest;

use crate::prometheus::failure::exception::LabelException;
use crate::prometheus::failure::invalid_key::{
    is_invalid_api_key_request, should_skip_metrics_for_invalid_key,
};
use crate::prometheus::failure::tests::support::{capture_logs, with_message, with_status};

#[test]
fn a_401_status_is_an_invalid_key_request() {
    assert!(is_invalid_api_key_request(Some(401), None));
}

#[test]
fn another_status_with_an_ordinary_message_is_not_an_invalid_key_request() {
    let exception = with_message(LabelException::default(), "bad request");
    assert!(!is_invalid_api_key_request(Some(400), Some(&exception)));
}

#[rstest]
#[case::virtual_key_expected("Virtual Key expected")]
#[case::missing_sk_prefix("The key is expected to start with 'sk-'")]
#[case::authentication_error("Authentication error: no key")]
#[case::invalid_api_key("Invalid API Key provided")]
#[case::api_key_not_valid("The API key not valid here")]
fn an_auth_message_is_an_invalid_key_request(#[case] message: &str) {
    let exception = with_message(LabelException::default(), message);
    assert!(is_invalid_api_key_request(None, Some(&exception)));
}

#[test]
fn a_message_without_an_auth_pattern_is_not_an_invalid_key_request() {
    let exception = with_message(LabelException::default(), "upstream timed out");
    assert!(!is_invalid_api_key_request(None, Some(&exception)));
}

#[test]
fn an_exception_without_a_message_is_not_an_invalid_key_request() {
    assert!(!is_invalid_api_key_request(
        None,
        Some(&LabelException::default())
    ));
}

#[test]
fn the_invalid_key_filter_skips_a_401() {
    assert!(should_skip_metrics_for_invalid_key(Some("401"), None, None));
}

#[test]
fn the_invalid_key_filter_reads_the_status_from_the_direct_exception() {
    let exception = with_status(LabelException::default(), Some("401"), None);
    assert!(should_skip_metrics_for_invalid_key(
        None,
        Some(&exception),
        None
    ));
}

#[test]
fn the_invalid_key_filter_reads_the_status_from_the_kwargs_exception() {
    let from_kwargs = with_status(LabelException::default(), Some("401"), None);
    assert!(should_skip_metrics_for_invalid_key(
        None,
        None,
        Some(&from_kwargs)
    ));
}

#[test]
fn the_invalid_key_filter_reads_the_message_from_the_direct_exception() {
    let exception = with_message(LabelException::default(), "invalid api key");
    assert!(should_skip_metrics_for_invalid_key(
        Some("400"),
        Some(&exception),
        None
    ));
}

#[test]
fn the_invalid_key_filter_reads_the_message_from_the_kwargs_exception() {
    let from_kwargs = with_message(LabelException::default(), "invalid api key");
    assert!(should_skip_metrics_for_invalid_key(
        Some("400"),
        None,
        Some(&from_kwargs)
    ));
}

#[test]
fn the_direct_exception_decides_before_the_kwargs_one() {
    let exception = with_message(LabelException::default(), "bad request");
    let from_kwargs = with_message(LabelException::default(), "invalid api key");
    assert!(!should_skip_metrics_for_invalid_key(
        Some("400"),
        Some(&exception),
        Some(&from_kwargs)
    ));
}

#[test]
fn the_invalid_key_filter_keeps_a_request_that_is_not_an_invalid_key() {
    assert!(!should_skip_metrics_for_invalid_key(
        Some("500"),
        None,
        None
    ));
}

#[test]
fn a_skipped_request_is_logged_at_debug() {
    let (logs, guard) = capture_logs();
    let skipped = should_skip_metrics_for_invalid_key(Some("401"), None, None);
    drop(guard);
    assert!(skipped);
    let text = logs.text();
    assert!(text.contains("Skipping Prometheus metrics for invalid API key request"));
    assert!(text.contains("status_code=Some(401)"));
}

#[test]
fn a_kept_request_is_not_logged() {
    let (logs, guard) = capture_logs();
    let skipped = should_skip_metrics_for_invalid_key(Some("500"), None, None);
    drop(guard);
    assert!(!skipped);
    assert!(!logs.text().contains("Skipping Prometheus metrics"));
}
