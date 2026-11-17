use std::sync::Arc;

use axum::http::StatusCode;
use betterlite_auth::auth_identity::records::Principal;
use betterlite_error::proxy_error_types::common::CommonProxyErrors;
use rstest::rstest;

use super::support::{
    DetailReply, EventListReply, FailureKind, FakeStore, LocReply, MessageListReply,
    ValidationReply, assert_unhandled_failure, get, master, member, member_without_key,
    proxy_admin, viewer,
};
use crate::workflow_runs::models::SequenceTable;

fn events_uri(run_id: &str) -> String {
    format!("/v1/workflows/runs/{run_id}/events")
}

fn messages_uri(run_id: &str) -> String {
    format!("/v1/workflows/runs/{run_id}/messages")
}

fn seeded() -> (Arc<FakeStore>, String) {
    let store = Arc::new(FakeStore::default());
    let run_id = store.seed_run(Some("hash-1"));
    store.seed_event(&run_id, 4);
    store.seed_event(&run_id, 0);
    store.seed_event(&run_id, 2);
    store.seed_message(&run_id, 3);
    store.seed_message(&run_id, 1);
    (store, run_id)
}

#[tokio::test]
async fn events_come_back_ascending_by_sequence() {
    let (store, run_id) = seeded();
    let reply = get(Some(&store), Some(&member("hash-1")), &events_uri(&run_id)).await;
    assert_eq!(reply.status, StatusCode::OK);
    let parsed: EventListReply = serde_json::from_str(&reply.body).unwrap();
    let numbers: Vec<i32> = parsed
        .events
        .iter()
        .map(|event| event.sequence_number)
        .collect();
    assert_eq!(numbers, [0_i32, 2_i32, 4_i32]);
    assert_eq!(parsed.count, 3);
}

#[tokio::test]
async fn messages_come_back_ascending_by_sequence() {
    let (store, run_id) = seeded();
    let reply = get(
        Some(&store),
        Some(&member("hash-1")),
        &messages_uri(&run_id),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let parsed: MessageListReply = serde_json::from_str(&reply.body).unwrap();
    let numbers: Vec<i32> = parsed
        .messages
        .iter()
        .map(|message| message.sequence_number)
        .collect();
    assert_eq!(numbers, [1_i32, 3_i32]);
    assert_eq!(parsed.count, 2);
}

#[tokio::test]
async fn a_list_holds_only_its_own_run_and_its_own_table() {
    let (store, run_id) = seeded();
    let other = store.seed_run(Some("hash-1"));
    store.seed_event(&other, 9);
    store.seed_message(&other, 9);
    let reply = get(Some(&store), Some(&member("hash-1")), &events_uri(&run_id)).await;
    let parsed: EventListReply = serde_json::from_str(&reply.body).unwrap();
    assert!(parsed.events.iter().all(|event| event.run_id == run_id));
    let message_reply = get(
        Some(&store),
        Some(&member("hash-1")),
        &messages_uri(&run_id),
    )
    .await;
    let message_parsed: MessageListReply = serde_json::from_str(&message_reply.body).unwrap();
    assert!(
        message_parsed
            .messages
            .iter()
            .all(|message| message.run_id == run_id)
    );
    assert_eq!(
        store.lists(),
        [
            (SequenceTable::Events, run_id.clone(), 100),
            (SequenceTable::Messages, run_id.clone(), 100),
        ]
    );
}

#[rstest]
#[case::default_limit("", 100)]
#[case::lowest("?limit=1", 1)]
#[case::highest("?limit=500", 500)]
#[tokio::test]
async fn an_event_list_passes_a_limit_inside_the_bounds(#[case] query: &str, #[case] take: u16) {
    let (store, run_id) = seeded();
    let uri = format!("{}{query}", events_uri(&run_id));
    let reply = get(Some(&store), Some(&member("hash-1")), &uri).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        store.lists().first().unwrap(),
        &(SequenceTable::Events, run_id.clone(), take)
    );
}

#[rstest]
#[case::default_limit("", 100)]
#[case::lowest("?limit=1", 1)]
#[case::highest("?limit=500", 500)]
#[tokio::test]
async fn a_message_list_passes_a_limit_inside_the_bounds(#[case] query: &str, #[case] take: u16) {
    let (store, run_id) = seeded();
    let uri = format!("{}{query}", messages_uri(&run_id));
    let reply = get(Some(&store), Some(&member("hash-1")), &uri).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        store.lists().first().unwrap(),
        &(SequenceTable::Messages, run_id.clone(), take)
    );
}

#[tokio::test]
async fn a_list_returns_at_most_its_limit() {
    let (store, run_id) = seeded();
    let uri = format!("{}?limit=2", events_uri(&run_id));
    let reply = get(Some(&store), Some(&member("hash-1")), &uri).await;
    assert_eq!(reply.status, StatusCode::OK);
    let parsed: EventListReply = serde_json::from_str(&reply.body).unwrap();
    assert_eq!(parsed.events.len(), 2);
    assert_eq!(parsed.count, 2);
}

#[rstest]
#[case::zero("?limit=0", "greater_than_equal")]
#[case::negative("?limit=-5", "greater_than_equal")]
#[case::just_above("?limit=501", "less_than_equal")]
#[case::beyond_an_integer("?limit=99999999999", "less_than_equal")]
#[case::text("?limit=abc", "int_parsing")]
#[case::empty("?limit=", "int_parsing")]
#[tokio::test]
async fn an_event_list_rejects_a_limit_outside_the_bounds(
    #[case] query: &str,
    #[case] issue_type: &str,
) {
    let (store, run_id) = seeded();
    let uri = format!("{}{query}", events_uri(&run_id));
    let reply = get(Some(&store), Some(&member("hash-1")), &uri).await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let parsed: ValidationReply = serde_json::from_str(&reply.body).unwrap();
    let issue = parsed.detail.first().unwrap();
    assert_eq!(issue.issue_type, issue_type);
    assert_eq!(
        issue.loc,
        [
            LocReply::Name("query".to_owned()),
            LocReply::Name("limit".to_owned())
        ]
    );
    assert_eq!(
        issue.msg,
        match issue_type {
            "int_parsing" =>
                "Input should be a valid integer, unable to parse string as an integer",
            "greater_than_equal" => "Input should be greater than or equal to 1",
            _ => "Input should be less than or equal to 500",
        }
    );
    assert!(store.lists().is_empty());
}

