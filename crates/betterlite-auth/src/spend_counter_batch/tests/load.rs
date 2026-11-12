use std::sync::Arc;
use std::time::Duration;

use betterlite_redis::request_batch::batches::POST_CALL_FLUSH_DEADLINE;
use betterlite_redis::request_batch::lock;
use betterlite_redis::request_batch::scope::{
    active_request_redis_batch, request_redis_batch_scope,
};
use tokio::task::JoinSet;
use tokio::time::timeout;

use crate::spend_counter_batch::batch::{
    BatchPhase, CounterValue, SpendCounterBatch, SpendCounterBatchError,
};

use super::support::{DebugEvents, FakeRedis, Reply, allow_cache, decimal, keys, refusing_cache};

async fn open_batch(server: &FakeRedis) -> Arc<SpendCounterBatch> {
    SpendCounterBatch::new(allow_cache(server).await)
}

// a broken waiter would otherwise hang the mutant run instead of failing this test
async fn join_reader(
    tasks: &mut JoinSet<Result<Option<CounterValue>, SpendCounterBatchError>>,
) -> Option<CounterValue> {
    timeout(Duration::from_secs(10), tasks.join_next())
        .await
        .expect("the shared reader returns")
        .expect("a reader task runs")
        .expect("the reader task joins")
        .expect("the reader read succeeds")
}

#[tokio::test]
async fn new_batch_empty_and_open() {
    let server = FakeRedis::start().await;
    let batch = open_batch(&server).await;
    let state = lock(&batch.state);
    assert_eq!(state.phase, BatchPhase::Open);
    assert!(state.keys.is_empty());
    assert!(state.fetched.is_empty());
    assert!(state.loaded.is_empty());
    assert!(state.inflight.is_empty());
    drop(state);
}

#[tokio::test]
async fn closed_or_unbound_read_none() {
    let server = FakeRedis::start().await;
    let batch = open_batch(&server).await;
    assert!(batch.read("spend:key:a").await.unwrap().is_none());
    batch.bind(&keys(&["spend:key:a"]));
    lock(&batch.state).phase = BatchPhase::Closed;
    assert!(batch.read("spend:key:a").await.unwrap().is_none());
    assert!(server.commands_named("MGET").is_empty());
}

#[tokio::test]
async fn present_null_is_authoritative() {
    let server = FakeRedis::start().await;
    let batch = open_batch(&server).await;
    batch.bind(&keys(&["spend:key:a"]));
    server.script_mget(Reply::Array(vec![Reply::Nil]));
    let value = batch.read("spend:key:a").await.unwrap();
    assert_eq!(value, Some(CounterValue::Missing));
}

#[tokio::test]
async fn omitted_key_requires_direct_read() {
    let server = FakeRedis::start().await;
    let batch = open_batch(&server).await;
    batch.bind(&keys(&["spend:key:a", "spend:key:b"]));
    server.script_mget(Reply::Array(vec![Reply::Bulk("3".to_owned())]));
    assert_eq!(
        batch.read("spend:key:a").await.unwrap(),
        Some(CounterValue::Amount(decimal("3")))
    );
    assert!(batch.read("spend:key:b").await.unwrap().is_none());
    assert_eq!(server.commands_named("MGET").len(), 1);
}

#[tokio::test]
async fn first_read_mget_sorted_pending_only() {
    let server = FakeRedis::start().await;
    let batch = open_batch(&server).await;
    batch.bind(&keys(&["spend:key:b", "spend:key:a"]));
    server.script_mget(Reply::Array(vec![
        Reply::Bulk("1".to_owned()),
        Reply::Bulk("2".to_owned()),
    ]));
    assert_eq!(
        batch.read("spend:key:a").await.unwrap(),
        Some(CounterValue::Amount(decimal("1")))
    );
    assert_eq!(
        server.mget_keys(),
        [vec!["spend:key:a".to_owned(), "spend:key:b".to_owned()]]
    );
    assert_eq!(
        batch.read("spend:key:b").await.unwrap(),
        Some(CounterValue::Amount(decimal("2")))
    );
    assert_eq!(server.commands_named("MGET").len(), 1);
}

