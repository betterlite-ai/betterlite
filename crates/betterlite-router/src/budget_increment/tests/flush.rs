use std::sync::Arc;

use rust_decimal::Decimal;
use tokio::task::JoinSet;
use tokio_util::task::TaskTracker;

use super::support::{Fixture, budget, flush, queued};

fn cost(value: i64) -> Decimal {
    Decimal::new(value, 2)
}

#[tokio::test]
async fn a_successful_flush_writes_the_batch_and_clears_it() {
    let fixture = Fixture::new(1_030.0_f64);
    fixture.redis().seed_start("start:a", 1_000.0_f64);
    fixture
        .increment
        .increment_spend_for_key(&budget(60), "spend:a", "start:a", cost(25))
        .await
        .unwrap();
    assert!(flush(&fixture).await);
    assert_eq!(
        fixture.redis().batches(),
        vec![vec![queued("spend:a", cost(25), 30)]]
    );
    assert!(flush(&fixture).await);
    assert_eq!(fixture.redis().batches().len(), 1);
}

#[tokio::test]
async fn a_failed_flush_returns_false_and_keeps_the_batch() {
    let fixture = Fixture::new(1_030.0_f64);
    fixture.redis().seed_start("start:a", 1_000.0_f64);
    fixture
        .increment
        .increment_spend_for_key(&budget(60), "spend:a", "start:a", cost(25))
        .await
        .unwrap();
    fixture.redis().fail(true);
    assert!(!flush(&fixture).await);
    assert!(fixture.redis().batches().is_empty());
    fixture.redis().fail(false);
    assert!(flush(&fixture).await);
    assert_eq!(
        fixture.redis().batches(),
        vec![vec![queued("spend:a", cost(25), 30)]]
    );
}

#[tokio::test]
async fn increments_that_arrive_during_a_failed_flush_merge_before_the_retry() {
    let fixture = Fixture::new(1_000.0_f64);
    fixture.redis().seed_start("start:a", 960.0_f64);
    fixture
        .increment
        .increment_spend_for_key(&budget(60), "spend:a", "start:a", cost(10))
        .await
        .unwrap();
    fixture.redis().fail(true);
    fixture.redis().block(true);
    let tasks = TaskTracker::new();
    let pending = fixture.increment.flush_increments(&tasks);
    tokio::join!(
        async {
            assert!(!pending.await);
        },
        async {
            fixture.redis().entered().notified().await;
            fixture.at(1_010.0_f64);
            fixture
                .increment
                .increment_spend_for_key(&budget(60), "spend:a", "start:a", cost(20))
                .await
                .unwrap();
            fixture.redis().block(false);
            fixture.redis().release();
        }
    );
    assert!(fixture.redis().batches().is_empty());
    fixture.redis().fail(false);
    assert!(fixture.increment.flush_increments(&tasks).await);
    assert_eq!(
        fixture.redis().batches(),
        vec![vec![queued("spend:a", cost(30), 10)]]
    );
}

#[tokio::test]
async fn a_flush_without_redis_returns_true() {
    let fixture = Fixture::memory_only(1_030.0_f64);
    fixture
        .increment
        .increment_spend_for_key(&budget(60), "spend:a", "start:a", cost(25))
        .await
        .unwrap();
    assert!(flush(&fixture).await);
    assert!(flush(&fixture).await);
}

#[tokio::test]
async fn a_closed_tracker_returns_false_without_writing() {
    let fixture = Fixture::new(1_030.0_f64);
    fixture.redis().seed_start("start:a", 1_000.0_f64);
    fixture
        .increment
        .increment_spend_for_key(&budget(60), "spend:a", "start:a", cost(25))
        .await
        .unwrap();
    let closed = TaskTracker::new();
    closed.close();
    assert!(!fixture.increment.flush_increments(&closed).await);
    let open = TaskTracker::new();
    assert!(fixture.increment.flush_increments(&open).await);
    assert_eq!(fixture.redis().batches().len(), 1);
}

#[tokio::test]
async fn a_dropped_caller_leaves_the_flush_running() {
    let fixture = Fixture::new(1_030.0_f64);
    fixture.redis().seed_start("start:a", 1_000.0_f64);
    fixture
        .increment
        .increment_spend_for_key(&budget(60), "spend:a", "start:a", cost(25))
        .await
        .unwrap();
    fixture.redis().block(true);
    let tasks = TaskTracker::new();
    let caller = Arc::clone(&fixture.increment);
    let tracker = tasks.clone();
    let mut callers = JoinSet::new();
    callers.spawn(async move { caller.flush_increments(&tracker).await });
    fixture.redis().entered().notified().await;
    callers.abort_all();
    let _aborted = callers.join_all().await;
    fixture.redis().block(false);
    fixture.redis().release();
    tasks.wait().await;
    assert_eq!(
        fixture.redis().batches(),
        vec![vec![queued("spend:a", cost(25), 30)]]
    );
    assert!(fixture.increment.flush_increments(&tasks).await);
    assert_eq!(fixture.redis().batches().len(), 1);
}
