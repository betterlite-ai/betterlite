use std::sync::Arc;

use betterlite_redis::request_batch::lock;

use crate::spend_batch::batch::{BatchPhase, SpendCounterBatch, SpendCounterValue};

use super::support::{FakeRedis, allow_cache, decimal, keys};

async fn open_batch(server: &FakeRedis) -> Arc<SpendCounterBatch> {
    SpendCounterBatch::new(allow_cache(server).await)
}

#[tokio::test]
async fn a_new_batch_is_open_and_empty() {
    let server = FakeRedis::start().await;
    let batch = open_batch(&server).await;
    let state = lock(&batch.state);
    assert_eq!(state.phase, BatchPhase::Open);
    assert!(state.keys.is_empty());
    assert!(state.fetched.is_empty());
    assert!(state.loaded.is_empty());
    assert!(state.inflight.is_empty());
    assert_eq!(state.generation, 0);
    assert!(!state.fetching);
    drop(state);
    assert!(batch.is_open());
}

#[tokio::test]
async fn bind_unions_keys_while_open() {
    let server = FakeRedis::start().await;
    let batch = open_batch(&server).await;
    batch.bind(&keys(&["spend:key:a"]));
    batch.bind(&keys(&["spend:key:b", "spend:key:a"]));
    assert_eq!(
        lock(&batch.state).keys,
        keys(&["spend:key:a", "spend:key:b"])
    );
}

#[tokio::test]
async fn record_binds_and_marks_the_key_fetched() {
    let server = FakeRedis::start().await;
    let batch = open_batch(&server).await;
    batch.record("spend:key:a", decimal("5"));
    let state = lock(&batch.state);
    assert!(state.keys.contains("spend:key:a"));
    assert!(state.fetched.contains("spend:key:a"));
    assert_eq!(state.loaded.get("spend:key:a"), Some(&Some(decimal("5"))));
    drop(state);
    assert_eq!(
        batch.read("spend:key:a").await,
        Some(SpendCounterValue::Amount(decimal("5")))
    );
}

#[tokio::test]
async fn a_closed_batch_ignores_bind_and_record() {
    let server = FakeRedis::start().await;
    let batch = open_batch(&server).await;
    lock(&batch.state).phase = BatchPhase::Closed;
    batch.bind(&keys(&["spend:key:a"]));
    batch.record("spend:key:b", decimal("1"));
    let state = lock(&batch.state);
    assert!(state.keys.is_empty());
    assert!(state.fetched.is_empty());
    assert!(state.loaded.is_empty());
    drop(state);
    assert!(!batch.is_open());
    assert_eq!(batch.read("spend:key:b").await, None);
}

#[tokio::test]
async fn forget_clears_the_key_even_when_closed() {
    let server = FakeRedis::start().await;
    let batch = open_batch(&server).await;
    batch.record("spend:key:a", decimal("5"));
    batch.bind(&keys(&["spend:key:b"]));
    lock(&batch.state).phase = BatchPhase::Closed;
    batch.forget("spend:key:a");
    batch.forget("spend:key:b");
    let state = lock(&batch.state);
    assert!(state.keys.is_empty());
    assert!(state.fetched.is_empty());
    assert!(state.loaded.is_empty());
    drop(state);
    assert_eq!(batch.read("spend:key:a").await, None);
}

#[tokio::test]
async fn is_open_follows_the_phase() {
    let server = FakeRedis::start().await;
    let batch = open_batch(&server).await;
    assert!(batch.is_open());
    lock(&batch.state).phase = BatchPhase::Closed;
    assert!(!batch.is_open());
    lock(&batch.state).phase = BatchPhase::Closed;
    assert!(!batch.is_open());
}
