#![cfg(test)]

use crate::cisco_verdict_enforcement::api_error::{ApiErrorDetail, ApiErrorOutcome};

#[test]
fn api_error_outcome_blocked_carries_the_detail_fields() {
    let outcome = ApiErrorOutcome::Blocked(ApiErrorDetail {
        error: "unavailable",
        message: "timed out".to_owned(),
        error_type: "timeout".to_owned(),
    });
    let ApiErrorOutcome::Blocked(detail) = outcome else {
        panic!("expected Blocked");
    };
    assert_eq!(detail.error, "unavailable");
    assert_eq!(detail.message, "timed out");
    assert_eq!(detail.error_type, "timeout");
}
