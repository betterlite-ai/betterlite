#![cfg(test)]

use super::fold_counts;
use crate::gateway_requests::GatewayRequestKey;
use crate::gateway_requests::gateway_request_counts::GatewayRequestCounts;

fn key(date: &str, category: &str, route: &str) -> GatewayRequestKey {
    GatewayRequestKey {
        date: date.to_owned(),
        category: category.to_owned(),
        route: route.to_owned(),
    }
}

fn counts(successful_requests: i64, failed_requests: i64) -> GatewayRequestCounts {
    GatewayRequestCounts {
        successful_requests,
        failed_requests,
    }
}

#[test]
fn fold_sums_the_counts_of_one_key() {
    let entries = vec![
        (key("2026-01-05", "api", "/v1/chat"), counts(2, 1)),
        (key("2026-01-05", "api", "/v1/chat"), counts(3, 4)),
    ];
    let folded = fold_counts(entries);
    assert_eq!(
        folded.get(&key("2026-01-05", "api", "/v1/chat")).copied(),
        Some(counts(5, 5))
    );
}

#[test]
fn fold_keeps_distinct_keys_apart() {
    let entries = vec![
        (key("2026-01-05", "api", "/v1/chat"), counts(2, 1)),
        (key("2026-01-06", "api", "/v1/chat"), counts(3, 0)),
        (key("2026-01-05", "passthrough", "/v1/chat"), counts(0, 4)),
    ];
    let folded = fold_counts(entries);
    assert_eq!(folded.len(), 3);
    assert_eq!(
        folded.get(&key("2026-01-05", "api", "/v1/chat")).copied(),
        Some(counts(2, 1))
    );
    assert_eq!(
        folded.get(&key("2026-01-06", "api", "/v1/chat")).copied(),
        Some(counts(3, 0))
    );
    assert_eq!(
        folded
            .get(&key("2026-01-05", "passthrough", "/v1/chat"))
            .copied(),
        Some(counts(0, 4))
    );
}

#[test]
fn fold_of_no_entries_is_an_empty_snapshot() {
    assert!(fold_counts(Vec::new()).is_empty());
}

#[test]
fn fold_stays_bounded_by_the_unique_keys() {
    let entries = vec![
        (key("2026-01-05", "api", "/v1/chat"), counts(1, 0)),
        (key("2026-01-05", "api", "/v1/chat"), counts(1, 0)),
        (key("2026-01-05", "api", "/v1/chat"), counts(1, 0)),
    ];
    let folded = fold_counts(entries);
    assert_eq!(folded.len(), 1);
    assert_eq!(
        folded.get(&key("2026-01-05", "api", "/v1/chat")).copied(),
        Some(counts(3, 0))
    );
}
