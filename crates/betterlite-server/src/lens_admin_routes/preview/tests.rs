#![cfg(test)]

use axum::http::StatusCode;
use betterlite_lens::models::execution::Sample;
use chrono::{DateTime, Utc};

use crate::lens_access::tests::support::{
    Harness, admin, call, execution_id, member, minutes_after, now, request,
};

const ACCESS_DENIED: &str = "Lens requires proxy administrator access";
const TRACING_DISABLED: &str =
    "Agent tracing is not enabled. Set `tracing:` in general_settings and CLICKHOUSE_URL.";

fn window_body(lookback_hours: u32, offset: i64) -> String {
    format!(
        r#"{{"settings":{{"name":"Nightly","model":"gpt-4o","context":"Watch the checkout flow"}},"lookback_hours":{lookback_hours},"offset":{offset}}}"#
    )
}

fn as_of_body(as_of: &str) -> String {
    format!(
        r#"{{"settings":{{"name":"Nightly","model":"gpt-4o","context":"Watch the checkout flow"}},"as_of":"{as_of}"}}"#
    )
}

fn millis(at: DateTime<Utc>) -> i64 {
    at.timestamp_micros().div_euclid(1_000)
}

#[tokio::test]
async fn a_broken_selection_is_422_before_anything_else() {
    let harness = Harness::new().without_storage();
    let logs = execution_id(&["logs", "team-1", "trace-1"]);
    let body = format!(
        r#"{{"settings":{{"name":"Nightly","model":"gpt-4o","context":"Watch the checkout flow","execution_ids":["{logs}"]}}}}"#
    );
    let reply = call(
        harness.app(),
        request("POST", "/lens/preview/sample", Some(&member()), Some(&body)),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        reply.detail(),
        "Choose execution IDs returned by the activity preview"
    );
}

#[tokio::test]
async fn without_storage_the_route_is_501_before_the_role_check() {
    let harness = Harness::new().without_storage();
    let body = window_body(24, 0);
    let reply = call(
        harness.app(),
        request("POST", "/lens/preview/sample", Some(&member()), Some(&body)),
    )
    .await;
    assert_eq!(reply.status, StatusCode::NOT_IMPLEMENTED);
    assert_eq!(reply.detail(), TRACING_DISABLED);
}

#[tokio::test]
async fn with_storage_a_non_admin_is_403() {
    let harness = Harness::new();
    let body = window_body(24, 0);
    let reply = call(
        harness.app(),
        request("POST", "/lens/preview/sample", Some(&member()), Some(&body)),
    )
    .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.detail(), ACCESS_DENIED);
}

#[tokio::test]
async fn the_window_runs_from_the_clock() {
    let harness = Harness::new();
    let body = window_body(3, 5);
    let reply = call(
        harness.app(),
        request("POST", "/lens/preview/sample", Some(&admin()), Some(&body)),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let sample: Sample = reply.json();
    assert!(sample.executions.is_empty());
    let storage = harness.storage.as_ref().expect("the storage exists");
    let samples = storage.samples();
    let [parameters] = samples.as_slice() else {
        panic!("one sample query");
    };
    assert_eq!(parameters.start, millis(minutes_after(now(), -180)));
    assert_eq!(parameters.end, millis(minutes_after(now(), -2)));
    assert_eq!(parameters.offset, 5);
    assert_eq!(parameters.limit, 100);
    assert!(parameters.preview);
    assert_eq!(parameters.after, "");
}

#[tokio::test]
async fn the_default_lookback_is_a_day() {
    let harness = Harness::new();
    let body =
        r#"{"settings":{"name":"Nightly","model":"gpt-4o","context":"Watch the checkout flow"}}"#;
    let reply = call(
        harness.app(),
        request("POST", "/lens/preview/sample", Some(&admin()), Some(body)),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let storage = harness.storage.as_ref().expect("the storage exists");
    let samples = storage.samples();
    let [parameters] = samples.as_slice() else {
        panic!("one sample query");
    };
    assert_eq!(parameters.start, millis(minutes_after(now(), -1440)));
}

#[tokio::test]
async fn an_earlier_as_of_moves_the_window() {
    let harness = Harness::new();
    let body = as_of_body("2026-05-01T10:00:00+00:00");
    let reply = call(
        harness.app(),
        request("POST", "/lens/preview/sample", Some(&admin()), Some(&body)),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let storage = harness.storage.as_ref().expect("the storage exists");
    let samples = storage.samples();
    let [parameters] = samples.as_slice() else {
        panic!("one sample query");
    };
    let as_of = DateTime::parse_from_rfc3339("2026-05-01T10:00:00+00:00")
        .expect("the fixture instant")
        .with_timezone(&Utc);
    assert_eq!(parameters.end, millis(as_of) - 120_000);
}

#[tokio::test]
async fn a_later_as_of_is_clamped_to_the_clock() {
    let harness = Harness::new();
    let body = as_of_body("2026-05-02T00:00:00+00:00");
    let reply = call(
        harness.app(),
        request("POST", "/lens/preview/sample", Some(&admin()), Some(&body)),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let storage = harness.storage.as_ref().expect("the storage exists");
    let samples = storage.samples();
    let [parameters] = samples.as_slice() else {
        panic!("one sample query");
    };
    assert_eq!(parameters.end, millis(minutes_after(now(), -2)));
}

#[tokio::test]
async fn a_negative_offset_is_422() {
    let harness = Harness::new();
    let body = window_body(24, -1);
    let reply = call(
        harness.app(),
        request("POST", "/lens/preview/sample", Some(&admin()), Some(&body)),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let issues = reply.issues();
    assert_eq!(issues[0].loc[0], "body");
}

#[tokio::test]
async fn a_lookback_outside_the_range_is_422() {
    let harness = Harness::new();
    let zero = window_body(0, 0);
    let reply = call(
        harness.app(),
        request("POST", "/lens/preview/sample", Some(&admin()), Some(&zero)),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let too_long = window_body(8761, 0);
    let refused = call(
        harness.app(),
        request(
            "POST",
            "/lens/preview/sample",
            Some(&admin()),
            Some(&too_long),
        ),
    )
    .await;
    assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn a_missing_settings_body_is_422() {
    let harness = Harness::new();
    let reply = call(
        harness.app(),
        request("POST", "/lens/preview/sample", Some(&admin()), Some("{}")),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
}
