use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{SyncSender, sync_channel};
use std::thread;
use std::time::{Duration, Instant};

use rust_decimal::Decimal;
use tokio::runtime::Handle;
use tokio::task::{JoinSet, yield_now};
use tokio::time::advance;

use crate::router_observations::counter::CounterSink;
use crate::routing_state::counter::RouterCounters;
use crate::routing_state::delta::{self, FlushGate, SyncOutcome};
use crate::routing_state::{FLUSH_DEADLINE, MAX_PENDING_DELTAS, sync};
use crate::routing_value_store::RouteIncrement;

use super::support::{FakeStore, Harness, route_key, settings, usage_batch};

const KEY: &str = "global_router:dep-a:gpt-4o:tpm:00-00";
const TTL: Duration = Duration::from_secs(60);

pub(super) async fn before_snapshot() {
    let harness = Harness::with_store(settings(false, None));
    let state = &harness.counters.state;
    harness
        .counters
        .increment_pipeline(&usage_batch(KEY, 1025))
        .unwrap();
    let owner = Arc::clone(&harness.counters);
    let runtime = Handle::current();
    let (ready, wait) = sync_channel(1);
    let mut callbacks = JoinSet::new();
    let _task = callbacks.spawn_blocking(move || stage_before_snapshot(&owner, &ready, &runtime));
    wait.recv_timeout(Duration::from_secs(5)).unwrap();
    harness.drain_flushes().await;
    callbacks.join_next().await.unwrap().unwrap();
    harness.drain_flushes().await;
    assert_eq!(harness.store.amount(KEY), Some(Decimal::from(1026_i64)));
    assert_eq!(harness.store.calls(), 1);
    assert_eq!(harness.pending_len(), 0);
    assert!(!state.flush_scheduled.load(Ordering::Acquire));
}

fn stage_before_snapshot(
    owner: &RouterCounters<FakeStore>,
    ready: &SyncSender<()>,
    runtime: &Handle,
) {
    // tracked_keys precedes the snapshot; staging itself needs only the pending lock
    let tracked = owner.state.tracked.lock().unwrap();
    ready.send(()).unwrap();
    let started = Instant::now();
    while owner.state.flush_scheduled.load(Ordering::Acquire) {
        assert!(started.elapsed() < Duration::from_secs(5));
        thread::yield_now();
    }
    let (_value, saturated) = owner
        .state
        .record_delta(&route_key(KEY), &RouteIncrement::Counter(1), TTL)
        .unwrap();
    assert!(saturated);
    let _entered = runtime.enter();
    sync::schedule_flush(&owner.state, &owner.flushes);
    assert!(owner.state.flush_scheduled.load(Ordering::Acquire));
    drop(tracked);
}

pub(super) async fn after_snapshot(
    refill: usize,
    stored: i64,
    remaining: usize,
    calls: usize,
    scheduled: bool,
) {
    let harness = Harness::with_store(settings(false, None));
    let state = &harness.counters.state;
    harness.store.set_stalled(true);
    harness
        .counters
        .increment_pipeline(&usage_batch(KEY, 1025))
        .unwrap();
    yield_now().await;
    assert_eq!(harness.store.calls(), 1);
    assert_eq!(harness.pending_len(), 0);
    assert!(!state.flush_scheduled.load(Ordering::Acquire));
    harness
        .counters
        .increment_pipeline(&usage_batch(KEY, refill))
        .unwrap();
    assert_eq!(state.flush_scheduled.load(Ordering::Acquire), scheduled);
    yield_now().await;
    harness.store.set_stalled(false);
    harness.store.release();
    harness.drain_flushes().await;
    assert_eq!(harness.store.amount(KEY), Some(Decimal::from(stored)));
    assert_eq!(harness.store.calls(), calls);
    assert_eq!(harness.pending_len(), remaining);
    assert!(!state.flush_scheduled.load(Ordering::Acquire));
}

