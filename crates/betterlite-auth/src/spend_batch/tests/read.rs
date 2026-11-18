use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinSet;
use tokio::task::yield_now;
use tokio::time::timeout;

use crate::spend_batch::batch::{SpendCounterBatch, SpendCounterBatchError, SpendCounterValue};

use super::support::{DebugEvents, FakeRedis, Reply, allow_cache, decimal, keys, refusing_cache};

async fn open_batch(server: &FakeRedis) -> Arc<SpendCounterBatch> {
    SpendCounterBatch::new(allow_cache(server).await)
}

async fn join_read(
    tasks: &mut JoinSet<Result<Option<SpendCounterValue>, SpendCounterBatchError>>,
) -> Option<SpendCounterValue> {
    let joined = timeout(Duration::from_secs(10), tasks.join_next())
        .await
        .expect("the reader finishes before the deadline");
    let Some(Ok(Ok(value))) = joined else {
        panic!("the reader task reports a value");
    };
    value
}

async fn read_once(batch: &Arc<SpendCounterBatch>, counter_key: &str) -> Option<SpendCounterValue> {
    let reading = Arc::clone(batch);
    let owned_key = counter_key.to_owned();
    let mut tasks = JoinSet::new();
    tasks.spawn(async move { reading.read(&owned_key).await });
    join_read(&mut tasks).await
}

async fn read_amount(
    batch: &Arc<SpendCounterBatch>,
    counter_key: &str,
) -> Option<SpendCounterValue> {
    batch.read(counter_key).await.expect("the read succeeds")
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
        read_amount(&batch, "spend:key:a").await,
        Some(SpendCounterValue::Amount(decimal("1")))
    );
    assert_eq!(
        server.mget_keys(),
        [vec!["spend:key:a".to_owned(), "spend:key:b".to_owned()]]
    );
    assert_eq!(
        read_amount(&batch, "spend:key:b").await,
        Some(SpendCounterValue::Amount(decimal("2")))
    );
    assert_eq!(server.mget_keys().len(), 1);
}

#[tokio::test]
async fn unbound_key_is_not_read() {
    let server = FakeRedis::start().await;
    let batch = open_batch(&server).await;
    batch.bind(&keys(&["spend:key:a"]));
    assert_eq!(read_amount(&batch, "spend:key:other").await, None);
    assert!(server.commands_named("MGET").is_empty());
}

#[tokio::test]
async fn present_null_is_an_authoritative_miss() {
    let server = FakeRedis::start().await;
    let batch = open_batch(&server).await;
    batch.bind(&keys(&["spend:key:a"]));
    server.script_mget(Reply::Array(vec![Reply::Nil]));
    assert_eq!(
        read_amount(&batch, "spend:key:a").await,
        Some(SpendCounterValue::Missing)
    );
    assert_eq!(
        read_amount(&batch, "spend:key:a").await,
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
        read_amount(&batch, "spend:key:a").await,
        Some(SpendCounterValue::Amount(decimal("3")))
    );
    assert_eq!(read_amount(&batch, "spend:key:b").await, None);
    assert_eq!(read_amount(&batch, "spend:key:b").await, None);
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
    assert_eq!(read_amount(&batch, "spend:key:a").await, None);
    assert_eq!(read_amount(&batch, "spend:key:b").await, None);
    assert_eq!(server.mget_keys().len(), 1);
    assert!(events.count() >= 1);
}