#[tokio::test]
async fn concurrent_readers_share_load() {
    let server = FakeRedis::start().await;
    let batch = open_batch(&server).await;
    batch.bind(&keys(&["spend:key:a"]));
    server.script_mget(Reply::Array(vec![Reply::Bulk("4".to_owned())]));
    let mut held = server.hold_next_mget();
    let mut tasks = JoinSet::new();
    let first = Arc::clone(&batch);
    let second = Arc::clone(&batch);
    tasks.spawn(async move { first.read("spend:key:a").await });
    tasks.spawn(async move { second.read("spend:key:a").await });
    held.wait_arrived().await;
    held.release();
    let one = join_reader(&mut tasks).await;
    let two = join_reader(&mut tasks).await;
    assert_eq!(one, Some(CounterValue::Amount(decimal("4"))));
    assert_eq!(two, Some(CounterValue::Amount(decimal("4"))));
    assert_eq!(server.mget_keys(), [vec!["spend:key:a".to_owned()]]);
}

#[tokio::test]
async fn keys_marked_fetched_before_await() {
    let server = FakeRedis::start().await;
    let batch = open_batch(&server).await;
    batch.bind(&keys(&["spend:key:a"]));
    server.script_mget(Reply::Array(vec![Reply::Bulk("5".to_owned())]));
    let mut held = server.hold_next_mget();
    let mut tasks = JoinSet::new();
    let reading = Arc::clone(&batch);
    tasks.spawn(async move { reading.read("spend:key:a").await });
    held.wait_arrived().await;
    let fetched = {
        let state = lock(&batch.state);
        state.fetched.contains("spend:key:a")
    };
    assert!(fetched);
    held.release();
    let value = tasks.join_next().await.unwrap().unwrap().unwrap();
    assert_eq!(value, Some(CounterValue::Amount(decimal("5"))));
}

#[tokio::test]
async fn recorded_write_wins_old_reply() {
    let server = FakeRedis::start().await;
    let batch = open_batch(&server).await;
    batch.bind(&keys(&["spend:key:a"]));
    server.script_mget(Reply::Array(vec![Reply::Bulk("3".to_owned())]));
    let mut held = server.hold_next_mget();
    let mut tasks = JoinSet::new();
    let reading = Arc::clone(&batch);
    tasks.spawn(async move { reading.read("spend:key:a").await });
    held.wait_arrived().await;
    batch.record("spend:key:a", decimal("5"));
    held.release();
    let value = tasks.join_next().await.unwrap().unwrap().unwrap();
    assert_eq!(value, Some(CounterValue::Amount(decimal("5"))));
}

#[tokio::test]
async fn failed_fetch_debug_and_no_batch_retry() {
    let server = FakeRedis::start().await;
    let events = DebugEvents::capture();
    let batch = SpendCounterBatch::new(refusing_cache(&server).await);
    batch.bind(&keys(&["spend:key:a"]));
    assert!(batch.read("spend:key:a").await.unwrap().is_none());
    let after_first = events.count();
    assert!(after_first >= 1);
    assert!(batch.read("spend:key:a").await.unwrap().is_none());
    assert_eq!(events.count(), after_first);
}

#[tokio::test]
async fn malformed_map_drops_whole_result() {
    let server = FakeRedis::start().await;
    let events = DebugEvents::capture();
    let batch = open_batch(&server).await;
    batch.bind(&keys(&["spend:key:a", "spend:key:b"]));
    server.script_mget(Reply::Array(vec![
        Reply::Bulk("{\"amount\": 1}".to_owned()),
        Reply::Bulk("2".to_owned()),
    ]));
    assert!(batch.read("spend:key:a").await.unwrap().is_none());
    assert!(batch.read("spend:key:b").await.unwrap().is_none());
    assert!(events.count() >= 1);
    assert_eq!(server.commands_named("MGET").len(), 1);
}

