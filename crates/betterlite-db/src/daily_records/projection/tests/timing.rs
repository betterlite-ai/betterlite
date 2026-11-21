#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::daily_records::support::{Case, column, payload_with, project, projected_row};
use crate::daily_spend_sql::entities::{DailySpendEntity, SpendRow};
use crate::daily_spend_sql::values::SpendColumnValue;

fn timed_payload(metadata: &str, duration: &str) -> String {
    let quoted = serde_json::to_string(metadata).unwrap();
    payload_with(&format!(
        "\"metadata\":{quoted},\"request_duration_ms\":{duration}"
    ))
}

async fn timed_row(metadata: &str, duration: &str) -> SpendRow {
    let case = Case::with_prices(BTreeMap::new());
    projected_row(
        &case,
        &timed_payload(metadata, duration),
        DailySpendEntity::User,
    )
    .await
}

fn assert_counts(row: &SpendRow, requests: i64, successful: i64, failed: i64) {
    assert_eq!(column(row, "api_requests"), SpendColumnValue::Int(requests));
    assert_eq!(
        column(row, "successful_requests"),
        SpendColumnValue::Int(successful)
    );
    assert_eq!(
        column(row, "failed_requests"),
        SpendColumnValue::Int(failed)
    );
}

fn assert_timed(row: &SpendRow, millis: i64, timed: i64) {
    assert_eq!(
        column(row, "total_response_time_ms"),
        SpendColumnValue::Int(millis)
    );
    assert_eq!(column(row, "timed_requests"), SpendColumnValue::Int(timed));
}

#[tokio::test]
async fn counts_a_successful_call_as_one_request() {
    let row = timed_row(r#"{"status":"success"}"#, "5").await;
    assert_counts(&row, 1, 1, 0);
    assert_timed(&row, 5, 1);
}

#[tokio::test]
async fn counts_a_failed_call_as_a_failed_request() {
    let row = timed_row(r#"{"status":"failure"}"#, "5").await;
    assert_counts(&row, 1, 0, 1);
    assert_timed(&row, 0, 0);
}

#[tokio::test]
async fn reads_the_status_from_the_metadata_not_the_payload() {
    let text = payload_with(&format!(
        "\"status\":\"failure\",\"metadata\":{}",
        serde_json::to_string(r#"{"status":"success"}"#).unwrap()
    ));
    let row = projected_row(
        &Case::with_prices(BTreeMap::new()),
        &text,
        DailySpendEntity::User,
    )
    .await;
    assert_counts(&row, 1, 1, 0);
}

#[tokio::test]
async fn an_internal_call_zeroes_every_request_counter() {
    let row = timed_row(r#"{"internal_call_origin":"autorouter_classifier"}"#, "5").await;
    assert_counts(&row, 0, 0, 0);
    assert_timed(&row, 0, 0);
}

#[tokio::test]
async fn an_internal_call_keeps_its_spend_and_tokens() {
    let row = timed_row(r#"{"internal_call_origin":"shadow_eval"}"#, "5").await;
    assert_eq!(
        serde_json::to_string(&column(&row, "spend")).unwrap(),
        "0.25"
    );
    assert_eq!(column(&row, "prompt_tokens"), SpendColumnValue::Int(3));
    assert_eq!(column(&row, "completion_tokens"), SpendColumnValue::Int(4));
}

#[tokio::test]
async fn an_internal_call_is_counted_as_a_request_even_when_it_failed() {
    let row = timed_row(
        r#"{"internal_call_origin":"classifier","status":"failure"}"#,
        "5",
    )
    .await;
    assert_counts(&row, 0, 0, 0);
    assert_timed(&row, 0, 0);
}

#[tokio::test]
async fn a_zero_duration_is_a_timed_request() {
    let row = timed_row("{}", "0").await;
    assert_timed(&row, 0, 1);
}

#[tokio::test]
async fn a_negative_duration_is_not_timed() {
    let row = timed_row("{}", "-1").await;
    assert_timed(&row, 0, 0);
}

#[rstest]
#[case::enabled("true", 1)]
#[case::disabled("false", 0)]
#[tokio::test]
async fn a_boolean_duration_follows_pythons_integer_test(
    #[case] duration: &str,
    #[case] millis: i64,
) {
    let row = timed_row("{}", duration).await;
    assert_timed(&row, millis, 1);
}

#[rstest]
#[case::whole_float("5.0")]
#[case::fraction("5.5")]
#[case::text(r#""5""#)]
#[case::null("null")]
#[case::list("[]")]
#[tokio::test]
async fn a_duration_that_is_not_an_integer_is_not_timed(#[case] duration: &str) {
    let row = timed_row("{}", duration).await;
    assert_timed(&row, 0, 0);
}

#[tokio::test]
async fn a_duration_beyond_the_integer_range_is_not_timed() {
    let row = timed_row("{}", "9223372036854775808").await;
    assert_timed(&row, 0, 0);
}

#[tokio::test]
async fn a_duration_at_the_integer_limit_is_timed() {
    let row = timed_row("{}", "9223372036854775807").await;
    assert_timed(&row, i64::MAX, 1);
}

#[tokio::test]
async fn an_absent_duration_leaves_the_request_untimed() {
    let case = Case::with_prices(BTreeMap::new());
    let text = payload_with(r#""metadata":"{}""#);
    let row = projected_row(&case, &text, DailySpendEntity::User).await;
    assert_timed(&row, 0, 0);
}

#[tokio::test]
async fn an_absent_metadata_still_builds_the_row() {
    let case = Case::with_prices(BTreeMap::new());
    let text = payload_with(r#""request_duration_ms":5"#);
    let projection = project(&case, &text, DailySpendEntity::User).await.unwrap();
    assert!(projection.is_some());
}

#[tokio::test]
async fn a_null_metadata_still_builds_the_row() {
    let case = Case::with_prices(BTreeMap::new());
    let text = payload_with(r#""metadata":null"#);
    let projection = project(&case, &text, DailySpendEntity::User).await.unwrap();
    assert!(projection.is_some());
}
