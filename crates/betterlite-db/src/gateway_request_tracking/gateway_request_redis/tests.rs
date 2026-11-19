#![cfg(test)]

use std::sync::Arc;

use super::GATEWAY_REQUESTS_BUFFER_KEY;
use super::GATEWAY_REQUESTS_JOB_NAME;
use super::GatewayRequestRedisError;
use super::MAX_REDIS_BUFFER_DEQUEUE_COUNT;
use crate::gateway_request_tracking::test_support::FakeDatabase;
use crate::gateway_request_tracking::test_support::FakeLease;
use crate::gateway_request_tracking::test_support::FakeList;
use crate::gateway_request_tracking::test_support::buffer_with;
use crate::gateway_request_tracking::test_support::captured_logs;
use crate::gateway_request_tracking::test_support::counts;
use crate::gateway_request_tracking::test_support::key;
use crate::gateway_request_tracking::test_support::snapshot;
use crate::gateway_requests::GatewayRequestSnapshot;

type Row = (String, String, String, i64, i64);

#[test]
fn the_buffer_constants_are_the_litellm_ones() {
    assert_eq!(
        GATEWAY_REQUESTS_BUFFER_KEY,
        "litellm_gateway_requests_buffer"
    );
    assert_eq!(GATEWAY_REQUESTS_JOB_NAME, "update_gateway_requests_job");
    assert_eq!(MAX_REDIS_BUFFER_DEQUEUE_COUNT, 100);
}

#[tokio::test]
async fn push_sends_the_snapshot_as_one_json_row_list() {
    let list = Arc::new(FakeList::default());
    let buffer = buffer_with(
        Arc::clone(&list),
        Arc::new(FakeLease::granting()),
        MAX_REDIS_BUFFER_DEQUEUE_COUNT,
    );
    buffer
        .push(&snapshot(&[("2026-01-05", "api", "/v1/chat", 3, 1)]))
        .await
        .expect("the push succeeds");
    let pushed = list.pushed();
    assert_eq!(pushed.len(), 1);
    let rows: Vec<Row> = serde_json::from_str(&pushed[0]).expect("the row list as JSON");
    assert_eq!(
        rows,
        vec![(
            "2026-01-05".to_owned(),
            "api".to_owned(),
            "/v1/chat".to_owned(),
            3,
            1
        )]
    );
}

#[tokio::test]
async fn push_of_an_empty_snapshot_sends_nothing() {
    let list = Arc::new(FakeList::default());
    let buffer = buffer_with(
        Arc::clone(&list),
        Arc::new(FakeLease::granting()),
        MAX_REDIS_BUFFER_DEQUEUE_COUNT,
    );
    buffer
        .push(&GatewayRequestSnapshot::new())
        .await
        .expect("the push succeeds");
    assert!(list.pushed().is_empty());
}

#[tokio::test]
async fn push_reports_a_list_failure() {
    let list = Arc::new(FakeList::default());
    list.fail_push();
    let buffer = buffer_with(
        Arc::clone(&list),
        Arc::new(FakeLease::granting()),
        MAX_REDIS_BUFFER_DEQUEUE_COUNT,
    );
    let error = buffer
        .push(&snapshot(&[("2026-01-05", "api", "/v1/chat", 3, 1)]))
        .await
        .expect_err("the push fails");
    match error {
        GatewayRequestRedisError::List { operation, .. } => {
            assert_eq!(operation, "async_rpush");
        }
        other @ GatewayRequestRedisError::Encode(_) => panic!("unexpected error: {other}"),
    }
}

#[tokio::test]
async fn pop_folds_every_buffered_entry() {
    let list = Arc::new(FakeList::with_entries(&[
        r#"[["2026-01-05","api","/v1/chat",2,1]]"#,
        r#"[["2026-01-05","api","/v1/chat",3,4],["2026-01-06","api","/v1/chat",1,0]]"#,
    ]));
    let buffer = buffer_with(
        Arc::clone(&list),
        Arc::new(FakeLease::granting()),
        MAX_REDIS_BUFFER_DEQUEUE_COUNT,
    );
    let popped = buffer.pop().await.expect("the pop succeeds");
    assert_eq!(popped.len(), 2);
    assert_eq!(
        popped.get(&key("2026-01-05", "api", "/v1/chat")).copied(),
        Some(counts(5, 5))
    );
    assert_eq!(
        popped.get(&key("2026-01-06", "api", "/v1/chat")).copied(),
        Some(counts(1, 0))
    );
    assert!(list.queued().is_empty());
}