#[tokio::test]
async fn numeric_string_and_bool_counter_values() {
    let server = FakeRedis::start().await;
    let batch = open_batch(&server).await;
    batch.bind(&keys(&[
        "spend:key:flag",
        "spend:key:null",
        "spend:key:number",
        "spend:key:text",
    ]));
    server.script_mget(Reply::Array(vec![
        Reply::Bulk("true".to_owned()),
        Reply::Bulk("null".to_owned()),
        Reply::Bulk("2.5".to_owned()),
        Reply::Bulk("\"1.5\"".to_owned()),
    ]));
    assert_eq!(
        batch.read("spend:key:flag").await.unwrap(),
        Some(CounterValue::Amount(decimal("1")))
    );
    assert_eq!(
        batch.read("spend:key:null").await.unwrap(),
        Some(CounterValue::Missing)
    );
    assert_eq!(
        batch.read("spend:key:number").await.unwrap(),
        Some(CounterValue::Amount(decimal("2.5")))
    );
    assert_eq!(
        batch.read("spend:key:text").await.unwrap(),
        Some(CounterValue::Amount(decimal("1.5")))
    );
}

#[tokio::test]
async fn pending_hook_joins_existing_pipeline() {
    let server = FakeRedis::start().await;
    let (_batches, ()) = request_redis_batch_scope(POST_CALL_FLUSH_DEADLINE, async {
        let cache = allow_cache(&server).await;
        let batch = SpendCounterBatch::new(Arc::clone(&cache));
        batch.bind(&keys(&["spend:key:a"]));
        server.script_mget(Reply::Array(vec![Reply::Nil]));
        server.script_mget(Reply::Array(vec![Reply::Bulk("9".to_owned())]));
        let request_batch = active_request_redis_batch(&cache).expect("the request batch");
        request_batch
            .mget(&["dummy".to_owned()])
            .await
            .expect("the dummy read");
        assert_eq!(server.mget_keys().len(), 2);
        assert_eq!(
            batch.read("spend:key:a").await.unwrap(),
            Some(CounterValue::Amount(decimal("9")))
        );
        assert_eq!(server.mget_keys().len(), 2);
        assert_eq!(server.mget_keys()[1], ["spend:key:a".to_owned()]);
    })
    .await;
}

#[tokio::test]
async fn collect_clears_inflight_before_await() {
    let server = FakeRedis::start().await;
    let (_batches, ()) = request_redis_batch_scope(POST_CALL_FLUSH_DEADLINE, async {
        let cache = allow_cache(&server).await;
        let batch = SpendCounterBatch::new(cache);
        batch.bind(&keys(&["spend:key:a"]));
        server.script_mget(Reply::Array(vec![Reply::Bulk("2".to_owned())]));
        let mut held = server.hold_next_mget();
        let mut tasks = JoinSet::new();
        let reading = Arc::clone(&batch);
        tasks.spawn(async move { reading.read("spend:key:a").await });
        held.wait_arrived().await;
        let inflight = {
            let state = lock(&batch.state);
            state.inflight.len()
        };
        assert_eq!(inflight, 0);
        held.release();
        let value = tasks.join_next().await.unwrap().unwrap().unwrap();
        assert_eq!(value, Some(CounterValue::Amount(decimal("2"))));
    })
    .await;
}

#[tokio::test]
async fn failed_result_does_not_skip_later_results() {
    let server = FakeRedis::start().await;
    let _events = DebugEvents::capture();
    let (_batches, ()) = request_redis_batch_scope(POST_CALL_FLUSH_DEADLINE, async {
        let cache = allow_cache(&server).await;
        let batch = SpendCounterBatch::new(Arc::clone(&cache));
        batch.bind(&keys(&["spend:key:bad"]));
        let request_batch = active_request_redis_batch(&cache).expect("the request batch");
        server.script_mget(Reply::Array(vec![Reply::Nil]));
        server.script_mget(Reply::Error("ERR mget failed".to_owned()));
        request_batch
            .mget(&["dummy-1".to_owned()])
            .await
            .expect("the first dummy read");
        batch.bind(&keys(&["spend:key:good"]));
        server.script_mget(Reply::Array(vec![Reply::Nil]));
        server.script_mget(Reply::Array(vec![Reply::Bulk("4".to_owned())]));
        request_batch
            .mget(&["dummy-2".to_owned()])
            .await
            .expect("the second dummy read");
        assert_eq!(
            batch.read("spend:key:good").await.unwrap(),
            Some(CounterValue::Amount(decimal("4")))
        );
        assert!(batch.read("spend:key:bad").await.unwrap().is_none());
    })
    .await;
}

