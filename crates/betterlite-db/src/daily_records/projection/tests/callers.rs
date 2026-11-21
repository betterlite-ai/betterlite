#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::daily_records::DailyRecordError;
use crate::daily_records::callers::{
    add_daily_agent_transaction, add_daily_end_user_transaction, add_daily_org_transaction,
    add_daily_tag_transaction, add_daily_team_transaction, add_daily_user_transaction,
};
use crate::daily_records::projection::rows::queue_key;
use crate::daily_records::support::{
    Case, RecordingQueue, base_payload, column, daily_payload, payload_with, projected_base,
};
use crate::daily_spend_sql::entities::{DailySpendEntity, SpendRow};
use crate::daily_spend_sql::values::SpendColumnValue;

fn case() -> Case {
    Case::with_prices(BTreeMap::new())
}

fn only_update(queue: &RecordingQueue) -> (String, SpendRow) {
    let mut recorded = queue.recorded();
    assert_eq!(recorded.len(), 1);
    let update = recorded.pop().expect("one update was recorded");
    assert_eq!(update.len(), 1);
    update.into_iter().next().expect("the update holds one row")
}

fn text(value: &str) -> SpendColumnValue {
    SpendColumnValue::Text(value.to_owned())
}

#[tokio::test]
async fn the_user_caller_queues_its_row_under_the_identity_key() {
    let mut case = case();
    case.endpoints.routes.insert(
        "aimage_generation".to_owned(),
        "/image/generations".to_owned(),
    );
    let queue = RecordingQueue::new(None);
    let payload = daily_payload(&payload_with(r#""call_type":"aimage_generation""#));
    add_daily_user_transaction(Some(&queue), &payload, &case.pricing(), &case.endpoints)
        .await
        .unwrap();
    let (key, row) = only_update(&queue);
    assert_eq!(key, "u1_2025-01-02_k1_gpt-4o_openai_/image/generations");
    assert_eq!(column(&row, "user_id"), text("u1"));
    assert!(!row.contains_key("request_id"));
}

#[tokio::test]
async fn every_caller_returns_before_queuing_without_a_database() {
    let case = case();
    let payload = daily_payload(&payload_with(
        r#""team_id":"t1","organization_id":"o1","end_user":"e1","agent_id":"a1","request_tags":["x"]"#,
    ));
    add_daily_user_transaction(None, &payload, &case.pricing(), &case.endpoints)
        .await
        .unwrap();
    add_daily_team_transaction(None, &payload, &case.pricing(), &case.endpoints)
        .await
        .unwrap();
    add_daily_org_transaction(None, &payload, Some("o2"), &case.pricing(), &case.endpoints)
        .await
        .unwrap();
    add_daily_end_user_transaction(None, &payload, &case.pricing(), &case.endpoints)
        .await
        .unwrap();
    add_daily_agent_transaction(None, &payload, &case.pricing(), &case.endpoints)
        .await
        .unwrap();
    add_daily_tag_transaction(None, &payload, &case.pricing(), &case.endpoints)
        .await
        .unwrap();
}

#[tokio::test]
async fn the_team_caller_queues_the_team_row() {
    let case = case();
    let queue = RecordingQueue::new(None);
    let payload = daily_payload(&payload_with(r#""team_id":"t1""#));
    add_daily_team_transaction(Some(&queue), &payload, &case.pricing(), &case.endpoints)
        .await
        .unwrap();
    let (key, row) = only_update(&queue);
    assert_eq!(key, "t1_2025-01-02_k1_gpt-4o_openai_");
    assert_eq!(column(&row, "team_id"), text("t1"));
}

#[tokio::test]
async fn the_team_caller_skips_a_null_team_id() {
    let case = case();
    let queue = RecordingQueue::new(None);
    let payload = daily_payload(&payload_with(r#""team_id":null"#));
    add_daily_team_transaction(Some(&queue), &payload, &case.pricing(), &case.endpoints)
        .await
        .unwrap();
    assert!(queue.recorded().is_empty());
}

#[tokio::test]
async fn the_org_caller_charges_the_explicit_organization_id() {
    let case = case();
    let queue = RecordingQueue::new(None);
    let payload = daily_payload(&payload_with(r#""organization_id":"o1""#));
    add_daily_org_transaction(
        Some(&queue),
        &payload,
        Some("o2"),
        &case.pricing(),
        &case.endpoints,
    )
    .await
    .unwrap();
    let (key, row) = only_update(&queue);
    assert_eq!(key, "o2_2025-01-02_k1_gpt-4o_openai_");
    assert_eq!(column(&row, "organization_id"), text("o2"));
}

#[tokio::test]
async fn the_org_caller_returns_before_reading_the_payload_without_an_explicit_id() {
    let case = case();
    let queue = RecordingQueue::new(None);
    let payload = daily_payload(&payload_with(r#""organization_id":"o1""#));
    add_daily_org_transaction(
        Some(&queue),
        &payload,
        None,
        &case.pricing(),
        &case.endpoints,
    )
    .await
    .unwrap();
    assert!(queue.recorded().is_empty());
}

#[tokio::test]
async fn the_end_user_caller_normalizes_a_nonempty_end_user() {
    let case = case();
    let queue = RecordingQueue::new(None);
    let payload = daily_payload(&payload_with(r#""end_user":"e1","end_user_id":"e2""#));
    add_daily_end_user_transaction(Some(&queue), &payload, &case.pricing(), &case.endpoints)
        .await
        .unwrap();
    let (key, row) = only_update(&queue);
    assert_eq!(key, "e1_2025-01-02_k1_gpt-4o_openai_");
    assert_eq!(column(&row, "end_user_id"), text("e1"));
}

#[rstest]
#[case::empty(r#""end_user":"""#)]
#[case::null(r#""end_user":null"#)]
#[case::absent("")]
#[tokio::test]
async fn the_end_user_caller_skips_an_empty_end_user(#[case] fragment: &str) {
    let case = case();
    let queue = RecordingQueue::new(None);
    let payload_text = if fragment.is_empty() {
        base_payload()
    } else {
        payload_with(fragment)
    };
    let payload = daily_payload(&payload_text);
    add_daily_end_user_transaction(Some(&queue), &payload, &case.pricing(), &case.endpoints)
        .await
        .unwrap();
    assert!(queue.recorded().is_empty());
}

#[tokio::test]
async fn the_agent_caller_charges_an_empty_agent_id() {
    let case = case();
    let queue = RecordingQueue::new(None);
    let payload = daily_payload(&payload_with(r#""agent_id":"""#));
    add_daily_agent_transaction(Some(&queue), &payload, &case.pricing(), &case.endpoints)
        .await
        .unwrap();
    let (key, row) = only_update(&queue);
    assert_eq!(key, "_2025-01-02_k1_gpt-4o_openai_");
    assert_eq!(column(&row, "agent_id"), text(""));
}

#[tokio::test]
async fn the_agent_caller_skips_a_null_agent_id() {
    let case = case();
    let queue = RecordingQueue::new(None);
    let payload = daily_payload(&payload_with(r#""agent_id":null"#));
    add_daily_agent_transaction(Some(&queue), &payload, &case.pricing(), &case.endpoints)
        .await
        .unwrap();
    assert!(queue.recorded().is_empty());
}

#[tokio::test]
async fn the_tag_caller_queues_one_row_per_deduplicated_tag() {
    let case = case();
    let queue = RecordingQueue::new(None);
    let payload = daily_payload(&payload_with(
        r#""request_tags":"[\"b\",\"a\",\"b\"]","request_id":"req-1""#,
    ));
    add_daily_tag_transaction(Some(&queue), &payload, &case.pricing(), &case.endpoints)
        .await
        .unwrap();
    let mut recorded = queue.recorded();
    assert_eq!(recorded.len(), 2);
    let second = recorded.pop().expect("two tag updates were recorded");
    let first = recorded.pop().expect("two tag updates were recorded");
    let (first_key, first_row) = first.into_iter().next().expect("the update holds one row");
    let (second_key, second_row) = second.into_iter().next().expect("the update holds one row");
    assert_eq!(first_key, "b_2025-01-02_k1_gpt-4o_openai_");
    assert_eq!(second_key, "a_2025-01-02_k1_gpt-4o_openai_");
    assert_eq!(column(&first_row, "tag"), text("b"));
    assert_eq!(column(&second_row, "tag"), text("a"));
    assert_eq!(column(&first_row, "request_id"), text("req-1"));
    assert_eq!(column(&second_row, "request_id"), text("req-1"));
}

#[rstest]
#[case::null(r#""request_tags":null"#)]
#[case::empty_list(r#""request_tags":[]"#)]
#[case::absent("")]
#[tokio::test]
async fn the_tag_caller_queues_nothing_without_tags(#[case] fragment: &str) {
    let case = case();
    let queue = RecordingQueue::new(None);
    let payload_text = if fragment.is_empty() {
        base_payload()
    } else {
        payload_with(fragment)
    };
    let payload = daily_payload(&payload_text);
    add_daily_tag_transaction(Some(&queue), &payload, &case.pricing(), &case.endpoints)
        .await
        .unwrap();
    assert!(queue.recorded().is_empty());
}

#[tokio::test]
async fn an_unreadable_tag_payload_fails_the_row() {
    let case = case();
    let queue = RecordingQueue::new(None);
    let payload = daily_payload(&payload_with(r#""request_tags":"""#));
    let result =
        add_daily_tag_transaction(Some(&queue), &payload, &case.pricing(), &case.endpoints).await;
    assert!(matches!(result, Err(DailyRecordError::Tags { .. })));
}

#[tokio::test]
async fn a_failed_enqueue_is_returned_with_its_entity() {
    let case = case();
    let queue = RecordingQueue::new(Some(0));
    let payload = daily_payload(&payload_with(r#""team_id":"t1""#));
    let result =
        add_daily_team_transaction(Some(&queue), &payload, &case.pricing(), &case.endpoints).await;
    assert!(matches!(
        result,
        Err(DailyRecordError::Enqueue { entity: "team", .. })
    ));
}

#[tokio::test]
async fn the_tag_caller_stops_at_the_first_failed_tag() {
    let case = case();
    let queue = RecordingQueue::new(Some(1));
    let payload = daily_payload(&payload_with(r#""request_tags":["a","b"]"#));
    let result =
        add_daily_tag_transaction(Some(&queue), &payload, &case.pricing(), &case.endpoints).await;
    assert!(matches!(
        result,
        Err(DailyRecordError::Enqueue { entity: "tag", .. })
    ));
    assert_eq!(queue.recorded().len(), 1);
}

#[tokio::test]
async fn a_missing_model_and_endpoint_keep_their_none_text_in_the_key() {
    let case = case();
    let text = r#"{"user":"u1","startTime":"2025-01-02T03:04:05Z","api_key":"k1","mcp_namespaced_tool_name":"tool"}"#;
    let base = projected_base(&case, text, DailySpendEntity::User).await;
    assert_eq!(queue_key(Some("u1"), &base), "u1_2025-01-02_k1_None_None_");
}

#[tokio::test]
async fn a_missing_identity_keeps_its_none_text_in_the_key() {
    let case = case();
    let text = r#"{"user":null,"startTime":"2025-01-02T03:04:05Z","api_key":"k1","model":"gpt-4o","custom_llm_provider":"openai","model_group":"gpt-4o"}"#;
    let base = projected_base(&case, text, DailySpendEntity::User).await;
    assert_eq!(queue_key(None, &base), "None_2025-01-02_k1_gpt-4o_openai_");
}
