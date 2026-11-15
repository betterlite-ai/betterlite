#![cfg(test)]

use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};

use super::super::tests::support::{Answer, open};
use super::{RetrievalError, get_request_response_payload};

fn start_time() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2024-03-09T12:00:00Z")
        .expect("the start time is valid")
        .with_timezone(&Utc)
}

fn payload(id: &str) -> Json {
    serde_json::from_str(&format!(r#"{{"id": "{id}"}}"#)).expect("the expected payload parses")
}

#[tokio::test]
async fn the_start_date_is_tried_first() {
    let harness = open(&[], Some("bucket"), Answer::ok(r#"{"id": "payload-1"}"#));
    let found = get_request_response_payload(&harness.logger, "chatcmpl-1", Some(start_time()))
        .await
        .expect("the lookup runs");
    assert_eq!(found, Some(payload("payload-1")));
    let requests = harness.requests();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].url().contains("2024-03-09%2Fchatcmpl-1"));
    assert!(requests[0].url().contains("/b/bucket/o/"));
}

#[tokio::test]
async fn the_next_day_is_tried_second() {
    let harness = open(&[], Some("bucket"), Answer::ok("{}"));
    harness.client.script(vec![
        Answer::with_status(404),
        Answer::ok(r#"{"id": "payload-2"}"#),
    ]);
    let found = get_request_response_payload(&harness.logger, "chatcmpl-1", Some(start_time()))
        .await
        .expect("the lookup runs");
    assert_eq!(found, Some(payload("payload-2")));
    let requests = harness.requests();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].url().contains("2024-03-09%2Fchatcmpl-1"));
    assert!(requests[1].url().contains("2024-03-10%2Fchatcmpl-1"));
}

#[tokio::test]
async fn the_previous_day_is_tried_last() {
    let harness = open(&[], Some("bucket"), Answer::ok("{}"));
    harness.client.script(vec![
        Answer::with_status(404),
        Answer::with_status(500),
        Answer::ok(r#"{"id": "payload-3"}"#),
    ]);
    let found = get_request_response_payload(&harness.logger, "chatcmpl-1", Some(start_time()))
        .await
        .expect("the lookup runs");
    assert_eq!(found, Some(payload("payload-3")));
    let requests = harness.requests();
    assert_eq!(requests.len(), 3);
    assert!(requests[1].url().contains("2024-03-10%2Fchatcmpl-1"));
    assert!(requests[2].url().contains("2024-03-08%2Fchatcmpl-1"));
}

#[tokio::test]
async fn a_body_that_is_not_json_is_skipped() {
    let harness = open(&[], Some("bucket"), Answer::ok("{}"));
    harness.client.script(vec![
        Answer::ok("not a payload"),
        Answer::ok(r#"{"id": "payload-4"}"#),
    ]);
    let found = get_request_response_payload(&harness.logger, "chatcmpl-1", Some(start_time()))
        .await
        .expect("the lookup runs");
    assert_eq!(found, Some(payload("payload-4")));
    assert_eq!(harness.requests().len(), 2);
}

#[tokio::test]
async fn a_transport_failure_is_treated_as_a_missing_object() {
    let harness = open(&[], Some("bucket"), Answer::ok("{}"));
    harness.client.fail_sends();
    let found = get_request_response_payload(&harness.logger, "chatcmpl-1", Some(start_time()))
        .await
        .expect("the lookup runs");
    assert_eq!(found, None);
    assert_eq!(harness.requests().len(), 3);
}

#[tokio::test]
async fn none_is_returned_after_three_missing_objects() {
    let harness = open(&[], Some("bucket"), Answer::with_status(404));
    let found = get_request_response_payload(&harness.logger, "chatcmpl-1", Some(start_time()))
        .await
        .expect("the lookup runs");
    assert_eq!(found, None);
    assert_eq!(harness.requests().len(), 3);
}

#[tokio::test]
async fn a_missing_start_time_is_refused() {
    let harness = open(&[], Some("bucket"), Answer::ok("{}"));
    let error = get_request_response_payload(&harness.logger, "chatcmpl-1", None)
        .await
        .expect_err("the start time is required");
    assert!(matches!(error, RetrievalError::MissingStartTime));
    assert!(harness.requests().is_empty());
}