pub(super) async fn after_final_check() {
    let harness = Harness::with_store(settings(false, None));
    let state = &harness.counters.state;
    harness
        .counters
        .increment_pipeline(&usage_batch(KEY, MAX_PENDING_DELTAS))
        .unwrap();
    let gate = FlushGate::acquire(&harness.counters.state.gate, FLUSH_DEADLINE)
        .await
        .unwrap();
    assert_eq!(
        delta::push_and_merge(&harness.counters.state).await,
        SyncOutcome::Pushed
    );
    assert_eq!(harness.pending_len(), 0);
    // hold the completed flush's gate across a new callback after its final empty check
    harness
        .counters
        .increment_pipeline(&usage_batch(KEY, 1025))
        .unwrap();
    yield_now().await;
    assert_eq!(harness.pending_len(), 1025);
    assert_eq!(harness.store.calls(), 1);
    assert!(state.flush_scheduled.load(Ordering::Acquire));
    drop(gate);
    harness.drain_flushes().await;
    assert_eq!(harness.store.amount(KEY), Some(Decimal::from(2049_i64)));
    assert_eq!(harness.pending_len(), 0);
}

pub(super) async fn gate_timeout() {
    let harness = Harness::with_store(settings(false, None));
    let state = &harness.counters.state;
    let gate = FlushGate::acquire(&harness.counters.state.gate, FLUSH_DEADLINE)
        .await
        .unwrap();
    harness
        .counters
        .increment_pipeline(&usage_batch(KEY, 1025))
        .unwrap();
    yield_now().await;
    advance(FLUSH_DEADLINE).await;
    harness.drain_flushes().await;
    assert_eq!(harness.store.calls(), 0);
    assert_eq!(harness.pending_len(), 0);
    assert!(!state.flush_scheduled.load(Ordering::Acquire));
    drop(gate);
    harness
        .counters
        .increment_pipeline(&usage_batch(KEY, 1025))
        .unwrap();
    harness.drain_flushes().await;
    assert_eq!(harness.store.amount(KEY), Some(Decimal::from(1025_i64)));
    assert_eq!(harness.counters.read(KEY), Some(2050));
}

#[derive(Clone, Copy)]
pub(super) enum FlushFailure {
    Deadline,
    Store,
}

// both failure modes discard the detached batches without changing callback results or replaying them
pub(super) async fn flush_discards_callbacks(failure: FlushFailure) {
    let harness = Harness::with_store(settings(false, None));
    harness.store.set_stalled(true);
    let sink: &dyn CounterSink = harness.counters.as_ref();
    sink.increment_pipeline(&usage_batch(KEY, MAX_PENDING_DELTAS + 1))
        .unwrap();
    yield_now().await;
    assert_eq!(harness.store.calls(), 1);
    sink.increment_pipeline(&usage_batch(KEY, MAX_PENDING_DELTAS + 1))
        .unwrap();
    assert_eq!(harness.pending_len(), MAX_PENDING_DELTAS + 1);
    fail_the_flush(&harness, failure).await;
    assert_eq!(harness.pending_len(), 0);
    harness.store.set_stalled(false);
    harness.store.set_down(false);
    assert!(!harness.counters.sync_with_redis().await);
    assert_eq!(harness.store.amount(KEY), None);
    assert_eq!(harness.counters.read(KEY), Some(2_050));
}

// the caller's deadline or the store's refusal ends the flush; only the store one reaches it twice
async fn fail_the_flush(harness: &Harness, failure: FlushFailure) {
    match failure {
        FlushFailure::Deadline => {
            yield_now().await;
            advance(FLUSH_DEADLINE).await;
        }
        FlushFailure::Store => {
            harness.store.set_down(true);
            harness.store.release();
        }
    }
    harness.drain_flushes().await;
    if matches!(failure, FlushFailure::Store) {
        assert_eq!(harness.store.calls(), 2);
    }
}
