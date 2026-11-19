#![cfg(test)]

use rstest::rstest;

use crate::concurrency_gauges::tests::support::gauge;
use crate::limiter_records::{RateLimitStatus, RateLimitType};

use super::{gauge_status, over_limit_response};

#[test]
fn a_gauge_status_names_the_parallel_limit_and_the_remaining_room() {
    let status = gauge_status(&gauge("g1", 5), 2, "OK");
    assert_eq!(
        status,
        RateLimitStatus {
            code: "OK".to_owned(),
            current_limit: 5,
            limit_remaining: 3,
            rate_limit_type: RateLimitType::MaxParallelRequests,
            descriptor_key: "api-key".to_owned(),
            descriptor_value: None,
        }
    );
}

#[rstest]
#[case::one_below_the_limit(4, 1)]
#[case::at_the_limit(5, 0)]
#[case::over_the_limit(7, 0)]
fn remaining_capacity_never_goes_below_zero(#[case] in_flight: i64, #[case] remaining: i64) {
    let status = gauge_status(&gauge("g1", 5), in_flight, "OK");
    assert_eq!(status.limit_remaining, remaining);
}

#[rstest]
#[case::ok("OK")]
#[case::over_limit("OVER_LIMIT")]
fn the_status_code_is_the_one_the_caller_asks_for(#[case] code: &str) {
    let status = gauge_status(&gauge("g1", 5), 1, code);
    assert_eq!(status.code, code);
}

#[test]
fn an_over_limit_response_holds_only_the_rejecting_gauge() {
    let response = over_limit_response(&gauge("g2", 3), 3);
    assert_eq!(response.overall_code, "OVER_LIMIT");
    assert!(response.reservation_windows.is_none());
    assert_eq!(response.statuses.len(), 1);
    let status = response.statuses.first().expect("the rejecting status");
    assert_eq!(status.code, "OVER_LIMIT");
    assert_eq!(status.current_limit, 3);
    assert_eq!(status.limit_remaining, 0);
    assert_eq!(status.rate_limit_type, RateLimitType::MaxParallelRequests);
    assert_eq!(status.descriptor_key, "api-key");
    assert!(status.descriptor_value.is_none());
}