#[rstest]
#[case::zero("?limit=0")]
#[case::just_above("?limit=501")]
#[tokio::test]
async fn a_message_list_rejects_a_limit_outside_the_bounds(#[case] query: &str) {
    let (store, run_id) = seeded();
    let uri = format!("{}{query}", messages_uri(&run_id));
    let reply = get(Some(&store), Some(&member("hash-1")), &uri).await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let parsed: ValidationReply = serde_json::from_str(&reply.body).unwrap();
    assert!(!parsed.detail.first().unwrap().issue_type.is_empty());
    assert!(store.lists().is_empty());
}

#[tokio::test]
async fn a_list_validates_the_limit_before_it_looks_for_a_database() {
    let reply = get(
        None,
        Some(&master()),
        "/v1/workflows/runs/run-0/events?limit=0",
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let message_reply = get(
        None,
        Some(&master()),
        "/v1/workflows/runs/run-0/messages?limit=501",
    )
    .await;
    assert_eq!(message_reply.status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[rstest]
#[case::master(master())]
#[case::proxy_admin(proxy_admin("hash-9"))]
#[case::view_only(viewer("hash-9"))]
#[tokio::test]
async fn an_admin_view_caller_reads_a_foreign_runs_events_and_messages(#[case] caller: Principal) {
    let (store, run_id) = seeded();
    let reply = get(Some(&store), Some(&caller), &events_uri(&run_id)).await;
    assert_eq!(reply.status, StatusCode::OK);
    let parsed: EventListReply = serde_json::from_str(&reply.body).unwrap();
    assert_eq!(parsed.count, 3);
    let message_reply = get(Some(&store), Some(&caller), &messages_uri(&run_id)).await;
    assert_eq!(message_reply.status, StatusCode::OK);
    let message_parsed: MessageListReply = serde_json::from_str(&message_reply.body).unwrap();
    assert_eq!(message_parsed.count, 2);
}

#[rstest]
#[case::foreign_member(member("hash-2"), true)]
#[case::foreign_key_on_messages(member("hash-2"), false)]
#[case::keyless_member(member_without_key(), true)]
#[tokio::test]
async fn an_ordinary_key_cannot_read_a_foreign_runs_events_or_messages(
    #[case] caller: Principal,
    #[case] events: bool,
) {
    let (store, run_id) = seeded();
    let uri = if events {
        events_uri(&run_id)
    } else {
        messages_uri(&run_id)
    };
    let reply = get(Some(&store), Some(&caller), &uri).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    let detail: DetailReply = serde_json::from_str(&reply.body).unwrap();
    assert_eq!(detail.detail, format!("Run '{run_id}' not found"));
    assert!(store.lists().is_empty());
}

#[rstest]
#[case::events(true)]
#[case::messages(false)]
#[tokio::test]
async fn a_missing_run_is_a_404_for_every_reader(#[case] events: bool) {
    let store = Arc::new(FakeStore::default());
    store.seed_run(Some("hash-1"));
    let uri = if events {
        events_uri("run-404")
    } else {
        messages_uri("run-404")
    };
    let reply = get(Some(&store), Some(&master()), &uri).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert!(store.lists().is_empty());
}

#[rstest]
#[case::events(true)]
#[case::messages(false)]
#[tokio::test]
async fn a_list_without_a_database_is_a_500_with_the_db_error(#[case] events: bool) {
    let uri = if events {
        events_uri("run-0")
    } else {
        messages_uri("run-0")
    };
    let reply = get(None, Some(&master()), &uri).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    let detail: DetailReply = serde_json::from_str(&reply.body).unwrap();
    assert_eq!(
        detail.detail,
        CommonProxyErrors::DbNotConnectedError.as_str()
    );
}

#[rstest]
#[case::events(true)]
#[case::messages(false)]
#[tokio::test]
async fn a_list_store_failure_is_a_500(#[case] events: bool) {
    let store = Arc::new(FakeStore::failing_as(FailureKind::List));
    store.seed_run(Some("hash-1"));
    let uri = if events {
        events_uri("run-0")
    } else {
        messages_uri("run-0")
    };
    let reply = get(Some(&store), Some(&master()), &uri).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[rstest]
#[case::events(true)]
#[case::messages(false)]
#[tokio::test]
async fn a_list_run_lookup_failure_is_an_unhandled_500(#[case] events: bool) {
    let store = Arc::new(FakeStore::failing_as(FailureKind::Read));
    let uri = if events {
        events_uri("run-0")
    } else {
        messages_uri("run-0")
    };
    let reply = get(Some(&store), Some(&master()), &uri).await;
    assert_unhandled_failure(&reply);
}

#[rstest]
#[case::events(true)]
#[case::messages(false)]
#[tokio::test]
async fn a_list_without_a_caller_is_unauthorized(#[case] events: bool) {
    let (store, run_id) = seeded();
    let uri = if events {
        events_uri(&run_id)
    } else {
        messages_uri(&run_id)
    };
    let reply = get(Some(&store), None, &uri).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert!(store.lists().is_empty());
}
