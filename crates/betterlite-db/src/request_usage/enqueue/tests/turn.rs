use chrono::NaiveDateTime;
use rust_decimal::Decimal;

use crate::autorouter_turn_sql::transaction::TurnTime;
use crate::request_usage::turn_enqueue::TurnEnqueueError;

use super::support::{TurnCase, turn_metadata, turn_payload};

#[tokio::test]
async fn enqueues_one_turn_for_a_routed_request() {
    let case = TurnCase::new();
    case.enqueue(&turn_payload(&turn_metadata(None))).await;
    let turns = case.turns();
    assert_eq!(turns.len(), 1);
    let turn = &turns[0];
    assert_eq!(turn.api_key, "sk-capture");
    assert_eq!(turn.session_id, "sess-1");
    assert_eq!(turn.router_name, "auto-router");
    assert_eq!(turn.model, "anthropic/claude-sonnet-5");
    assert_eq!(turn.total_tokens, 7);
    assert_eq!(turn.spend, Decimal::new(25, 2));
    assert_eq!(turn.saved_spend, Decimal::ZERO);
    assert_eq!(turn.classifier_cost, Decimal::ZERO);
    assert_eq!(
        turn.baseline_model.as_deref(),
        Some("anthropic/claude-opus-5")
    );
    assert_eq!(turn.user_id, "u1");
    let TurnTime::Naive(at) = turn.turn_at else {
        panic!("a logged start time reads as a naive turn time");
    };
    assert_eq!(
        at,
        NaiveDateTime::parse_from_str("2026-01-02T03:04:05", "%Y-%m-%dT%H:%M:%S").unwrap()
    );
    assert!(case.baseline_records().is_empty());
}

#[tokio::test]
async fn skips_a_turn_that_names_no_routing_decision() {
    let cases = [
        String::new(),
        String::from("{}"),
        String::from(r#"{"routing_decision":{}}"#),
        String::from(r#"{"routing_decision":"auto-router"}"#),
        String::from(r#"{"routing_decision":[]}"#),
        String::from(r#"{"routing_decision":null}"#),
    ];
    for metadata in cases {
        let case = TurnCase::new();
        case.enqueue(&turn_payload(&metadata)).await;
        assert!(case.turns().is_empty());
        assert!(case.baseline_records().is_empty());
    }
}

#[tokio::test]
async fn rejects_metadata_that_is_not_a_mapping() {
    let case = TurnCase::new();
    let error = case
        .enqueue_result(&turn_payload("[1, 2]"))
        .await
        .unwrap_err();
    assert!(matches!(error, TurnEnqueueError::Metadata(_)));
}

#[tokio::test]
async fn rejects_a_request_whose_payload_is_not_json() {
    let case = TurnCase::new();
    let error = case.enqueue_result("not json").await.unwrap_err();
    assert!(matches!(error, TurnEnqueueError::Payload(_)));
}

#[tokio::test]
async fn rejects_an_end_time_that_is_not_a_bill_time() {
    for end_time in ["true", "5", "[1]", "{\"at\":1}"] {
        let case = TurnCase::new();
        let payload = turn_payload(&turn_metadata(None)).replace(
            r#""endTime":"2026-01-02T03:04:06Z""#,
            &format!(r#""endTime":{end_time}"#),
        );
        let error = case.enqueue_result(&payload).await.unwrap_err();
        assert!(matches!(error, TurnEnqueueError::EndTime));
    }
}

#[tokio::test]
async fn bills_a_request_without_an_end_time() {
    let case = TurnCase::new();
    let payload =
        turn_payload(&turn_metadata(None)).replace(r#","endTime":"2026-01-02T03:04:06Z""#, "");
    case.enqueue(&payload).await;
    assert_eq!(case.turn_count(), 1);
}

#[tokio::test]
async fn reports_a_turn_queue_that_no_longer_admits_records() {
    let case = TurnCase::new();
    assert_eq!(case.close_turns(), 0);
    let error = case
        .enqueue_result(&turn_payload(&turn_metadata(None)))
        .await
        .unwrap_err();
    assert!(matches!(error, TurnEnqueueError::Queue(_)));
    assert!(case.turns().is_empty());
}