#[tokio::test]
async fn refused_fetch_is_logged_and_not_retried() {
    let server = FakeRedis::start().await;
    let events = DebugEvents::capture();
    let batch = SpendCounterBatch::new(refusing_cache(&server).await);
    batch.bind(&keys(&["spend:key:a"]));
    assert_eq!(read_amount(&batch, "spend:key:a").await, None);
    let after_first = events.count();
    assert!(after_first >= 1);
    assert_eq!(read_amount(&batch, "spend:key:a").await, None);
    assert_eq!(events.count(), after_first);
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
        read_amount(&batch, "spend:key:a").await,
        Some(SpendCounterValue::Amount(decimal("1")))
    );
    assert_eq!(server.mget_keys(), [vec!["spend:key:b".to_owned()]]);
    assert_eq!(
        read_amount(&batch, "spend:key:b").await,
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
        read_amount(&batch, "spend:key:a").await,
        Some(SpendCounterValue::Amount(decimal("1")))
    );
    batch.bind(&keys(&["spend:key:b"]));
    server.script_mget(Reply::Array(vec![Reply::Bulk("2".to_owned())]));
    assert_eq!(
        read_amount(&batch, "spend:key:b").await,
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
    let first = Arc::clone(&batch);
    let second = Arc::clone(&batch);
    let third = Arc::clone(&batch);
    tasks.spawn(async move { first.read("spend:key:a").await });
    tasks.spawn(async move { second.read("spend:key:a").await });
    tasks.spawn(async move { third.read("spend:key:a").await });
    held.wait_arrived().await;
    held.release();
    assert_eq!(
        join_read(&mut tasks).await,
        Some(SpendCounterValue::Amount(decimal("4")))
    );
    assert_eq!(
        join_read(&mut tasks).await,
        Some(SpendCounterValue::Amount(decimal("4")))
    );
    assert_eq!(
        join_read(&mut tasks).await,
        Some(SpendCounterValue::Amount(decimal("4")))
    );
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
async fn a_completed_fetch_stays_published_through_the_next_one() {
    let server = FakeRedis::start().await;
    let batch = open_batch(&server).await;
    batch.bind(&keys(&["spend:key:a"]));
    server.script_mget(Reply::Array(vec![Reply::Bulk("5".to_owned())]));
    server.script_mget(Reply::Array(vec![Reply::Bulk("6".to_owned())]));
    let mut held = server.hold_next_mget();
    let mut readers = JoinSet::new();
    let first = Arc::clone(&batch);
    readers.spawn(async move { first.read("spend:key:a").await });
    held.wait_arrived().await;
    batch.bind(&keys(&["spend:key:b"]));
    let mut waiters = JoinSet::new();
    let second = Arc::clone(&batch);
    waiters.spawn(async move { second.read("spend:key:a").await });
    yield_now().await;
    held.release();
    assert_eq!(
        join_read(&mut readers).await,
        Some(SpendCounterValue::Amount(decimal("5")))
    );
    assert_eq!(
        join_read(&mut waiters).await,
        Some(SpendCounterValue::Amount(decimal("5")))
    );
    assert_eq!(
        read_amount(&batch, "spend:key:b").await,
        Some(SpendCounterValue::Amount(decimal("6")))
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
async fn a_cancelled_fetcher_releases_its_waiters() {
    let server = FakeRedis::start().await;
    let batch = open_batch(&server).await;
    batch.bind(&keys(&["spend:key:a"]));
    server.script_mget(Reply::Array(vec![Reply::Bulk("7".to_owned())]));
    let mut held = server.hold_next_mget();
    let mut readers = JoinSet::new();
    let reader_batch = Arc::clone(&batch);
    let abort_handle = readers.spawn(async move { reader_batch.read("spend:key:a").await });
    held.wait_arrived().await;
    let mut waiting_reads = JoinSet::new();
    let waiting_batch = Arc::clone(&batch);
    waiting_reads.spawn(async move { waiting_batch.read("spend:key:a").await });
    yield_now().await;
    abort_handle.abort();
    let cancelled = readers.join_next().await.expect("the fetcher runs");
    assert!(cancelled.is_err(), "the first reader is aborted");
    held.release();
    assert_eq!(join_read(&mut waiting_reads).await, None);
    assert_eq!(server.mget_keys().len(), 1);
}

#[tokio::test]
async fn overlapping_fetches_never_strand_a_late_reader() {
    let server = FakeRedis::start().await;
    let batch = open_batch(&server).await;
    batch.bind(&keys(&["spend:key:a"]));
    server.script_mget(Reply::Array(vec![Reply::Bulk("2".to_owned())]));
    server.script_mget(Reply::Array(vec![Reply::Bulk("2".to_owned())]));
    let mut held = server.hold_next_mget();
    let mut readers = JoinSet::new();
    let reader_batch = Arc::clone(&batch);
    let abort_handle = readers.spawn(async move { reader_batch.read("spend:key:a").await });
    held.wait_arrived().await;
    batch.bind(&keys(&["spend:key:b"]));
    let mut waiting_reads = JoinSet::new();
    let waiting_batch = Arc::clone(&batch);
    waiting_reads.spawn(async move { waiting_batch.read("spend:key:a").await });
    yield_now().await;
    abort_handle.abort();
    let abort_error = readers
        .join_next()
        .await
        .expect("the fetcher runs")
        .expect_err("the fetcher task is aborted");
    assert!(abort_error.is_cancelled());
    held.release();
    assert_eq!(join_read(&mut waiting_reads).await, None);
    assert_eq!(
        read_amount(&batch, "spend:key:b").await,
        Some(SpendCounterValue::Amount(decimal("2")))
    );
    batch.bind(&keys(&["spend:key:c"]));
    server.script_mget(Reply::Array(vec![Reply::Bulk("3".to_owned())]));
    assert_eq!(
        read_amount(&batch, "spend:key:c").await,
        Some(SpendCounterValue::Amount(decimal("3")))
    );
    assert_eq!(server.mget_keys().len(), 3);
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
