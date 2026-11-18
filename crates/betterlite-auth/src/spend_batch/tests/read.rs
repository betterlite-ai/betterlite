use std::sync::Arc;
use std::time::Duration;

use betterlite_redis::request_batch::lock;
use tokio::task::JoinSet;
use tokio::time::timeout;

use crate::spend_batch::batch::{BatchPhase, SpendCounterBatch, SpendCounterValue};

use super::support::{DebugEvents, FakeRedis, Reply, allow_cache, decimal, keys, refusing_cache};

async fn open_batch(server: &FakeRedis) -> Arc<SpendCounterBatch> {
    SpendCounterBatch::new(allow_cache(server).await)
}

// a broken waiter would otherwise hang the mutant run instead of failing this test
async fn join_read(tasks: &mut JoinSet<Option<SpendCounterValue>>) -> Option<SpendCounterValue> {
    timeout(Duration::from_secs(10), tasks.join_next())
        .await
        .expect("the reader returns")
        .expect("a reader task runs")
        .expect("the reader task joins")
}

async fn read_once(batch: &Arc<SpendCounterBatch>, counter_key: &str) -> Option<SpendCounterValue> {
    let reading = Arc::clone(batch);
    let mut tasks = JoinSet::new();
    tasks.spawn(async move { reading.read(counter_key).await });
    join_read(&mut tasks).await
}

#[tokio::test]
async fn first_read_fetches_every_bound_key_sorted() {
    let server = FakeRedis::start().await;
    let batch = open_batch(&server).await;
    batch.bind(&keys(&["spend:key:b", "spend:key:a"]));
    server.script_mget(Reply::Array(vec![
        Reply::Bulk("1".to_owned()),
        Reply::Bulk("2".to_owned()),
    ]));
    assert_eq!(
        batch.read("spend:key:a").await,
        Some(SpendCounterValue::Amount(decimal("1")))
    );
    assert_eq!(
        server.mget_keys(),
        [vec!["spend:key:a".to_owned(), "spend:key:b".to_owned()]]
    );
    assert_eq!(
        batch.read("spend:key:b").await,
        Some(SpendCounterValue::Amount(decimal("2")))
    );
    assert_eq!(server.mget_keys().len(), 1);
}

#[tokio::test]
async fn unbound_key_is_not_read() {
    let server = FakeRedis::start().await;
    let batch = open_batch(&server).await;
    batch.bind(&keys(&["spend:key:a"]));
    assert_eq!(batch.read("spend:key:other").await, None);
    assert!(server.commands_named("MGET").is_empty());
}

#[tokio::test]
async fn present_null_is_an_authoritative_miss() {
    let server = FakeRedis::start().await;
    let batch = open_batch(&server).await;
    batch.bind(&keys(&["spend:key:a"]));
    server.script_mget(Reply::Array(vec![Reply::Nil]));
    assert_eq!(
        batch.read("spend:key:a").await,
        Some(SpendCounterValue::Missing)
    );
    assert_eq!(
        batch.read("spend:key:a").await,
        Some(SpendCounterValue::Missing)
    );
    assert_eq!(server.mget_keys().len(), 1);
}

#[tokio::test]
async fn absent_key_falls_back_without_a_second_mget() {
    let server = FakeRedis::start().await;
    let batch = open_batch(&server).await;
    batch.bind(&keys(&["spend:key:a", "spend:key:b"]));
    server.script_mget(Reply::Array(vec![Reply::Bulk("3".to_owned())]));
    assert_eq!(
        batch.read("spend:key:a").await,
        Some(SpendCounterValue::Amount(decimal("3")))
    );
    assert_eq!(batch.read("spend:key:b").await, None);
    assert_eq!(batch.read("spend:key:b").await, None);
    assert_eq!(server.mget_keys().len(), 1);
}

#[tokio::test]
async fn malformed_value_drops_the_whole_result() {
    let server = FakeRedis::start().await;
    let events = DebugEvents::capture();
    let batch = open_batch(&server).await;
    batch.bind(&keys(&["spend:key:a", "spend:key:b"]));
    server.script_mget(Reply::Array(vec![
        Reply::Bulk("2".to_owned()),
        Reply::Bulk("[1]".to_owned()),
    ]));
    assert_eq!(batch.read("spend:key:a").await, None);
    assert_eq!(batch.read("spend:key:b").await, None);
    assert_eq!(server.mget_keys().len(), 1);
    assert!(events.count() >= 1);
}

#[tokio::test]
async fn refused_fetch_is_logged_and_not_retried() {
    let server = FakeRedis::start().await;
    let events = DebugEvents::capture();
    let batch = SpendCounterBatch::new(refusing_cache(&server).await);
    batch.bind(&keys(&["spend:key:a"]));
    assert_eq!(batch.read("spend:key:a").await, None);
    let after_first = events.count();
    assert!(after_first >= 1);
    assert_eq!(batch.read("spend:key:a").await, None);
    assert_eq!(events.count(), after_first);
    assert!(lock(&batch.state).fetched.contains("spend:key:a"));
    assert!(server.commands_named("MGET").is_empty());
}

#[tokio::test]
async fn record_serves_the_read_without_a_fetch() {
    let server = FakeRedis::start().await;
    let batch = open_batch(&server).await;
    batch.record("spend:key:a", decimal("5"));
    assert_eq!(
        batch.read("spend:key:a").await,
        Some(SpendCounterValue::Amount(decimal("5")))
    );
    assert!(server.commands_named("MGET").is_empty());
}

