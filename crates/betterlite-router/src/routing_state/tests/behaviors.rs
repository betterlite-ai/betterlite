use std::collections::BTreeSet;
use std::future::{Future, pending};
use std::mem;
use std::pin::Pin;
use std::sync::{Arc, Mutex, Weak};
use std::thread;
use std::time::Duration;

use async_trait::async_trait;
use rust_decimal::Decimal;
use tokio::task::{JoinSet, yield_now};
use tokio::time::{advance, timeout};

use crate::router_observations::counter::{CounterSink, Increment};
use crate::routing_state::counter::RouterCounters;
use crate::routing_state::delta::store::local_amount;
use crate::routing_state::delta::{Delta, DeltaStore, SyncState};
use crate::routing_state::error::RoutingStateError;
use crate::routing_state::metadata::{
    CallbackChannels, deployment_channel, get_counted_usage_tokens,
};
use crate::routing_state::{KEY_PATTERN_TO_SYNC, MAX_PENDING_DELTAS};
use crate::routing_state_store::RoutingStateStore;
use crate::routing_state_store::test_support::FakeRedis;
use crate::routing_value_store::{RouteIncrement, RouteKey, RoutingValueStore};

use super::flush::{FlushFailure, flush_discards_callbacks};
use super::support::{
    Harness, TestClock, breaker, fill_window, local_store, redis_settings, route_key, same_f64,
    settings, two_entry_batch, usage_batch,
};

const TTL: Duration = Duration::from_secs(60);
const RPM_KEY: &str = "global_router:dep-a:gpt-4o:rpm:00-00";
const TPM_KEY: &str = "global_router:dep-a:gpt-4o:tpm:00-00";

type CaseFuture = Pin<Box<dyn Future<Output = ()>>>;
type Case = fn() -> CaseFuture;

// one table keeps the case names and their futures together; the dispatcher only finds the entry
#[rustfmt::skip]
const CASES: &[(&str, Case)] = &[
    ("callback_before_snapshot", || Box::pin(super::flush::before_snapshot())),
    ("callback_after_snapshot", || Box::pin(super::flush::after_snapshot(1025, 2050, 0, 2, true))),
    ("refill_at_flush_threshold", || Box::pin(super::flush::after_snapshot(1024, 2049, 0, 2, false))),
    ("refill_below_flush_threshold", || Box::pin(super::flush::after_snapshot(1023, 1025, 1023, 1, false))),
    ("callback_after_final_check", || Box::pin(super::flush::after_final_check())),
    ("sync_merges_the_stored_value", || Box::pin(sync_merges_the_stored_value())),
    ("sync_keeps_a_value_redis_is_behind_on", || Box::pin(sync_keeps_a_value_redis_is_behind_on())),
    ("sync_treats_a_missing_reply_as_zero", || Box::pin(sync_treats_a_missing_reply_as_zero())),
    ("failed_push_discards_its_batch", || Box::pin(failed_push_discards_its_batch())),
    ("periodic_sync_writes_on_the_interval", || Box::pin(periodic_sync_writes_on_the_interval())),
    ("no_periodic_task_without_batching", || Box::pin(no_periodic_task_without_batching())),
    ("cleanup_stops_the_periodic_task", || Box::pin(cleanup_stops_the_periodic_task())),
    ("retire_pushes_and_stops_the_task", || Box::pin(retire_pushes_and_stops_the_task())),
    (
        "sync_compresses_repeated_keys_into_one_operation",
        || Box::pin(sync_compresses_repeated_keys_into_one_operation()),
    ),
    (
        "concurrent_increment_during_the_push_is_not_double_counted",
        || Box::pin(concurrent_increment_during_the_push_is_not_double_counted()),
    ),
    (
        "concurrent_increment_under_a_leading_store_is_merged_once",
        || Box::pin(concurrent_increment_under_a_leading_store_is_merged_once()),
    ),
    ("failure_callback_increment_survives_the_merge", || Box::pin(failure_callback_increment_survives_the_merge())),
    ("real_store_smoke", || Box::pin(real_store_smoke())),
    ("callback_saturation_flushes_the_whole_queue", || Box::pin(callback_saturation_flushes_the_whole_queue())),
    ("callback_saturation_honors_the_caller_deadline", || Box::pin(callback_saturation_honors_the_caller_deadline())),
    ("full_queue_callback_flushes_both_entries", || Box::pin(full_queue_callback_flushes_both_entries())),
    ("sync_holding_the_gate_drains_queued_callbacks", || Box::pin(sync_holding_the_gate_drains_queued_callbacks())),
    ("periodic_sync_drains_queued_callbacks", || Box::pin(periodic_sync_drains_queued_callbacks())),
    (
        "scheduled_flush_past_the_deadline_drops_queued_callbacks",
        || Box::pin(flush_discards_callbacks(FlushFailure::Deadline)),
    ),
    ("failed_scheduled_flush_drops_queued_callbacks", || Box::pin(flush_discards_callbacks(FlushFailure::Store))),
    ("cleanup_waits_for_the_scheduled_flush", || Box::pin(cleanup_waits_for_the_scheduled_flush())),
    (
        "saturated_flush_past_the_deadline_drops_its_delta",
        || Box::pin(saturated_flush_past_the_deadline_drops_its_delta()),
    ),
    ("window_increment_returns_and_queues", || Box::pin(window_increment_returns_and_queues())),
    ("window_increment_without_a_store", || Box::pin(window_increment_without_a_store())),
    ("fractional_spend_is_kept_exactly", || Box::pin(fractional_spend_is_kept_exactly())),
    ("spend_beyond_i64_adds", || Box::pin(spend_beyond_i64_adds())),
    ("tracked_key_accessors", || Box::pin(async { tracked_key_accessors(); })),
    ("success_and_failure_counters", || Box::pin(async { success_and_failure_counters(); })),
    ("usage_pipeline_increments_locally_and_queues", || Box::pin(usage_pipeline_increments_locally_and_queues())),
    ("usage_key_merges_the_shared_store_total", || Box::pin(usage_key_merges_the_shared_store_total())),
    ("counter_write_and_read_round_trip", || Box::pin(async { counter_write_and_read_round_trip(); })),
    ("metadata_bucket_selection", || Box::pin(async { metadata_bucket_selection(); })),
    ("key_pattern_is_absent", || Box::pin(async { key_pattern_is_absent(); })),
];

