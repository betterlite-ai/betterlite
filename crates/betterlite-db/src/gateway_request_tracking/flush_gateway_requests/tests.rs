#![cfg(test)]

use std::sync::Arc;

use super::flush_gateway_requests;
use crate::gateway_request_tracking::gateway_request_accumulator::GatewayRequestAccumulator;
use crate::gateway_request_tracking::gateway_request_redis::MAX_REDIS_BUFFER_DEQUEUE_COUNT;
use crate::gateway_request_tracking::test_support::FakeDatabase;
use crate::gateway_request_tracking::test_support::FakeLease;
use crate::gateway_request_tracking::test_support::FakeList;
use crate::gateway_request_tracking::test_support::buffer_with;
use crate::gateway_request_tracking::test_support::snapshot;
use crate::gateway_requests::GatewayRequestSnapshot;

fn accumulator_with(initial: &GatewayRequestSnapshot) -> GatewayRequestAccumulator {
    let accumulator = GatewayRequestAccumulator::new();
    accumulator.restore(initial);
    accumulator
}

#[tokio::test]
async fn the_flush_without_a_buffer_commits_the_drained_counts() {
    let database = FakeDatabase::default();
    let initial = snapshot(&[("2026-01-05", "api", "/v1/chat", 2, 1)]);
    let accumulator = accumulator_with(&initial);
    flush_gateway_requests(&database, &accumulator, None).await;
    assert_eq!(database.calls(), 1);
    assert_eq!(database.last_parameters(), 5);
    assert!(accumulator.drain().is_empty());
}

#[tokio::test]
async fn the_flush_without_a_buffer_restores_the_counts_on_failure() {
    let database = FakeDatabase::failing();
    let initial = snapshot(&[("2026-01-05", "api", "/v1/chat", 2, 1)]);
    let accumulator = accumulator_with(&initial);
    flush_gateway_requests(&database, &accumulator, None).await;
    assert_eq!(database.calls(), 1);
    assert_eq!(accumulator.drain(), initial);
}

#[tokio::test]
async fn the_flush_with_a_buffer_pushes_and_the_leader_commits() {
    let list = Arc::new(FakeList::default());
    let lease = Arc::new(FakeLease::granting());
    let buffer = buffer_with(
        Arc::clone(&list),
        Arc::clone(&lease),
        MAX_REDIS_BUFFER_DEQUEUE_COUNT,
    );
    let database = FakeDatabase::default();
    let initial = snapshot(&[("2026-01-05", "api", "/v1/chat", 2, 1)]);
    let accumulator = accumulator_with(&initial);
    flush_gateway_requests(&database, &accumulator, Some(&buffer)).await;
    assert_eq!(list.pushed().len(), 1);
    assert_eq!(database.calls(), 1);
    assert!(accumulator.drain().is_empty());
}

#[tokio::test]
async fn the_flush_with_a_buffer_leaves_the_commit_to_the_leader() {
    let list = Arc::new(FakeList::default());
    let buffer = buffer_with(
        Arc::clone(&list),
        Arc::new(FakeLease::denying()),
        MAX_REDIS_BUFFER_DEQUEUE_COUNT,
    );
    let database = FakeDatabase::default();
    let initial = snapshot(&[("2026-01-05", "api", "/v1/chat", 2, 1)]);
    let accumulator = accumulator_with(&initial);
    flush_gateway_requests(&database, &accumulator, Some(&buffer)).await;
    assert_eq!(database.calls(), 0);
    assert!(accumulator.drain().is_empty());
    assert_eq!(list.queued().len(), 1);
}

#[tokio::test]
async fn a_push_failure_restores_the_counts() {
    let list = Arc::new(FakeList::default());
    list.fail_push();
    let buffer = buffer_with(
        Arc::clone(&list),
        Arc::new(FakeLease::granting()),
        MAX_REDIS_BUFFER_DEQUEUE_COUNT,
    );
    let database = FakeDatabase::default();
    let initial = snapshot(&[("2026-01-05", "api", "/v1/chat", 2, 1)]);
    let accumulator = accumulator_with(&initial);
    flush_gateway_requests(&database, &accumulator, Some(&buffer)).await;
    assert_eq!(database.calls(), 0);
    assert_eq!(accumulator.drain(), initial);
}

#[tokio::test]
async fn a_failed_leader_commit_restores_the_unrequeued_rows() {
    let list = Arc::new(FakeList::default());
    list.fail_push_after(1);
    let buffer = buffer_with(
        Arc::clone(&list),
        Arc::new(FakeLease::granting()),
        MAX_REDIS_BUFFER_DEQUEUE_COUNT,
    );
    let database = FakeDatabase::failing();
    let initial = snapshot(&[("2026-01-05", "api", "/v1/chat", 2, 1)]);
    let accumulator = accumulator_with(&initial);
    flush_gateway_requests(&database, &accumulator, Some(&buffer)).await;
    assert_eq!(database.calls(), 1);
    assert_eq!(accumulator.drain(), initial);
}

#[tokio::test]
async fn the_flush_never_raises_when_the_leader_drain_fails() {
    let list = Arc::new(FakeList::default());
    list.fail_take();
    let buffer = buffer_with(
        Arc::clone(&list),
        Arc::new(FakeLease::granting()),
        MAX_REDIS_BUFFER_DEQUEUE_COUNT,
    );
    let database = FakeDatabase::default();
    let initial = snapshot(&[("2026-01-05", "api", "/v1/chat", 2, 1)]);
    let accumulator = accumulator_with(&initial);
    flush_gateway_requests(&database, &accumulator, Some(&buffer)).await;
    assert_eq!(database.calls(), 0);
    assert!(accumulator.drain().is_empty());
}

#[tokio::test]
async fn an_empty_accumulator_flushes_nothing() {
    let database = FakeDatabase::default();
    let accumulator = GatewayRequestAccumulator::new();
    flush_gateway_requests(&database, &accumulator, None).await;
    assert_eq!(database.calls(), 0);
}

#[tokio::test]
async fn an_empty_accumulator_pushes_nothing() {
    let list = Arc::new(FakeList::default());
    let buffer = buffer_with(
        Arc::clone(&list),
        Arc::new(FakeLease::granting()),
        MAX_REDIS_BUFFER_DEQUEUE_COUNT,
    );
    let database = FakeDatabase::default();
    let accumulator = GatewayRequestAccumulator::new();
    flush_gateway_requests(&database, &accumulator, Some(&buffer)).await;
    assert!(list.pushed().is_empty());
    assert_eq!(database.calls(), 0);
}

#[tokio::test]
async fn the_flush_commits_a_restored_snapshot_on_the_next_run() {
    let database = FakeDatabase::default();
    database.fail();
    let initial = snapshot(&[("2026-01-05", "api", "/v1/chat", 2, 1)]);
    let accumulator = accumulator_with(&initial);
    flush_gateway_requests(&database, &accumulator, None).await;
    assert_eq!(accumulator.drain(), initial);
    let retry = FakeDatabase::default();
    accumulator.restore(&initial);
    flush_gateway_requests(&retry, &accumulator, None).await;
    assert_eq!(retry.calls(), 1);
    assert!(accumulator.drain().is_empty());
}