#[tokio::test]
async fn pop_stops_at_the_first_short_batch() {
    let list = Arc::new(FakeList::with_entries(&[
        r#"[["2026-01-05","api","/v1/a",1,0]]"#,
        r#"[["2026-01-05","api","/v1/b",1,0]]"#,
        r#"[["2026-01-05","api","/v1/c",1,0]]"#,
    ]));
    let buffer = buffer_with(Arc::clone(&list), Arc::new(FakeLease::granting()), 2);
    let popped = buffer.pop().await.expect("the pop succeeds");
    assert_eq!(popped.len(), 3);
    assert_eq!(list.take_calls(), vec![2, 2]);
}

#[tokio::test]
async fn a_later_take_failure_keeps_the_entries_already_popped() {
    let (logs, _guard) = captured_logs();
    let list = Arc::new(FakeList::with_entries(&[
        r#"[["2026-01-05","api","/v1/a",1,0]]"#,
        r#"[["2026-01-05","api","/v1/b",2,0]]"#,
        r#"[["2026-01-05","api","/v1/c",4,0]]"#,
    ]));
    list.fail_take_after(2);
    let buffer = buffer_with(Arc::clone(&list), Arc::new(FakeLease::granting()), 1);
    let popped = buffer.pop().await.expect("the drain keeps what it popped");
    assert_eq!(
        popped.get(&key("2026-01-05", "api", "/v1/a")).copied(),
        Some(counts(1, 0))
    );
    assert_eq!(
        popped.get(&key("2026-01-05", "api", "/v1/b")).copied(),
        Some(counts(2, 0))
    );
    assert_eq!(popped.len(), 2);
    assert_eq!(
        list.queued(),
        vec![r#"[["2026-01-05","api","/v1/c",4,0]]"#.to_owned()]
    );
    assert!(logs.text().contains("Redis drain stopped after 2 entries"));
}

#[tokio::test]
async fn pop_reads_a_full_batch_twice_when_the_count_divides_evenly() {
    let list = Arc::new(FakeList::with_entries(&[
        r#"[["2026-01-05","api","/v1/a",1,0]]"#,
        r#"[["2026-01-05","api","/v1/b",1,0]]"#,
    ]));
    let buffer = buffer_with(Arc::clone(&list), Arc::new(FakeLease::granting()), 1);
    let popped = buffer.pop().await.expect("the pop succeeds");
    assert_eq!(popped.len(), 2);
    assert_eq!(list.take_calls(), vec![1, 1, 1]);
}

#[tokio::test]
async fn pop_of_an_empty_buffer_is_empty() {
    let list = Arc::new(FakeList::default());
    let buffer = buffer_with(
        Arc::clone(&list),
        Arc::new(FakeLease::granting()),
        MAX_REDIS_BUFFER_DEQUEUE_COUNT,
    );
    assert!(buffer.pop().await.expect("the pop succeeds").is_empty());
    assert_eq!(list.take_calls(), vec![MAX_REDIS_BUFFER_DEQUEUE_COUNT]);
}

#[tokio::test]
async fn pop_with_a_zero_dequeue_count_still_finishes() {
    let list = Arc::new(FakeList::with_entries(&[
        r#"[["2026-01-05","api","/v1/a",1,0]]"#,
    ]));
    let buffer = buffer_with(Arc::clone(&list), Arc::new(FakeLease::granting()), 0);
    let popped = buffer.pop().await.expect("the pop succeeds");
    assert_eq!(popped.len(), 1);
    assert_eq!(list.take_calls(), vec![1, 1]);
}

#[tokio::test]
async fn pop_reports_a_list_failure() {
    let list = Arc::new(FakeList::default());
    list.fail_take();
    let buffer = buffer_with(
        Arc::clone(&list),
        Arc::new(FakeLease::granting()),
        MAX_REDIS_BUFFER_DEQUEUE_COUNT,
    );
    let error = buffer.pop().await.expect_err("the pop fails");
    match error {
        GatewayRequestRedisError::List { operation, .. } => {
            assert_eq!(operation, "async_lpop");
        }
        other @ GatewayRequestRedisError::Encode(_) => panic!("unexpected error: {other}"),
    }
}

#[tokio::test]
async fn an_entry_that_is_not_a_row_list_is_dropped_alone() {
    let (logs, _guard) = captured_logs();
    let list = Arc::new(FakeList::with_entries(&[
        "not a row list",
        r#"[["2026-01-05","api","/v1/chat",2,1]]"#,
    ]));
    let buffer = buffer_with(
        Arc::clone(&list),
        Arc::new(FakeLease::granting()),
        MAX_REDIS_BUFFER_DEQUEUE_COUNT,
    );
    let popped = buffer.pop().await.expect("the bad entry is dropped");
    assert_eq!(
        popped.get(&key("2026-01-05", "api", "/v1/chat")).copied(),
        Some(counts(2, 1))
    );
    assert_eq!(popped.len(), 1);
    assert!(list.queued().is_empty());
    assert!(
        logs.text()
            .contains("dropped a buffered entry that is not a row list")
    );
}

#[test]
fn a_byte_entry_that_is_not_utf8_is_dropped_alone() {
    let (logs, _guard) = captured_logs();
    assert_eq!(
        super::adapters::decode_text(b"a row list".to_vec()),
        Some("a row list".to_owned())
    );
    assert_eq!(super::adapters::decode_text(vec![0xff, 0xfe]), None);
    assert!(
        logs.text()
            .contains("dropped a buffered entry that is not UTF-8")
    );
}

#[tokio::test]
async fn commit_without_the_lease_commits_nothing() {
    let list = Arc::new(FakeList::with_entries(&[
        r#"[["2026-01-05","api","/v1/a",1,0]]"#,
    ]));
    let database = FakeDatabase::default();
    let buffer = buffer_with(
        Arc::clone(&list),
        Arc::new(FakeLease::denying()),
        MAX_REDIS_BUFFER_DEQUEUE_COUNT,
    );
    let leftover = buffer
        .commit_if_leader(&database)
        .await
        .expect("the leader check succeeds");
    assert!(leftover.is_empty());
    assert_eq!(database.calls(), 0);
    assert_eq!(list.queued().len(), 1);
}

#[tokio::test]
async fn commit_without_a_lease_answer_commits_nothing() {
    let list = Arc::new(FakeList::with_entries(&[
        r#"[["2026-01-05","api","/v1/a",1,0]]"#,
    ]));
    let database = FakeDatabase::default();
    let buffer = buffer_with(
        Arc::clone(&list),
        Arc::new(FakeLease::unavailable()),
        MAX_REDIS_BUFFER_DEQUEUE_COUNT,
    );
    let leftover = buffer
        .commit_if_leader(&database)
        .await
        .expect("the leader check succeeds");
    assert!(leftover.is_empty());
    assert_eq!(database.calls(), 0);
}

#[tokio::test]
async fn the_leader_commits_the_buffered_rows() {
    let list = Arc::new(FakeList::with_entries(&[
        r#"[["2026-01-05","api","/v1/chat",2,1]]"#,
    ]));
    let database = FakeDatabase::default();
    let lease = Arc::new(FakeLease::granting());
    let buffer = buffer_with(
        Arc::clone(&list),
        Arc::clone(&lease),
        MAX_REDIS_BUFFER_DEQUEUE_COUNT,
    );
    let leftover = buffer
        .commit_if_leader(&database)
        .await
        .expect("the leader commit succeeds");
    assert!(leftover.is_empty());
    assert_eq!(database.calls(), 1);
    assert_eq!(database.last_parameters(), 5);
    assert_eq!(lease.calls(), vec![GATEWAY_REQUESTS_JOB_NAME.to_owned()]);
    assert!(list.queued().is_empty());
}

#[tokio::test]
async fn the_leader_commits_what_it_popped_before_a_take_failure() {
    let list = Arc::new(FakeList::with_entries(&[
        r#"[["2026-01-05","api","/v1/a",1,0]]"#,
        r#"[["2026-01-05","api","/v1/b",2,0]]"#,
        r#"[["2026-01-05","api","/v1/c",4,0]]"#,
    ]));
    list.fail_take_after(2);
    let database = FakeDatabase::default();
    let buffer = buffer_with(Arc::clone(&list), Arc::new(FakeLease::granting()), 1);
    let leftover = buffer
        .commit_if_leader(&database)
        .await
        .expect("the leader commits what it drained");
    assert!(leftover.is_empty());
    assert_eq!(database.calls(), 1);
    assert_eq!(database.last_parameters(), 10);
    assert_eq!(
        list.queued(),
        vec![r#"[["2026-01-05","api","/v1/c",4,0]]"#.to_owned()]
    );
}

#[tokio::test]
async fn a_failed_commit_requeues_the_rows() {
    let list = Arc::new(FakeList::with_entries(&[
        r#"[["2026-01-05","api","/v1/chat",2,1]]"#,
    ]));
    let database = FakeDatabase::failing();
    let buffer = buffer_with(
        Arc::clone(&list),
        Arc::new(FakeLease::granting()),
        MAX_REDIS_BUFFER_DEQUEUE_COUNT,
    );
    let leftover = buffer
        .commit_if_leader(&database)
        .await
        .expect("the requeue keeps the rows in Redis");
    assert!(leftover.is_empty());
    assert_eq!(database.calls(), 1);
    let pushed = list.pushed();
    assert_eq!(pushed.len(), 1);
    let rows: Vec<Row> = serde_json::from_str(&pushed[0]).expect("the row list as JSON");
    assert_eq!(
        rows,
        vec![(
            "2026-01-05".to_owned(),
            "api".to_owned(),
            "/v1/chat".to_owned(),
            2,
            1
        )]
    );
}

#[tokio::test]
async fn a_failed_commit_warns_with_the_error_text() {
    let (logs, _guard) = captured_logs();
    let list = Arc::new(FakeList::with_entries(&[
        r#"[["2026-01-05","api","/v1/a",1,0]]"#,
    ]));
    let buffer = buffer_with(
        Arc::clone(&list),
        Arc::new(FakeLease::granting()),
        MAX_REDIS_BUFFER_DEQUEUE_COUNT,
    );
    let database = FakeDatabase::failing();
    buffer
        .commit_if_leader(&database)
        .await
        .expect("the leader re-queues the rows");
    assert!(logs.text().contains(
        "failed to commit 1 buffered rows, re-queuing to Redis for the next flush: the fake database is unavailable"
    ));
}

#[tokio::test]
async fn a_failed_requeue_returns_the_rows_to_the_caller() {
    let list = Arc::new(FakeList::with_entries(&[
        r#"[["2026-01-05","api","/v1/chat",2,1]]"#,
    ]));
    list.fail_push();
    let database = FakeDatabase::failing();
    let buffer = buffer_with(
        Arc::clone(&list),
        Arc::new(FakeLease::granting()),
        MAX_REDIS_BUFFER_DEQUEUE_COUNT,
    );
    let leftover = buffer
        .commit_if_leader(&database)
        .await
        .expect("the caller keeps the rows");
    assert_eq!(
        leftover.get(&key("2026-01-05", "api", "/v1/chat")).copied(),
        Some(counts(2, 1))
    );
    assert_eq!(database.calls(), 1);
}

#[tokio::test]
async fn a_pop_failure_is_reported_to_the_leader() {
    let list = Arc::new(FakeList::default());
    list.fail_take();
    let database = FakeDatabase::default();
    let buffer = buffer_with(
        Arc::clone(&list),
        Arc::new(FakeLease::granting()),
        MAX_REDIS_BUFFER_DEQUEUE_COUNT,
    );
    let error = buffer
        .commit_if_leader(&database)
        .await
        .expect_err("the drain fails");
    assert!(matches!(error, GatewayRequestRedisError::List { .. }));
    assert_eq!(database.calls(), 0);
}
