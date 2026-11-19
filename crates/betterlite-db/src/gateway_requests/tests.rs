#![cfg(test)]

use serde_json::from_str;
use serde_json::to_string;

use super::GatewayRequestActivityResponse;
use super::GatewayRequestBreakdownEntry;
use super::GatewayRequestDailyEntry;
use super::GatewayRequestKey;
use super::GatewayRequestSnapshot;
use super::gateway_request_counts::GatewayRequestCounts;

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
fn a_daily_entry_defaults_its_counts_to_zero() {
    let entry: GatewayRequestDailyEntry =
        from_str(r#"{"date": "2026-01-05"}"#).expect("a daily entry without counts");
    assert_eq!(
        entry,
        GatewayRequestDailyEntry {
            date: "2026-01-05".to_owned(),
            successful_requests: 0,
            failed_requests: 0,
        }
    );
}

#[test]
fn a_daily_entry_serializes_with_every_field() {
    let entry = GatewayRequestDailyEntry {
        date: "2026-01-05".to_owned(),
        successful_requests: 4,
        failed_requests: 1,
    };
    assert_eq!(
        to_string(&entry).expect("JSON text"),
        r#"{"date":"2026-01-05","successful_requests":4,"failed_requests":1}"#
    );
}

#[test]
fn a_breakdown_entry_defaults_its_counts_to_zero() {
    let entry: GatewayRequestBreakdownEntry =
        from_str(r#"{"category": "api", "route": "/v1/chat/completions"}"#)
            .expect("a breakdown entry without counts");
    assert_eq!(
        entry,
        GatewayRequestBreakdownEntry {
            category: "api".to_owned(),
            route: "/v1/chat/completions".to_owned(),
            successful_requests: 0,
            failed_requests: 0,
        }
    );
}

#[test]
fn a_breakdown_entry_serializes_with_every_field() {
    let entry = GatewayRequestBreakdownEntry {
        category: "api".to_owned(),
        route: "/v1/chat/completions".to_owned(),
        successful_requests: 9,
        failed_requests: 2,
    };
    assert_eq!(
        to_string(&entry).expect("JSON text"),
        r#"{"category":"api","route":"/v1/chat/completions","successful_requests":9,"failed_requests":2}"#
    );
}

#[test]
fn the_activity_response_defaults_to_zero_totals_and_empty_collections() {
    let response: GatewayRequestActivityResponse =
        from_str("{}").expect("an activity response without members");
    assert_eq!(response, GatewayRequestActivityResponse::default());
    assert_eq!(response.total_successful_requests, 0);
    assert_eq!(response.total_failed_requests, 0);
    assert!(response.by_date.is_empty());
    assert!(response.by_route.is_empty());
}

#[test]
fn the_activity_response_serializes_every_field_in_wire_order() {
    let response = GatewayRequestActivityResponse {
        total_successful_requests: 7,
        total_failed_requests: 2,
        by_date: vec![GatewayRequestDailyEntry {
            date: "2026-01-05".to_owned(),
            successful_requests: 7,
            failed_requests: 2,
        }],
        by_route: vec![GatewayRequestBreakdownEntry {
            category: "api".to_owned(),
            route: "/v1/chat/completions".to_owned(),
            successful_requests: 7,
            failed_requests: 2,
        }],
    };
    assert_eq!(
        to_string(&response).expect("JSON text"),
        concat!(
            r#"{"total_successful_requests":7,"total_failed_requests":2,"#,
            r#""by_date":[{"date":"2026-01-05","successful_requests":7,"failed_requests":2}],"#,
            r#""by_route":[{"category":"api","route":"/v1/chat/completions","successful_requests":7,"failed_requests":2}]}"#
        )
    );
}

#[test]
fn snapshot_keys_are_ordered_by_date_then_category_then_route() {
    let snapshot = GatewayRequestSnapshot::from([
        (key("2026-01-02", "api", "/v1/chat"), counts(1, 0)),
        (key("2026-01-01", "passthrough", "/v1/chat"), counts(1, 0)),
        (key("2026-01-01", "api", "/v1/route"), counts(1, 0)),
        (key("2026-01-01", "api", "/v1/chat"), counts(1, 0)),
    ]);
    let ordered: Vec<GatewayRequestKey> = snapshot.keys().cloned().collect();
    assert_eq!(
        ordered,
        vec![
            key("2026-01-01", "api", "/v1/chat"),
            key("2026-01-01", "api", "/v1/route"),
            key("2026-01-01", "passthrough", "/v1/chat"),
            key("2026-01-02", "api", "/v1/chat"),
        ]
    );
}