pub(crate) fn run(name: &str) -> Pin<Box<dyn Future<Output = ()>>> {
    let Some((_, case)) = CASES.iter().find(|(case_name, _)| *case_name == name) else {
        panic!("unknown case {name}");
    };
    (*case)()
}

async fn window_increment_returns_and_queues() {
    let harness = Harness::with_store(settings(false, None));
    let first = harness
        .counters
        .increment_window_value(RPM_KEY, Decimal::from(2_i64), TTL)
        .await;
    assert_eq!(first.unwrap(), Decimal::from(2_i64));
    let second = harness
        .counters
        .increment_window_value(RPM_KEY, Decimal::from(3_i64), TTL)
        .await;
    assert_eq!(second.unwrap(), Decimal::from(5_i64));
    assert_eq!(harness.local_amount(RPM_KEY), Some(Decimal::from(5_i64)));
    assert_eq!(harness.pending_len(), 2);
    let increments = [(TPM_KEY.to_owned(), Decimal::from(4_i64))];
    let results = harness
        .counters
        .increment_window_values(&increments, TTL)
        .await
        .unwrap();
    assert_eq!(results, vec![Decimal::from(4_i64)]);
    assert_eq!(
        harness.counters.get_tracked_keys(),
        set(&[RPM_KEY, TPM_KEY])
    );
    assert_eq!(harness.store.calls(), 0);
    assert_eq!(harness.store.amount(RPM_KEY), None);
}

async fn window_increment_without_a_store() {
    let harness = Harness::without_store(settings(false, None));
    let value = harness
        .counters
        .increment_window_value(RPM_KEY, Decimal::from(3_i64), TTL)
        .await;
    assert_eq!(value.unwrap(), Decimal::from(3_i64));
    assert_eq!(harness.pending_len(), 0);
    assert_eq!(harness.store.calls(), 0);
    assert_eq!(harness.counters.get_tracked_keys(), set(&[RPM_KEY]));
}

async fn fractional_spend_is_kept_exactly() {
    let harness = Harness::with_store(settings(false, None));
    let quarter = Decimal::new(25, 2);
    let half = Decimal::new(5, 1);
    harness
        .counters
        .increment_window_value(RPM_KEY, quarter, TTL)
        .await
        .unwrap();
    let total = harness
        .counters
        .increment_window_value(RPM_KEY, half, TTL)
        .await
        .unwrap();
    assert_eq!(total, Decimal::new(75, 2));
    assert!(harness.counters.sync_with_redis().await);
    assert_eq!(harness.store.amount(RPM_KEY), Some(Decimal::new(75, 2)));
    assert_eq!(harness.local_amount(RPM_KEY), Some(Decimal::new(75, 2)));
}

async fn spend_beyond_i64_adds() {
    let harness = Harness::with_store(settings(false, None));
    let large = Decimal::from(i64::MAX);
    let expected = large.saturating_add(large);
    harness
        .counters
        .increment_window_value(RPM_KEY, large, TTL)
        .await
        .unwrap();
    let total = harness
        .counters
        .increment_window_value(RPM_KEY, large, TTL)
        .await
        .unwrap();
    assert_eq!(total, expected);
    assert!(harness.counters.sync_with_redis().await);
    assert_eq!(harness.store.pushed().len(), 1);
    assert_eq!(harness.store.pushed()[0].value, expected);
    assert_eq!(harness.store.amount(RPM_KEY), Some(expected));
    assert_eq!(harness.local_amount(RPM_KEY), Some(expected));
}