#[tokio::test]
async fn read_of_a_recorded_key_still_fetches_the_other_bound_keys() {
    let server = FakeRedis::start().await;
    let batch = open_batch(&server).await;
    batch.bind(&keys(&["spend:key:a", "spend:key:b"]));
    batch.record("spend:key:a", decimal("1"));
    server.script_mget(Reply::Array(vec![Reply::Bulk("2".to_owned())]));
    assert_eq!(
        batch.read("spend:key:a").await,
        Some(SpendCounterValue::Amount(decimal("1")))
    );
    assert_eq!(server.mget_keys(), [vec!["spend:key:b".to_owned()]]);
    assert_eq!(
        batch.read("spend:key:b").await,
        Some(SpendCounterValue::Amount(decimal("2")))
    );
}

#[tokio::test]
async fn a_late_bind_enters_the_next_mget() {
    let server = FakeRedis::start().await;
    let batch = open_batch(&server).await;
    batch.bind(&keys(&["spend:key:a"]));
    server.script_mget(Reply::Array(vec![Reply::Bulk("1".to_owned())]));
    assert_eq!(
        batch.read("spend:key:a").await,
        Some(SpendCounterValue::Amount(decimal("1")))
    );
    batch.bind(&keys(&["spend:key:b"]));
    server.script_mget(Reply::Array(vec![Reply::Bulk("2".to_owned())]));
    assert_eq!(
        batch.read("spend:key:b").await,
        Some(SpendCounterValue::Amount(decimal("2")))
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
async fn concurrent_readers_share_one_mget() {
    let server = FakeRedis::start().await;
    let batch = open_batch(&server).await;
    batch.bind(&keys(&["spend:key:a"]));
    server.script_mget(Reply::Array(vec![Reply::Bulk("4".to_owned())]));
    let mut held = server.hold_next_mget();
    let mut tasks = JoinSet::new();
    for _reader in 0..3 {
        let reading = Arc::clone(&batch);
        tasks.spawn(async move { reading.read("spend:key:a").await });
    }
    held.wait_arrived().await;
    held.release();
    for _reader in 0..3 {
        assert_eq!(
            join_read(&mut tasks).await,
            Some(SpendCounterValue::Amount(decimal("4")))
        );
    }
    assert_eq!(server.mget_keys().len(), 1);
}

#[tokio::test]
async fn a_write_during_the_mget_wins_over_the_fetched_value() {
    let server = FakeRedis::start().await;
    let batch = open_batch(&server).await;
    batch.bind(&keys(&["spend:key:a"]));
    server.script_mget(Reply::Array(vec![Reply::Bulk("5".to_owned())]));
    let mut held = server.hold_next_mget();
    let mut tasks = JoinSet::new();
    let reading = Arc::clone(&batch);
    tasks.spawn(async move { reading.read("spend:key:a").await });
    held.wait_arrived().await;
    batch.record("spend:key:a", decimal("9"));
    held.release();
    assert_eq!(
        join_read(&mut tasks).await,
        Some(SpendCounterValue::Amount(decimal("9")))
    );
}

#[tokio::test]
async fn a_cancelled_fetcher_releases_its_waiters() {
    let server = FakeRedis::start().await;
    let batch = open_batch(&server).await;
    batch.bind(&keys(&["spend:key:a"]));
    server.script_mget(Reply::Array(vec![Reply::Bulk("7".to_owned())]));
    let mut held = server.hold_next_mget();
    let mut fetchers = JoinSet::new();
    let fetching = Arc::clone(&batch);
    let fetcher = fetchers.spawn(async move { fetching.read("spend:key:a").await });
    held.wait_arrived().await;
    let mut waiters = JoinSet::new();
    let waiting = Arc::clone(&batch);
    waiters.spawn(async move { waiting.read("spend:key:a").await });
    tokio::task::yield_now().await;
    fetcher.abort();
    let cancelled = fetchers.join_next().await.expect("the fetcher runs");
    assert!(cancelled.is_err(), "the first reader is aborted");
    held.release();
    assert_eq!(join_read(&mut waiters).await, None);
    assert_eq!(server.mget_keys().len(), 1);
}

#[tokio::test]
async fn a_closed_batch_never_fetches() {
    let server = FakeRedis::start().await;
    let batch = open_batch(&server).await;
    batch.bind(&keys(&["spend:key:a"]));
    batch.record("spend:key:a", decimal("5"));
    lock(&batch.state).phase = BatchPhase::Closed;
    assert_eq!(batch.read("spend:key:a").await, None);
    assert!(server.commands_named("MGET").is_empty());
}

#[tokio::test]
async fn a_reader_task_joins_the_shared_result() {
    let server = FakeRedis::start().await;
    let batch = open_batch(&server).await;
    batch.bind(&keys(&["spend:key:a", "spend:key:b"]));
    server.script_mget(Reply::Array(vec![Reply::Bulk("8".to_owned()), Reply::Nil]));
    assert_eq!(
        read_once(&batch, "spend:key:a").await,
        Some(SpendCounterValue::Amount(decimal("8")))
    );
    assert_eq!(
        read_once(&batch, "spend:key:b").await,
        Some(SpendCounterValue::Missing)
    );
    assert_eq!(server.mget_keys().len(), 1);
}
