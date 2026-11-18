#![cfg(test)]

use std::sync::Arc;

use chrono::DateTime;
use chrono::TimeZone as _;
use chrono::Utc;
use rstest::rstest;

use super::GatewayRequestAccumulator;
use crate::gateway_request_tracking::GATEWAY_REQUEST_DATE_FORMAT;
use crate::gateway_request_tracking::GatewayRequestClock;
use crate::gateway_requests::GatewayRequestKey;
use crate::gateway_requests::GatewayRequestSnapshot;
use crate::gateway_requests::gateway_request_counts::GatewayRequestCounts;

struct FixedClock(DateTime<Utc>);

impl GatewayRequestClock for FixedClock {
    fn utc_now(&self) -> DateTime<Utc> {
        self.0
    }
}

fn accumulator_at(year: i32, month: u32, day: u32) -> GatewayRequestAccumulator {
    let clock = FixedClock(
        Utc.with_ymd_and_hms(year, month, day, 12, 0, 0)
            .single()
            .expect("a valid UTC instant"),
    );
    GatewayRequestAccumulator::with_clock(Arc::new(clock))
}

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
fn the_accumulator_starts_empty() {
    assert!(accumulator_at(2026, 1, 5).drain().is_empty());
}

#[rstest]
#[case::informational(199, false)]
#[case::success_minimum(200, true)]
#[case::success_below_maximum(298, true)]
#[case::success_maximum(299, true)]
#[case::redirect(300, false)]
#[case::client_error(404, false)]
#[case::server_error(500, false)]
fn record_counts_by_the_success_status_range(#[case] status_code: u16, #[case] succeeded: bool) {
    let accumulator = accumulator_at(2026, 1, 5);
    accumulator.record("api", "/v1/chat/completions", status_code);
    let snapshot = accumulator.drain();
    let expected = if succeeded {
        counts(1, 0)
    } else {
        counts(0, 1)
    };
    assert_eq!(
        snapshot
            .get(&key("2026-01-05", "api", "/v1/chat/completions"))
            .copied(),
        Some(expected)
    );
}

#[test]
fn record_keys_by_the_utc_date_of_the_clock() {
    let accumulator = accumulator_at(2026, 9, 4);
    accumulator.record("api", "/v1/chat/completions", 200);
    let keys: Vec<GatewayRequestKey> = accumulator.drain().keys().cloned().collect();
    assert_eq!(keys, vec![key("2026-09-04", "api", "/v1/chat/completions")]);
}

#[test]
fn the_default_accumulator_keys_by_the_current_utc_date() {
    let accumulator = GatewayRequestAccumulator::new();
    let before = Utc::now().format(GATEWAY_REQUEST_DATE_FORMAT).to_string();
    accumulator.record("api", "/v1/chat/completions", 200);
    let after = Utc::now().format(GATEWAY_REQUEST_DATE_FORMAT).to_string();
    let dates: Vec<String> = accumulator
        .drain()
        .keys()
        .map(|key| key.date.clone())
        .collect();
    assert_eq!(dates.len(), 1);
    assert!(dates[0] == before || dates[0] == after);
}

#[test]
fn record_accumulates_repeated_requests() {
    let accumulator = accumulator_at(2026, 1, 5);
    accumulator.record("api", "/v1/chat/completions", 200);
    accumulator.record("api", "/v1/chat/completions", 201);
    accumulator.record("api", "/v1/chat/completions", 500);
    assert_eq!(
        accumulator
            .drain()
            .get(&key("2026-01-05", "api", "/v1/chat/completions"))
            .copied(),
        Some(counts(2, 1))
    );
}

#[test]
fn record_keeps_categories_and_routes_apart() {
    let accumulator = accumulator_at(2026, 1, 5);
    accumulator.record("api", "/v1/chat/completions", 200);
    accumulator.record("passthrough", "/v1/chat/completions", 500);
    accumulator.record("api", "/v1/embeddings", 200);
    let snapshot = accumulator.drain();
    assert_eq!(snapshot.len(), 3);
    assert_eq!(
        snapshot
            .get(&key("2026-01-05", "passthrough", "/v1/chat/completions"))
            .copied(),
        Some(counts(0, 1))
    );
    assert_eq!(
        snapshot
            .get(&key("2026-01-05", "api", "/v1/embeddings"))
            .copied(),
        Some(counts(1, 0))
    );
}

#[test]
fn drain_returns_the_counts_and_leaves_an_empty_map() {
    let accumulator = accumulator_at(2026, 1, 5);
    accumulator.record("api", "/v1/chat/completions", 200);
    assert_eq!(accumulator.drain().len(), 1);
    assert!(accumulator.drain().is_empty());
    accumulator.record("api", "/v1/chat/completions", 200);
    assert_eq!(
        accumulator
            .drain()
            .get(&key("2026-01-05", "api", "/v1/chat/completions"))
            .copied(),
        Some(counts(1, 0))
    );
}

#[test]
fn restore_merges_the_snapshot_into_the_recorded_counts() {
    let accumulator = accumulator_at(2026, 1, 5);
    accumulator.record("api", "/v1/chat/completions", 200);
    let snapshot = GatewayRequestSnapshot::from([
        (
            key("2026-01-05", "api", "/v1/chat/completions"),
            counts(2, 3),
        ),
        (
            key("2026-01-06", "api", "/v1/chat/completions"),
            counts(1, 0),
        ),
    ]);
    accumulator.restore(&snapshot);
    let merged = accumulator.drain();
    assert_eq!(merged.len(), 2);
    assert_eq!(
        merged
            .get(&key("2026-01-05", "api", "/v1/chat/completions"))
            .copied(),
        Some(counts(3, 3))
    );
    assert_eq!(
        merged
            .get(&key("2026-01-06", "api", "/v1/chat/completions"))
            .copied(),
        Some(counts(1, 0))
    );
}

#[test]
fn restore_collapses_repeated_keys_onto_one_count() {
    let accumulator = accumulator_at(2026, 1, 5);
    let snapshot =
        GatewayRequestSnapshot::from([(key("2026-01-05", "api", "/v1/chat"), counts(2, 3))]);
    accumulator.restore(&snapshot);
    accumulator.restore(&snapshot);
    assert_eq!(
        accumulator
            .drain()
            .get(&key("2026-01-05", "api", "/v1/chat"))
            .copied(),
        Some(counts(4, 6))
    );
}

#[test]
fn restore_stays_bounded_by_the_unique_keys() {
    let accumulator = accumulator_at(2026, 1, 5);
    let snapshot = GatewayRequestSnapshot::from([
        (key("2026-01-05", "api", "/v1/chat"), counts(2, 3)),
        (key("2026-01-06", "api", "/v1/chat"), counts(1, 0)),
    ]);
    accumulator.restore(&snapshot);
    accumulator.restore(&snapshot);
    accumulator.restore(&snapshot);
    assert_eq!(accumulator.drain().len(), 2);
}
