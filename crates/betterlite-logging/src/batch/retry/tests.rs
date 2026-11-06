#![cfg(test)]

use rstest::rstest;

use super::{RequeuePolicy, is_retryable_status, undelivered_after_http_error};

const EVENTS: [&str; 3] = ["e0", "e1", "e2"];

#[rstest]
#[case::informational(100, true)]
#[case::success(200, true)]
#[case::redirection(399, true)]
#[case::bad_request(400, false)]
#[case::request_timeout(408, true)]
#[case::payload_too_large(413, false)]
#[case::too_many_requests(429, true)]
#[case::client_error_tail(499, false)]
#[case::server_error(500, true)]
#[case::server_error_tail(599, true)]
fn batch_delivery_behavior_tests_behavior(#[case] status: u16, #[case] retryable: bool) {
    assert_eq!(is_retryable_status(status), retryable);
    assert!(RequeuePolicy::RequeueAll.retains(status));
    assert_eq!(RequeuePolicy::Undelivered.retains(status), retryable);
    let undelivered = undelivered_after_http_error(&EVENTS, status);
    if retryable {
        assert_eq!(undelivered, EVENTS);
    } else {
        assert!(undelivered.is_empty());
    }
}