fn tracked_key_accessors() {
    let harness = Harness::without_store(settings(false, None));
    harness.counters.track_key("a");
    harness.counters.track_key("a");
    harness.counters.track_key("b");
    assert_eq!(harness.counters.get_tracked_keys(), set(&["a", "b"]));
    assert_eq!(harness.counters.take_tracked_keys(), set(&["a", "b"]));
    assert!(harness.counters.get_tracked_keys().is_empty());
    harness.counters.track_key("c");
    assert_eq!(harness.counters.take_tracked_keys(), set(&["c"]));
    harness.counters.track_key("d");
    harness.counters.reset_tracked_keys();
    assert!(harness.counters.get_tracked_keys().is_empty());
    harness.counters.track_key("e");
    assert_eq!(harness.counters.get_tracked_keys(), set(&["e"]));
}

fn success_and_failure_counters() {
    let harness = Harness::without_store(settings(false, None));
    let key = harness.counters.increment_successes_for_minute("dep-a");
    assert_eq!(key.unwrap(), "dep-a:successes");
    harness
        .counters
        .increment_successes_for_minute("dep-a")
        .unwrap();
    harness
        .counters
        .increment_failures_for_minute("dep-a")
        .unwrap();
    assert_eq!(harness.counters.successes_for_minute("dep-a"), 2);
    assert_eq!(harness.counters.failures_for_minute("dep-a"), 1);
    assert_eq!(harness.counters.read("dep-a:fails"), Some(1));
    assert_eq!(harness.counters.successes_for_minute("dep-b"), 0);
    assert_eq!(harness.counters.failures_for_minute("dep-b"), 0);
}

async fn sync_merges_the_stored_value() {
    let harness = Harness::with_store(settings(false, None));
    harness.store.seed(RPM_KEY, Decimal::from(5_i64));
    harness
        .counters
        .increment_window_value(RPM_KEY, Decimal::from(2_i64), TTL)
        .await
        .unwrap();
    assert!(harness.counters.sync_with_redis().await);
    assert_eq!(harness.store.amount(RPM_KEY), Some(Decimal::from(7_i64)));
    assert_eq!(harness.local_amount(RPM_KEY), Some(Decimal::from(7_i64)));
    assert_eq!(harness.store.calls(), 1);
    assert!(!harness.counters.sync_with_redis().await);
    assert_eq!(harness.store.calls(), 1);
}

async fn sync_keeps_a_value_redis_is_behind_on() {
    let harness = Harness::with_store(settings(false, None));
    harness.store.set_stale(true);
    harness
        .counters
        .increment_window_value(RPM_KEY, Decimal::from(2_i64), TTL)
        .await
        .unwrap();
    assert!(harness.counters.sync_with_redis().await);
    assert_eq!(harness.store.amount(RPM_KEY), Some(Decimal::from(2_i64)));
    assert_eq!(harness.local_amount(RPM_KEY), Some(Decimal::from(2_i64)));
    assert_eq!(harness.counters.get_tracked_keys(), set(&[RPM_KEY]));
}

// a reply shorter than the pushed operations leaves those keys out, and their store value reads as zero
async fn sync_treats_a_missing_reply_as_zero() {
    let harness = Harness::with_store(settings(false, None));
    harness.store.set_short_reply(true);
    harness
        .counters
        .increment_window_value(RPM_KEY, Decimal::from(2_i64), TTL)
        .await
        .unwrap();
    assert!(harness.counters.sync_with_redis().await);
    assert_eq!(harness.store.amount(RPM_KEY), Some(Decimal::from(2_i64)));
    assert_eq!(harness.local_amount(RPM_KEY), Some(Decimal::from(2_i64)));
}

async fn failed_push_discards_its_batch() {
    let harness = Harness::with_store(settings(false, None));
    harness.store.set_down(true);
    harness
        .counters
        .increment_window_value(RPM_KEY, Decimal::from(2_i64), TTL)
        .await
        .unwrap();
    assert!(!harness.counters.sync_with_redis().await);
    assert_eq!(harness.store.calls(), 1);
    assert_eq!(harness.store.amount(RPM_KEY), None);
    harness.store.set_down(false);
    assert!(!harness.counters.sync_with_redis().await);
    assert_eq!(harness.store.calls(), 1);
    assert_eq!(harness.local_amount(RPM_KEY), Some(Decimal::from(2_i64)));
    harness
        .counters
        .increment_window_value(RPM_KEY, Decimal::from(1_i64), TTL)
        .await
        .unwrap();
    assert!(harness.counters.sync_with_redis().await);
    assert_eq!(harness.store.amount(RPM_KEY), Some(Decimal::from(1_i64)));
    assert_eq!(harness.local_amount(RPM_KEY), Some(Decimal::from(3_i64)));
}

async fn periodic_sync_writes_on_the_interval() {
    let harness = Harness::with_store(settings(true, Some(Duration::from_millis(250))));
    harness.counters.start();
    yield_now().await;
    fill_window(&harness.counters, RPM_KEY, 2).await;
    advance(Duration::from_millis(249)).await;
    yield_now().await;
    assert_eq!(harness.store.calls(), 0);
    advance(Duration::from_millis(1)).await;
    yield_now().await;
    assert_eq!(harness.store.amount(RPM_KEY), Some(Decimal::from(2_i64)));
    // a failed tick logs and sleeps again, so the next interval retries
    retry_after_a_failed_tick(&harness).await;
    harness.cleanup().await;
}