#[tokio::test]
async fn first_loaded_value_wins_duplicate_reply() {
    let server = FakeRedis::start().await;
    let (_batches, ()) = request_redis_batch_scope(POST_CALL_FLUSH_DEADLINE, async {
        let cache = allow_cache(&server).await;
        let batch = SpendCounterBatch::new(Arc::clone(&cache));
        batch.bind(&keys(&["spend:key:a"]));
        let request_batch = active_request_redis_batch(&cache).expect("the request batch");
        server.script_mget(Reply::Array(vec![Reply::Nil]));
        server.script_mget(Reply::Array(vec![Reply::Bulk("1".to_owned())]));
        request_batch
            .mget(&["dummy-1".to_owned()])
            .await
            .expect("the first dummy read");
        batch.forget("spend:key:a");
        batch.bind(&keys(&["spend:key:a"]));
        server.script_mget(Reply::Array(vec![Reply::Nil]));
        server.script_mget(Reply::Array(vec![Reply::Bulk("2".to_owned())]));
        request_batch
            .mget(&["dummy-2".to_owned()])
            .await
            .expect("the second dummy read");
        assert_eq!(
            batch.read("spend:key:a").await.unwrap(),
            Some(CounterValue::Amount(decimal("1")))
        );
    })
    .await;
}

#[tokio::test]
async fn cancelled_load_propagates() {
    let server = FakeRedis::start().await;
    let (_batches, ()) = request_redis_batch_scope(POST_CALL_FLUSH_DEADLINE, async {
        let cache = allow_cache(&server).await;
        let batch = SpendCounterBatch::new(cache);
        batch.bind(&keys(&["spend:key:a"]));
        batch.declare_pending();
        {
            let state = lock(&batch.state);
            state
                .inflight
                .first()
                .expect("the declared result")
                .cancel();
        };
        let error = batch
            .read("spend:key:a")
            .await
            .expect_err("the cancelled load propagates");
        assert!(matches!(error, SpendCounterBatchError::Cancelled));
    })
    .await;
}

#[tokio::test]
async fn late_keys_join_next_fetch() {
    let server = FakeRedis::start().await;
    let batch = open_batch(&server).await;
    batch.bind(&keys(&["spend:key:a"]));
    server.script_mget(Reply::Array(vec![Reply::Bulk("1".to_owned())]));
    assert_eq!(
        batch.read("spend:key:a").await.unwrap(),
        Some(CounterValue::Amount(decimal("1")))
    );
    batch.bind(&keys(&["spend:key:b"]));
    server.script_mget(Reply::Array(vec![Reply::Bulk("2".to_owned())]));
    assert_eq!(
        batch.read("spend:key:b").await.unwrap(),
        Some(CounterValue::Amount(decimal("2")))
    );
    assert_eq!(
        server.mget_keys(),
        [
            vec!["spend:key:a".to_owned()],
            vec!["spend:key:b".to_owned()]
        ]
    );
}

#[tokio::test]
async fn close_during_read_does_not_abort_it() {
    let server = FakeRedis::start().await;
    let batch = open_batch(&server).await;
    batch.bind(&keys(&["spend:key:a"]));
    server.script_mget(Reply::Array(vec![Reply::Bulk("7".to_owned())]));
    let mut held = server.hold_next_mget();
    let mut tasks = JoinSet::new();
    let reading = Arc::clone(&batch);
    tasks.spawn(async move { reading.read("spend:key:a").await });
    held.wait_arrived().await;
    lock(&batch.state).phase = BatchPhase::Closed;
    held.release();
    let value = tasks.join_next().await.unwrap().unwrap().unwrap();
    assert_eq!(value, Some(CounterValue::Amount(decimal("7"))));
}
