#![cfg(test)]

use std::collections::BTreeMap;

use axum::http::StatusCode;
use serde::Deserialize;

use crate::debug_routes::otel_spans::RecordedSpan;
use crate::debug_routes::tests::test_support;

#[derive(Deserialize)]
struct SpansBody {
    otel_spans: Vec<String>,
    spans_grouped_by_parent: BTreeMap<String, Vec<String>>,
}

fn span(name: &str, parent: Option<u128>, start: u64) -> RecordedSpan {
    RecordedSpan {
        name: name.to_owned(),
        parent_trace_id: parent,
        start_time_nanos: start,
    }
}

async fn response_with(spans: Vec<RecordedSpan>) -> (StatusCode, String) {
    let state = test_support::with_spans(test_support::state(), spans);
    let router = test_support::protected(test_support::router(state, None));
    test_support::call(router, "GET", "/otel-spans", Some(test_support::MASTER_KEY)).await
}

#[tokio::test]
async fn an_absent_exporter_answers_with_empty_results() {
    let router = test_support::protected(test_support::router(test_support::state(), None));
    let (status, text) =
        test_support::call(router, "GET", "/otel-spans", Some(test_support::MASTER_KEY)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        text,
        r#"{"otel_spans":[],"spans_grouped_by_parent":{},"most_recent_parent":null}"#
    );
}

#[tokio::test]
async fn groups_spans_by_parent_trace_id() {
    let (status, text) = response_with(vec![
        span("fetch", Some(10), 5_000_000),
        span("validate", Some(10), 7_000_000),
        span("other", Some(20), 3_000_000),
        span("root", None, 9_000_000),
    ])
    .await;
    assert_eq!(status, StatusCode::OK);
    let body: SpansBody = serde_json::from_str(&text).expect("a spans body");
    assert_eq!(body.otel_spans, vec!["fetch", "validate", "other", "root"]);
    assert_eq!(
        body.spans_grouped_by_parent.get("10"),
        Some(&vec!["fetch".to_owned(), "validate".to_owned()])
    );
    assert_eq!(
        body.spans_grouped_by_parent.get("20"),
        Some(&vec!["other".to_owned()])
    );
    assert_eq!(body.spans_grouped_by_parent.len(), 2);
    assert!(text.contains("\"most_recent_parent\":10"));
}

#[tokio::test]
async fn an_equal_start_time_keeps_the_first_parent() {
    let (_, text) = response_with(vec![
        span("first", Some(1), 5_000_000),
        span("second", Some(2), 5_000_000),
    ])
    .await;
    assert!(text.contains("\"most_recent_parent\":1"));
}

#[tokio::test]
async fn the_starting_timestamp_is_not_recent_enough() {
    let (_, at_start) = response_with(vec![span("at-start", Some(1), 1_000_000)]).await;
    assert!(at_start.contains("\"most_recent_parent\":null"));
    assert!(at_start.contains("\"1\":[\"at-start\"]"));
    let (_, above_start) = response_with(vec![span("above", Some(2), 1_000_001)]).await;
    assert!(above_start.contains("\"most_recent_parent\":2"));
}

#[tokio::test]
async fn keeps_a_full_width_trace_id() {
    let (_, text) = response_with(vec![span("wide", Some(u128::MAX), 5_000_000)]).await;
    assert!(text.contains("\"most_recent_parent\":340282366920938463463374607431768211455"));
    assert!(text.contains("\"340282366920938463463374607431768211455\":[\"wide\"]"));
}

#[tokio::test]
async fn refuses_a_request_without_a_key() {
    let router = test_support::router(test_support::state(), None);
    let (status, text) = test_support::call(router, "GET", "/otel-spans", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(text.contains("\"type\":\"auth_error\""));
}