async fn retry_after_a_failed_tick(harness: &Harness) {
    harness.store.set_down(true);
    fill_window(&harness.counters, RPM_KEY, 1).await;
    advance(Duration::from_millis(250)).await;
    yield_now().await;
    assert_eq!(harness.store.calls(), 2);
    harness.store.set_down(false);
    fill_window(&harness.counters, RPM_KEY, 3).await;
    advance(Duration::from_millis(250)).await;
    yield_now().await;
    assert_eq!(harness.store.calls(), 3);
    assert_eq!(harness.store.amount(RPM_KEY), Some(Decimal::from(5_i64)));
}

async fn no_periodic_task_without_batching() {
    let harness = Harness::with_store(settings(false, Some(Duration::from_millis(50))));
    harness.counters.start();
    yield_now().await;
    harness
        .counters
        .increment_window_value(RPM_KEY, Decimal::from(1_i64), TTL)
        .await
        .unwrap();
    advance(Duration::from_secs(10)).await;
    yield_now().await;
    assert_eq!(harness.store.calls(), 0);
    harness.cleanup().await;
}

async fn cleanup_stops_the_periodic_task() {
    let harness = Harness::with_store(settings(true, Some(Duration::from_millis(50))));
    harness.counters.start();
    yield_now().await;
    harness.cleanup().await;
    harness
        .counters
        .increment_window_value(RPM_KEY, Decimal::from(1_i64), TTL)
        .await
        .unwrap();
    advance(Duration::from_secs(10)).await;
    yield_now().await;
    assert_eq!(harness.store.calls(), 0);
}

async fn retire_pushes_and_stops_the_task() {
    let harness = Harness::with_store(settings(true, Some(Duration::from_millis(50))));
    harness.counters.start();
    yield_now().await;
    harness
        .counters
        .increment_window_value(RPM_KEY, Decimal::from(2_i64), TTL)
        .await
        .unwrap();
    harness.retire().await;
    assert_eq!(harness.store.amount(RPM_KEY), Some(Decimal::from(2_i64)));
    harness
        .counters
        .increment_window_value(RPM_KEY, Decimal::from(1_i64), TTL)
        .await
        .unwrap();
    advance(Duration::from_secs(1)).await;
    yield_now().await;
    assert_eq!(harness.store.amount(RPM_KEY), Some(Decimal::from(2_i64)));
    harness.cleanup().await;
}

async fn usage_pipeline_increments_locally_and_queues() {
    let harness = Harness::with_store(settings(false, None));
    let increments = usage_increments();
    harness.counters.increment_pipeline(&increments).unwrap();
    assert_eq!(harness.counters.read(TPM_KEY), Some(7));
    assert_eq!(harness.pending_len(), 3);
    assert_eq!(
        harness.counters.get_tracked_keys(),
        set(&[RPM_KEY, TPM_KEY])
    );
    // the sink cannot await the store, so the batch waits for the owner's next sync
    for _turn in 0..4_u32 {
        yield_now().await;
    }
    assert_eq!(harness.store.calls(), 0);
    assert_eq!(harness.store.amount(TPM_KEY), None);
    assert!(harness.counters.sync_with_redis().await);
    assert_eq!(harness.store.amount(TPM_KEY), Some(Decimal::from(7_i64)));
    let local_only = Harness::without_store(settings(false, None));
    local_only.counters.increment_pipeline(&increments).unwrap();
    assert_eq!(local_only.counters.read(TPM_KEY), Some(7));
    assert_eq!(local_only.pending_len(), 0);
    assert!(local_only.counters.get_tracked_keys().is_empty());
}

// LiteLLM copies redis's post-increment total back locally (router.py:8198-8202); the merge does it on the sync
async fn usage_key_merges_the_shared_store_total() {
    let harness = Harness::with_store(settings(false, None));
    harness.store.seed(TPM_KEY, Decimal::from(100_i64));
    harness
        .counters
        .increment_pipeline(&[Increment {
            key: TPM_KEY.to_owned(),
            value: 7,
            ttl: TTL,
        }])
        .unwrap();
    assert_eq!(harness.counters.read(TPM_KEY), Some(7));
    assert!(harness.counters.get_tracked_keys().contains(TPM_KEY));
    assert!(harness.counters.sync_with_redis().await);
    assert_eq!(harness.store.amount(TPM_KEY), Some(Decimal::from(107_i64)));
    assert_eq!(harness.counters.read(TPM_KEY), Some(107));
}

