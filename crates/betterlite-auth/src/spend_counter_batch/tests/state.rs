use std::sync::Arc;

use betterlite_redis::request_batch::batches::POST_CALL_FLUSH_DEADLINE;
use betterlite_redis::request_batch::lock;
use betterlite_redis::request_batch::scope::request_redis_batch_scope;
use rstest::rstest;
use tokio::task::JoinSet;

use crate::spend_counter_batch::batch::{BatchPhase, CounterValue, SpendCounterBatch};
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
    let batch = lock(&scope.active).clone().expect("the active batch");
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
    lock(&batch.state).phase = BatchPhase::Closed;
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
    let batch = lock(&scope.active).clone().expect("the active batch");
    batch.record("spend:key:a", decimal("5"));
    lock(&batch.state).phase = BatchPhase::Closed;
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

#[derive(Clone, Copy, Debug)]
enum Pipeline {
    Direct,
    RequestBatch,
}

#[derive(Clone, Copy, Debug)]
enum DuringFetch {
    Forget,
    ForgetAndRebind,
    Record,
}

struct RaceReads {
    inflight: Option<CounterValue>,
    next: Option<CounterValue>,
    other: Option<CounterValue>,
}

async fn race(server: &FakeRedis, during: DuringFetch) -> RaceReads {
    let batch = open_batch(server).await;
    batch.bind(&keys(&["spend:key:a", "spend:key:b"]));
    server.script_mget(Reply::Array(vec![
        Reply::Bulk("6".to_owned()),
        Reply::Bulk("8".to_owned()),
    ]));
    server.script_mget(Reply::Array(vec![Reply::Bulk("9".to_owned())]));
    let mut held = server.hold_next_mget();
    let mut tasks = JoinSet::new();
    let reading = Arc::clone(&batch);
    tasks.spawn(async move { reading.read("spend:key:a").await });
    held.wait_arrived().await;
    match during {
        DuringFetch::Forget => batch.forget("spend:key:a"),
        DuringFetch::ForgetAndRebind => {
            batch.forget("spend:key:a");
            batch.bind(&keys(&["spend:key:a"]));
        }
        DuringFetch::Record => batch.record("spend:key:a", decimal("5")),
    }
    held.release();
    let inflight = tasks.join_next().await.unwrap().unwrap().unwrap();
    let next = batch.read("spend:key:a").await.unwrap();
    let other = batch.read("spend:key:b").await.unwrap();
    RaceReads {
        inflight,
        next,
        other,
    }
}

#[rstest]
#[case::forget_during_inflight_fetch_drops_the_old_reply(
    Pipeline::Direct,
    DuringFetch::Forget,
    None,
    None
)]
#[case::forget_during_request_batch_fetch_drops_the_old_reply(
    Pipeline::RequestBatch,
    DuringFetch::Forget,
    None,
    None
)]
#[case::rebound_key_takes_the_newer_reply(
    Pipeline::Direct,
    DuringFetch::ForgetAndRebind,
    None,
    Some(CounterValue::Amount(decimal("9")))
)]
#[case::recorded_write_outranks_an_older_reply(
    Pipeline::RequestBatch,
    DuringFetch::Record,
    Some(CounterValue::Amount(decimal("5"))),
    Some(CounterValue::Amount(decimal("5")))
)]
#[tokio::test]
async fn reply_asked_before_a_forget(
    #[case] pipeline: Pipeline,
    #[case] during: DuringFetch,
    #[case] inflight: Option<CounterValue>,
    #[case] next: Option<CounterValue>,
) {
    let server = FakeRedis::start().await;
    let reads = match pipeline {
        Pipeline::Direct => race(&server, during).await,
        Pipeline::RequestBatch => {
            request_redis_batch_scope(POST_CALL_FLUSH_DEADLINE, race(&server, during))
                .await
                .1
        }
    };
    assert_eq!(reads.inflight, inflight);
    assert_eq!(reads.next, next);
    assert_eq!(reads.other, Some(CounterValue::Amount(decimal("8"))));
}
