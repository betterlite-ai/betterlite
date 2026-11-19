#![cfg(test)]

use super::GatewayRequestCounts;

#[test]
fn the_default_counts_are_zero() {
    let counts = GatewayRequestCounts::default();
    assert_eq!(counts.successful_requests, 0);
    assert_eq!(counts.failed_requests, 0);
}

#[test]
fn a_success_adds_one_to_the_successful_count() {
    let counts = GatewayRequestCounts {
        successful_requests: 5,
        failed_requests: 2,
    }
    .plus(true);
    assert_eq!(counts.successful_requests, 6);
    assert_eq!(counts.failed_requests, 2);
}

#[test]
fn a_failure_adds_one_to_the_failed_count() {
    let counts = GatewayRequestCounts {
        successful_requests: 5,
        failed_requests: 2,
    }
    .plus(false);
    assert_eq!(counts.successful_requests, 5);
    assert_eq!(counts.failed_requests, 3);
}

#[test]
fn a_default_count_plus_one_success_is_one_success() {
    assert_eq!(
        GatewayRequestCounts::default().plus(true),
        GatewayRequestCounts {
            successful_requests: 1,
            failed_requests: 0,
        }
    );
}

#[test]
fn a_default_count_plus_one_failure_is_one_failure() {
    assert_eq!(
        GatewayRequestCounts::default().plus(false),
        GatewayRequestCounts {
            successful_requests: 0,
            failed_requests: 1,
        }
    );
}

#[test]
fn a_success_at_the_integer_limit_stays_at_the_limit() {
    let counts = GatewayRequestCounts {
        successful_requests: i64::MAX,
        failed_requests: 0,
    }
    .plus(true);
    assert_eq!(counts.successful_requests, i64::MAX);
}

#[test]
fn a_failure_at_the_integer_limit_stays_at_the_limit() {
    let counts = GatewayRequestCounts {
        successful_requests: 0,
        failed_requests: i64::MAX,
    }
    .plus(false);
    assert_eq!(counts.failed_requests, i64::MAX);
}

#[test]
fn fold_sums_both_fields() {
    let left = GatewayRequestCounts {
        successful_requests: 5,
        failed_requests: 2,
    };
    let right = GatewayRequestCounts {
        successful_requests: 3,
        failed_requests: 7,
    };
    assert_eq!(
        left.fold(right),
        GatewayRequestCounts {
            successful_requests: 8,
            failed_requests: 9,
        }
    );
}

#[test]
fn fold_saturates_both_fields() {
    let left = GatewayRequestCounts {
        successful_requests: i64::MAX,
        failed_requests: i64::MAX,
    };
    let right = GatewayRequestCounts {
        successful_requests: 1,
        failed_requests: 1,
    };
    assert_eq!(
        left.fold(right),
        GatewayRequestCounts {
            successful_requests: i64::MAX,
            failed_requests: i64::MAX,
        }
    );
}
