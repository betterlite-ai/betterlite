use std::sync::Arc;

use betterlite_redis::request_batch::lock;
use tokio::task::JoinSet;

use crate::spend_counter_batch::batch::{CounterValue, SpendCounterBatch};
use crate::spend_counter_batch::scope::SpendCounterScope;

use super::support::{FakeRedis, Reply, allow_cache, decimal, keys};

async fn open_batch(server: &FakeRedis) -> Arc<SpendCounterBatch> {
    SpendCounterBatch::new(allow_cache(server).await)
}

#[tokio::test]
async fn record_binds_and_marks_fetched() {
    let server = FakeRedis::start().await;
    let cache = allow_cache(&server).await;
    let scope = SpendCounterScope::default();
    let _guard = scope.enter(Some(&cache), &keys(&[]));
    scope.record_spend_counter_value("spend:key:a", decimal("5"));
    let batch = scope
        .active_spend_counter_batch()
        .expect("the active batch");
    {
        let state = lock(&batch.state);
        assert!(state.keys.contains("spend:key:a"));
        assert!(state.fetched.contains("spend:key:a"));
        assert_eq!(state.loaded.get("spend:key:a"), Some(&Some(decimal("5"))));
    };
    assert_eq!(
        scope.read_spend_counter("spend:key:a").await.unwrap(),
        Some(CounterValue::Amount(decimal("5")))
    );
    assert!(server.commands_named("MGET").is_empty());
}

#[tokio::test]
async fn closed_record_and_bind_noop() {
    let server = FakeRedis::start().await;
    let batch = open_batch(&server).await;
    batch.close();
    batch.bind(&keys(&["spend:key:a"]));
    batch.record("spend:key:b", decimal("1"));
    {
        let state = lock(&batch.state);
        assert!(state.keys.is_empty());
        assert!(state.fetched.is_empty());
        assert!(state.loaded.is_empty());
        drop(state);
    };
}

#[tokio::test]
async fn forget_clears_even_closed() {
    let server = FakeRedis::start().await;
    let cache = allow_cache(&server).await;
    let scope = SpendCounterScope::default();
    let _guard = scope.enter(Some(&cache), &keys(&["spend:key:a"]));
    let batch = scope
        .active_spend_counter_batch()
        .expect("the active batch");
    batch.record("spend:key:a", decimal("5"));
    batch.close();
    scope.forget_spend_counter("spend:key:a");
    {
        let state = lock(&batch.state);
        assert!(state.keys.is_empty());
        assert!(state.fetched.is_empty());
        assert!(state.loaded.is_empty());
        drop(state);
    };
    assert!(batch.read("spend:key:a").await.unwrap().is_none());
}

#[tokio::test]
async fn pending_reply_does_not_rebind_forgotten_key() {
    let server = FakeRedis::start().await;
    let batch = open_batch(&server).await;
    batch.bind(&keys(&["spend:key:a"]));
    server.script_mget(Reply::Array(vec![Reply::Bulk("6".to_owned())]));
    let mut held = server.hold_next_mget();
    let mut tasks = JoinSet::new();
    let reading = Arc::clone(&batch);
    tasks.spawn(async move { reading.read("spend:key:a").await });
    held.wait_arrived().await;
    batch.forget("spend:key:a");
    held.release();
    let value = tasks.join_next().await.unwrap().unwrap().unwrap();
    assert_eq!(value, Some(CounterValue::Amount(decimal("6"))));
    {
        let state = lock(&batch.state);
        assert_eq!(state.loaded.get("spend:key:a"), Some(&Some(decimal("6"))));
        assert!(!state.keys.contains("spend:key:a"));
        drop(state);
    };
    assert!(batch.read("spend:key:a").await.unwrap().is_none());
}

#[tokio::test]
async fn rebound_forgotten_key_keeps_loaded_merge_precedence() {
    let server = FakeRedis::start().await;
    let batch = open_batch(&server).await;
    batch.bind(&keys(&["spend:key:a"]));
    server.script_mget(Reply::Array(vec![Reply::Bulk("6".to_owned())]));
    let mut held = server.hold_next_mget();
    let mut tasks = JoinSet::new();
    let reading = Arc::clone(&batch);
    tasks.spawn(async move { reading.read("spend:key:a").await });
    held.wait_arrived().await;
    batch.forget("spend:key:a");
    held.release();
    tasks.join_next().await.unwrap().unwrap().unwrap();
    batch.bind(&keys(&["spend:key:a"]));
    server.script_mget(Reply::Array(vec![Reply::Bulk("9".to_owned())]));
    assert_eq!(
        batch.read("spend:key:a").await.unwrap(),
        Some(CounterValue::Amount(decimal("6")))
    );
    assert_eq!(server.commands_named("MGET").len(), 2);
}