// the callback sink stages the whole batch before requesting the flush
async fn callback_saturation_flushes_the_whole_queue() {
    let harness = Harness::with_store(settings(false, None));
    let sink: &dyn CounterSink = harness.counters.as_ref();
    let one = usage_batch(TPM_KEY, 1);
    for _step in 0..=MAX_PENDING_DELTAS {
        sink.increment_pipeline(&one).unwrap();
    }
    // the delta that found the queue full waits for the flush it scheduled, beside the full queue
    assert_eq!(harness.pending_len(), MAX_PENDING_DELTAS + 1);
    assert_eq!(harness.store.calls(), 0);
    harness.drain_flushes().await;
    assert_eq!(harness.store.calls(), 1);
    assert_eq!(
        harness.store.amount(TPM_KEY),
        Some(Decimal::from(1_025_u64))
    );
    assert_eq!(harness.pending_len(), 0);
    assert_eq!(harness.counters.read(TPM_KEY), Some(1_025));
    // a failing store discards the batch the flush carried, and the local count keeps every increment
    let down = Harness::with_store(settings(false, None));
    down.store.set_down(true);
    let down_sink: &dyn CounterSink = down.counters.as_ref();
    for _step in 0..=MAX_PENDING_DELTAS {
        down_sink.increment_pipeline(&one).unwrap();
    }
    down.drain_flushes().await;
    assert_eq!(down.pending_len(), 0);
    assert_eq!(down.store.calls(), 1);
    assert_eq!(down.counters.read(TPM_KEY), Some(1_025));
    assert_eq!(down.store.amount(TPM_KEY), None);
}

// cleanup joins a flush the sink scheduled, so the batch reaches the store before cleanup returns
async fn cleanup_waits_for_the_scheduled_flush() {
    let harness = Harness::with_store(settings(false, None));
    let sink: &dyn CounterSink = harness.counters.as_ref();
    sink.increment_pipeline(&usage_batch(TPM_KEY, MAX_PENDING_DELTAS + 1))
        .unwrap();
    harness.cleanup().await;
    assert_eq!(harness.store.calls(), 1);
    assert_eq!(
        harness.store.amount(TPM_KEY),
        Some(Decimal::from(1_025_u64))
    );
    assert_eq!(harness.pending_len(), 0);
}

// a callback returns inside its caller's deadline even while a stalled flush holds the store
async fn callback_saturation_honors_the_caller_deadline() {
    let harness = Harness::with_store(settings(false, None));
    harness.store.set_stalled(true);
    let sink: &dyn CounterSink = harness.counters.as_ref();
    sink.increment_pipeline(&usage_batch(TPM_KEY, MAX_PENDING_DELTAS))
        .unwrap();
    // the delta past the cap schedules its flush and returns before the store is even called
    sink.increment_pipeline(&usage_batch(TPM_KEY, 1)).unwrap();
    yield_now().await;
    assert_eq!(harness.store.calls(), 1);
    // the queue fills again while the store gate remains held
    sink.increment_pipeline(&usage_batch(TPM_KEY, MAX_PENDING_DELTAS))
        .unwrap();
    assert_eq!(harness.pending_len(), MAX_PENDING_DELTAS);
    let counters = Arc::clone(&harness.counters);
    let one = usage_batch(TPM_KEY, 1);
    let outcome = timeout(Duration::from_millis(50), async move {
        counters.increment_pipeline(&one).unwrap();
    })
    .await;
    outcome.expect("the callback returned inside its caller deadline");
    assert_eq!(harness.store.calls(), 1);
    // the successor request guarantees the new saturated batch is not stranded
    harness.store.set_stalled(false);
    harness.store.release();
    harness.drain_flushes().await;
    assert_eq!(harness.store.calls(), 2);
    assert_eq!(
        harness.store.amount(TPM_KEY),
        Some(Decimal::from(2_050_u64))
    );
    assert_eq!(harness.pending_len(), 0);
    assert_eq!(harness.counters.read(TPM_KEY), Some(2_050));
}

// a full queue and a two-entry TPM/RPM callback: both entries reach the store on the one flush the call schedules
async fn full_queue_callback_flushes_both_entries() {
    let harness = Harness::with_store(settings(false, None));
    let sink: &dyn CounterSink = harness.counters.as_ref();
    sink.increment_pipeline(&usage_batch(TPM_KEY, MAX_PENDING_DELTAS))
        .unwrap();
    sink.increment_pipeline(&two_entry_batch(TPM_KEY, RPM_KEY, TTL))
        .unwrap();
    assert_eq!(harness.pending_len(), MAX_PENDING_DELTAS + 2);
    harness.drain_flushes().await;
    assert_eq!(harness.store.calls(), 1);
    assert_eq!(
        harness.store.amount(TPM_KEY),
        Some(Decimal::from(1_025_u64))
    );
    assert_eq!(harness.store.amount(RPM_KEY), Some(Decimal::ONE));
    assert_eq!(harness.counters.read(TPM_KEY), Some(1_025));
    assert_eq!(harness.counters.read(RPM_KEY), Some(1));
    assert_eq!(harness.pending_len(), 0);
}

// a saturated callback requests a successor while an explicit sync holds the gate
async fn sync_holding_the_gate_drains_queued_callbacks() {
    let harness = Harness::with_store(settings(false, None));
    harness.store.set_stalled(true);
    let sink: &dyn CounterSink = harness.counters.as_ref();
    sink.increment_pipeline(&usage_batch(TPM_KEY, MAX_PENDING_DELTAS))
        .unwrap();
    let owner = Arc::clone(&harness.counters);
    let mut syncs = JoinSet::new();
    let _task = syncs.spawn(async move { owner.sync_with_redis().await });
    yield_now().await;
    assert_eq!(harness.store.calls(), 1);
    // the queue fills behind the sync and requests a successor
    sink.increment_pipeline(&usage_batch(TPM_KEY, MAX_PENDING_DELTAS + 1))
        .unwrap();
    assert_eq!(harness.pending_len(), MAX_PENDING_DELTAS + 1);
    harness.store.set_stalled(false);
    harness.store.release();
    assert!(syncs.join_next().await.unwrap().unwrap());
    harness.drain_flushes().await;
    assert_eq!(harness.store.calls(), 2);
    assert_eq!(
        harness.store.amount(TPM_KEY),
        Some(Decimal::from(2_049_u64))
    );
    assert_eq!(harness.pending_len(), 0);
    assert_eq!(harness.counters.read(TPM_KEY), Some(2_049));
}

// periodic synchronization and a scheduled successor drain the callback batch
async fn periodic_sync_drains_queued_callbacks() {
    let harness = Harness::with_store(settings(true, Some(Duration::from_millis(250))));
    harness.store.set_stalled(true);
    harness.counters.start();
    yield_now().await;
    // the first tick finds an empty queue and sleeps, while the queue fills behind it
    fill_window(&harness.counters, RPM_KEY, MAX_PENDING_DELTAS).await;
    advance(Duration::from_millis(250)).await;
    yield_now().await;
    // the tick detaches its batch while the next callback fills the queue
    assert_eq!(harness.store.calls(), 1);
    harness
        .counters
        .increment_pipeline(&usage_batch(TPM_KEY, MAX_PENDING_DELTAS))
        .unwrap();
    harness
        .counters
        .increment_pipeline(&two_entry_batch(TPM_KEY, RPM_KEY, TTL))
        .unwrap();
    assert_eq!(harness.pending_len(), MAX_PENDING_DELTAS + 2);
    harness.store.set_stalled(false);
    harness.store.release();
    yield_now().await;
    // both callback entries reach storage once it answers
    assert_eq!(harness.store.calls(), 2);
    assert_eq!(
        harness.store.amount(RPM_KEY),
        Some(Decimal::from(1_025_u64))
    );
    assert_eq!(
        harness.store.amount(TPM_KEY),
        Some(Decimal::from(1_025_u64))
    );
    assert_eq!(harness.pending_len(), 0);
    assert_eq!(harness.counters.read(TPM_KEY), Some(1_025));
    harness.cleanup().await;
}

// a store that never answers: the one second deadline ends the caller's wait and the new delta is dropped
async fn saturated_flush_past_the_deadline_drops_its_delta() {
    let clock = TestClock::new();
    let local = Arc::new(local_store(&clock));
    let counters = RouterCounters::new(
        settings(false, None),
        Arc::clone(&local),
        Some(Arc::new(HangingStore)),
    );
    for _step in 0..MAX_PENDING_DELTAS {
        counters
            .increment_window_value(RPM_KEY, Decimal::ONE, TTL)
            .await
            .unwrap();
    }
    counters
        .increment_window_value(RPM_KEY, Decimal::ONE, TTL)
        .await
        .unwrap();
    assert_eq!(counters.read(RPM_KEY), Some(1_025));
    assert_eq!(pending_len(&counters), 0);
}

struct HangingStore;

#[async_trait]
impl DeltaStore for HangingStore {
    async fn increment_pipeline(
        &self,
        _deltas: &[Delta],
    ) -> Result<Vec<Decimal>, RoutingStateError> {
        pending::<Result<Vec<Decimal>, RoutingStateError>>().await
    }
}

// the failure callback's increment runs on another thread while a sync reconciles the same key
async fn failure_callback_increment_survives_the_merge() {
    let (counters, store, _local) = racing_counters();
    store.seed(Decimal::from(100_i64));
    counters
        .increment_window_value(RPM_KEY, Decimal::from(7_i64), TTL)
        .await
        .unwrap();
    *store.failure_during_push.lock().unwrap() = true;
    assert!(counters.sync_with_redis().await);
    // redis holds 107, and the merge adds its growth to the 8 the callback left behind
    assert_eq!(store.amount(), Decimal::from(107_i64));
    assert_eq!(counters.read(RPM_KEY), Some(108));
    assert_eq!(pending_len(&counters), 0);
}

async fn sync_compresses_repeated_keys_into_one_operation() {
    let harness = Harness::with_store(settings(false, None));
    let first_ttl = Duration::from_secs(60);
    let later_ttl = Duration::from_secs(90);
    let other_ttl = Duration::from_secs(30);
    harness
        .counters
        .increment_window_value(RPM_KEY, Decimal::from(2_i64), first_ttl)
        .await
        .unwrap();
    harness
        .counters
        .increment_window_value(RPM_KEY, Decimal::from(3_i64), later_ttl)
        .await
        .unwrap();
    harness
        .counters
        .increment_window_value(TPM_KEY, Decimal::ONE, other_ttl)
        .await
        .unwrap();
    assert!(harness.counters.sync_with_redis().await);
    let pushed = harness.store.pushed();
    assert_eq!(pushed.len(), 2);
    assert_eq!(pushed[0].key.as_ref(), RPM_KEY);
    assert_eq!(pushed[0].value, Decimal::from(5_i64));
    assert_eq!(pushed[0].ttl, first_ttl);
    assert_eq!(pushed[1].key.as_ref(), TPM_KEY);
    assert_eq!(pushed[1].value, Decimal::ONE);
    assert_eq!(pushed[1].ttl, other_ttl);
}

async fn concurrent_increment_during_the_push_is_not_double_counted() {
    let (counters, store, local) = racing_counters();
    counters
        .increment_window_value(RPM_KEY, Decimal::from(2_i64), TTL)
        .await
        .unwrap();
    let _armed = store.during_push.lock().unwrap().replace(Decimal::ONE);
    assert!(counters.sync_with_redis().await);
    assert_eq!(store.amount(), Decimal::from(2_i64));
    // the concurrent increment landed after the batch was detached, so it is queued and local keeps it once
    assert_eq!(
        local_amount(&local, &route_key(RPM_KEY)),
        Decimal::from(3_i64)
    );
    assert_eq!(pending_len(&counters), 1);
    assert!(counters.sync_with_redis().await);
    assert_eq!(store.amount(), Decimal::from(3_i64));
    assert_eq!(
        local_amount(&local, &route_key(RPM_KEY)),
        Decimal::from(3_i64)
    );
    assert_eq!(pending_len(&counters), 0);
}

async fn concurrent_increment_under_a_leading_store_is_merged_once() {
    let (counters, store, local) = racing_counters();
    store.seed(Decimal::from(100_i64));
    counters
        .increment_window_value(RPM_KEY, Decimal::from(7_i64), TTL)
        .await
        .unwrap();
    let _armed = store.during_push.lock().unwrap().replace(Decimal::ONE);
    assert!(counters.sync_with_redis().await);
    // redis holds 107, the merge adds the one increment still queued, and the next push lands it once
    assert_eq!(store.amount(), Decimal::from(107_i64));
    assert_eq!(
        local_amount(&local, &route_key(RPM_KEY)),
        Decimal::from(108_i64)
    );
    assert_eq!(pending_len(&counters), 1);
    assert!(counters.sync_with_redis().await);
    assert_eq!(store.amount(), Decimal::from(108_i64));
    assert_eq!(
        local_amount(&local, &route_key(RPM_KEY)),
        Decimal::from(108_i64)
    );
    assert_eq!(pending_len(&counters), 0);
}

fn usage_increments() -> [Increment; 3] {
    [
        Increment {
            key: TPM_KEY.to_owned(),
            value: 5,
            ttl: TTL,
        },
        Increment {
            key: TPM_KEY.to_owned(),
            value: 2,
            ttl: TTL,
        },
        Increment {
            key: RPM_KEY.to_owned(),
            value: 1,
            ttl: TTL,
        },
    ]
}

fn counter_write_and_read_round_trip() {
    let harness = Harness::without_store(settings(false, None));
    harness.counters.write("counter", 5, Some(TTL));
    assert_eq!(harness.counters.read("counter"), Some(5));
    harness.counters.write("counter", 2, None);
    assert_eq!(harness.counters.read("counter"), Some(2));
    harness.counters.increment("counter", 1, TTL).unwrap();
    assert_eq!(harness.counters.read("counter"), Some(3));
    assert_eq!(harness.counters.read("missing"), None);
}

fn metadata_bucket_selection() {
    let both = channels(
        r#"{"litellm_metadata": {"model_info": {"id": "a"}, "_litellm_router_usage_counted_tokens": 7},
            "metadata": {"model_info": {"id": "b"}}}"#,
    );
    assert_eq!(channel_name(&both), Some("litellm_metadata"));
    assert_eq!(get_counted_usage_tokens(&both), Some(7));
    let fallback = channels(r#"{"metadata": {"model_info": {"id": "b"}}}"#);
    assert_eq!(channel_name(&fallback), Some("metadata"));
    assert_eq!(get_counted_usage_tokens(&fallback), None);
    let absent = channels(r#"{"litellm_metadata": "text", "metadata": {"deployment": "x"}}"#);
    assert_eq!(channel_name(&absent), None);
}

fn key_pattern_is_absent() {
    assert!(KEY_PATTERN_TO_SYNC.is_none());
}

async fn real_store_smoke() {
    let server = FakeRedis::try_start().await.unwrap();
    let clock = TestClock::new();
    let store = Arc::new(
        RoutingStateStore::new(
            &redis_settings(&server, breaker(100)),
            clock.breaker_clock(),
        )
        .unwrap(),
    );
    let local = Arc::new(local_store(&clock));
    let counters = RouterCounters::new(settings(false, None), local, Some(Arc::clone(&store)));
    let key = route_key(RPM_KEY);
    counters
        .increment_window_value(RPM_KEY, Decimal::from(2_i64), TTL)
        .await
        .unwrap();
    assert!(counters.sync_with_redis().await);
    assert!(same_f64(stored(&store, &key).await, 2.0_f64));
    a_failed_push_drops_the_batch(&server, &counters, &store, &key).await;
    recovery_pushes_only_the_new_increment(&counters, &store, &key).await;
}

// the store's outage loses the batch, and the next sync after recovery carries only what came later
async fn a_failed_push_drops_the_batch(
    server: &FakeRedis,
    counters: &RouterCounters<RoutingStateStore>,
    store: &RoutingStateStore,
    key: &RouteKey,
) {
    server.set_down(true);
    counters
        .increment_window_value(RPM_KEY, Decimal::from(4_i64), TTL)
        .await
        .unwrap();
    assert!(!counters.sync_with_redis().await);
    server.set_down(false);
    assert!(!counters.sync_with_redis().await);
    assert!(same_f64(stored(store, key).await, 2.0_f64));
}

async fn recovery_pushes_only_the_new_increment(
    counters: &RouterCounters<RoutingStateStore>,
    store: &RoutingStateStore,
    key: &RouteKey,
) {
    counters
        .increment_window_value(RPM_KEY, Decimal::ONE, TTL)
        .await
        .unwrap();
    assert!(counters.sync_with_redis().await);
    assert!(same_f64(stored(store, key).await, 3.0_f64));
}

async fn stored(store: &RoutingStateStore, key: &RouteKey) -> f64 {
    store
        .increment_float(key, 0.0_f64, None, false)
        .await
        .unwrap()
        .value
}

// a store that answers the push and runs one increment the way another thread's callback can, after the detach
struct RacingStore {
    value: Mutex<Decimal>,
    state: Mutex<Option<Weak<SyncState<Self>>>>,
    counters: Mutex<Option<Weak<RouterCounters<Self>>>>,
    during_push: Mutex<Option<Decimal>>,
    failure_during_push: Mutex<bool>,
}

impl RacingStore {
    fn new() -> Self {
        Self {
            value: Mutex::new(Decimal::ZERO),
            state: Mutex::new(None),
            counters: Mutex::new(None),
            during_push: Mutex::new(None),
            failure_during_push: Mutex::new(false),
        }
    }

    fn seed(&self, value: Decimal) {
        *self.value.lock().unwrap() = value;
    }

    fn amount(&self) -> Decimal {
        *self.value.lock().unwrap()
    }
}

#[async_trait]
impl DeltaStore for RacingStore {
    async fn increment_pipeline(
        &self,
        deltas: &[Delta],
    ) -> Result<Vec<Decimal>, RoutingStateError> {
        let mut value = self.value.lock().unwrap();
        let mut results = Vec::with_capacity(deltas.len());
        for delta in deltas {
            let next = value.saturating_add(delta.value);
            *value = next;
            results.push(next);
        }
        drop(value);
        let racing = self.during_push.lock().unwrap().take();
        if let Some(amount) = racing {
            let racing_state = self.state.lock().unwrap().as_ref().and_then(Weak::upgrade);
            if let Some(state) = racing_state {
                let key = route_key(RPM_KEY);
                let _recorded = state
                    .record_delta(&key, &RouteIncrement::Spend(amount), TTL)
                    .unwrap();
            }
        }
        if mem::take(&mut *self.failure_during_push.lock().unwrap()) {
            let failing_counters = self
                .counters
                .lock()
                .unwrap()
                .as_ref()
                .and_then(Weak::upgrade);
            if let Some(counters) = failing_counters {
                thread::scope(|scope| {
                    scope.spawn(move || counters.increment(RPM_KEY, 1, TTL).unwrap());
                });
            }
        }
        Ok(results)
    }
}

fn racing_counters() -> (
    Arc<RouterCounters<RacingStore>>,
    Arc<RacingStore>,
    Arc<RoutingValueStore>,
) {
    let clock = TestClock::new();
    let local = Arc::new(local_store(&clock));
    let store = Arc::new(RacingStore::new());
    let counters = Arc::new(RouterCounters::new(
        settings(false, None),
        Arc::clone(&local),
        Some(Arc::clone(&store)),
    ));
    let _state = store
        .state
        .lock()
        .unwrap()
        .replace(Arc::downgrade(&counters.state));
    let _counters = store
        .counters
        .lock()
        .unwrap()
        .replace(Arc::downgrade(&counters));
    (counters, store, local)
}

fn pending_len<S>(counters: &RouterCounters<S>) -> usize
where
    S: DeltaStore + 'static,
{
    counters.state.pending.lock().unwrap().len()
}

fn channels(text: &str) -> CallbackChannels {
    serde_json::from_str(text).unwrap()
}

fn channel_name(channels: &CallbackChannels) -> Option<&'static str> {
    deployment_channel(channels).map(|(name, _bucket)| name)
}

fn set(keys: &[&str]) -> BTreeSet<String> {
    keys.iter().map(|key| (*key).to_owned()).collect()
}
