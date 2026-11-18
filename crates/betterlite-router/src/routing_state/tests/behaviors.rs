use std::collections::BTreeSet;
use std::mem;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use async_trait::async_trait;
use rust_decimal::Decimal;

use crate::router_observations::counter::{CounterSink, Increment};
use crate::routing_state::MAX_PENDING_DELTAS;
use crate::routing_state::counter::RouterCounters;
use crate::routing_state::delta::store::local_amount;
use crate::routing_state::delta::{Delta, DeltaStore, SyncState};
use crate::routing_state::error::RoutingStateError;
use crate::routing_state::metadata::{
    CallbackChannels, deployment_channel, get_counted_usage_tokens,
};
use crate::routing_state_store::RoutingStateStore;
use crate::routing_state_store::test_support::FakeRedis;
use crate::routing_value_store::{RouteKey, RoutingValueStore};

use super::support::{
    Harness, TestClock, breaker, local_store, redis_settings, route_key, same_f64, settings,
};

const TTL: Duration = Duration::from_secs(60);
const RPM_KEY: &str = "global_router:dep-a:gpt-4o:rpm:00-00";
const TPM_KEY: &str = "global_router:dep-a:gpt-4o:tpm:00-00";

pub(crate) async fn run(name: &str) {
    match name {
        "window_increment_returns_and_queues" => window_increment_returns_and_queues().await,
        "window_increment_without_a_store" => window_increment_without_a_store().await,
        "fractional_spend_is_kept_exactly" => fractional_spend_is_kept_exactly().await,
        "spend_beyond_i64_adds" => spend_beyond_i64_adds().await,
        "tracked_key_accessors" => tracked_key_accessors(),
        "success_and_failure_counters" => success_and_failure_counters(),
        "sync_merges_the_stored_value" => sync_merges_the_stored_value().await,
        "sync_keeps_a_value_redis_is_behind_on" => sync_keeps_a_value_redis_is_behind_on().await,
        "sync_treats_a_missing_reply_as_zero" => sync_treats_a_missing_reply_as_zero().await,
        "saturated_flush_past_the_deadline_drops_its_delta" => {
            saturated_flush_past_the_deadline_drops_its_delta().await;
        }
        "failed_push_discards_its_batch" => failed_push_discards_its_batch().await,
        "periodic_sync_writes_on_the_interval" => periodic_sync_writes_on_the_interval().await,
        "no_periodic_task_without_batching" => no_periodic_task_without_batching().await,
        "cleanup_stops_the_periodic_task" => cleanup_stops_the_periodic_task().await,
        "retire_pushes_and_stops_the_task" => retire_pushes_and_stops_the_task().await,
        "usage_pipeline_increments_locally_and_queues" => {
            usage_pipeline_increments_locally_and_queues().await;
        }
        "usage_key_merges_the_shared_store_total" => {
            usage_key_merges_the_shared_store_total().await;
        }
        "usage_overflow_flushes_within_the_deadline" => {
            usage_overflow_flushes_within_the_deadline().await;
        }
        "sync_compresses_repeated_keys_into_one_operation" => {
            sync_compresses_repeated_keys_into_one_operation().await;
        }
        "concurrent_increment_during_the_push_is_not_double_counted" => {
            concurrent_increment_during_the_push_is_not_double_counted().await;
        }
        "concurrent_increment_under_a_leading_store_is_merged_once" => {
            concurrent_increment_under_a_leading_store_is_merged_once().await;
        }
        "counter_write_and_read_round_trip" => counter_write_and_read_round_trip(),
        "metadata_bucket_selection" => metadata_bucket_selection(),
        "key_pattern_is_absent" => key_pattern_is_absent(),
        "real_store_smoke" => real_store_smoke().await,
        "usage_saturation_flushes_before_dropping" => {
            usage_saturation_flushes_before_dropping().await;
        }
        "failure_callback_increment_survives_the_merge" => {
            failure_callback_increment_survives_the_merge().await;
        }
        _ => panic!("unknown case {name}"),
    }
}

async fn window_increment_returns_and_queues() {
    let harness = Harness::with_store(settings(false, None));
    let first = harness
        .counters
        .increment_window_value(RPM_KEY, Decimal::from(2), TTL)
        .await;
    assert_eq!(first.unwrap(), Decimal::from(2));
    let second = harness
        .counters
        .increment_window_value(RPM_KEY, Decimal::from(3), TTL)
        .await;
    assert_eq!(second.unwrap(), Decimal::from(5));
    assert_eq!(harness.local_amount(RPM_KEY), Some(Decimal::from(5)));
    assert_eq!(harness.pending_len(), 2);
    let increments = [(TPM_KEY.to_owned(), Decimal::from(4))];
    let results = harness
        .counters
        .increment_window_values(&increments, TTL)
        .await
        .unwrap();
    assert_eq!(results, vec![Decimal::from(4)]);
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
        .increment_window_value(RPM_KEY, Decimal::from(3), TTL)
        .await;
    assert_eq!(value.unwrap(), Decimal::from(3));
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
    harness.store.seed(RPM_KEY, Decimal::from(5));
    harness
        .counters
        .increment_window_value(RPM_KEY, Decimal::from(2), TTL)
        .await
        .unwrap();
    assert!(harness.counters.sync_with_redis().await);
    assert_eq!(harness.store.amount(RPM_KEY), Some(Decimal::from(7)));
    assert_eq!(harness.local_amount(RPM_KEY), Some(Decimal::from(7)));
    assert_eq!(harness.store.calls(), 1);
    assert!(!harness.counters.sync_with_redis().await);
    assert_eq!(harness.store.calls(), 1);
}

async fn sync_keeps_a_value_redis_is_behind_on() {
    let harness = Harness::with_store(settings(false, None));
    harness.store.set_stale(true);
    harness
        .counters
        .increment_window_value(RPM_KEY, Decimal::from(2), TTL)
        .await
        .unwrap();
    assert!(harness.counters.sync_with_redis().await);
    assert_eq!(harness.store.amount(RPM_KEY), Some(Decimal::from(2)));
    assert_eq!(harness.local_amount(RPM_KEY), Some(Decimal::from(2)));
    assert_eq!(harness.counters.get_tracked_keys(), set(&[RPM_KEY]));
}

// a reply shorter than the pushed operations leaves those keys out, and their store value reads as zero
async fn sync_treats_a_missing_reply_as_zero() {
    let harness = Harness::with_store(settings(false, None));
    harness.store.set_short_reply(true);
    harness
        .counters
        .increment_window_value(RPM_KEY, Decimal::from(2), TTL)
        .await
        .unwrap();
    assert!(harness.counters.sync_with_redis().await);
    assert_eq!(harness.store.amount(RPM_KEY), Some(Decimal::from(2)));
    assert_eq!(harness.local_amount(RPM_KEY), Some(Decimal::from(2)));
}

async fn failed_push_discards_its_batch() {
    let harness = Harness::with_store(settings(false, None));
    harness.store.set_down(true);
    harness
        .counters
        .increment_window_value(RPM_KEY, Decimal::from(2), TTL)
        .await
        .unwrap();
    assert!(!harness.counters.sync_with_redis().await);
    assert_eq!(harness.store.calls(), 1);
    assert_eq!(harness.store.amount(RPM_KEY), None);
    harness.store.set_down(false);
    assert!(!harness.counters.sync_with_redis().await);
    assert_eq!(harness.store.calls(), 1);
    assert_eq!(harness.local_amount(RPM_KEY), Some(Decimal::from(2)));
    harness
        .counters
        .increment_window_value(RPM_KEY, Decimal::from(1), TTL)
        .await
        .unwrap();
    assert!(harness.counters.sync_with_redis().await);
    assert_eq!(harness.store.amount(RPM_KEY), Some(Decimal::from(1)));
    assert_eq!(harness.local_amount(RPM_KEY), Some(Decimal::from(3)));
}

async fn periodic_sync_writes_on_the_interval() {
    let harness = Harness::with_store(settings(true, Some(Duration::from_millis(250))));
    harness.counters.start();
    tokio::task::yield_now().await;
    harness
        .counters
        .increment_window_value(RPM_KEY, Decimal::from(2), TTL)
        .await
        .unwrap();
    tokio::time::advance(Duration::from_millis(249)).await;
    tokio::task::yield_now().await;
    assert_eq!(harness.store.calls(), 0);
    tokio::time::advance(Duration::from_millis(1)).await;
    tokio::task::yield_now().await;
    assert_eq!(harness.store.amount(RPM_KEY), Some(Decimal::from(2)));
    harness.counters.cleanup().await;
}

async fn no_periodic_task_without_batching() {
    let harness = Harness::with_store(settings(false, Some(Duration::from_millis(50))));
    harness.counters.start();
    tokio::task::yield_now().await;
    harness
        .counters
        .increment_window_value(RPM_KEY, Decimal::from(1), TTL)
        .await
        .unwrap();
    tokio::time::advance(Duration::from_secs(10)).await;
    tokio::task::yield_now().await;
    assert_eq!(harness.store.calls(), 0);
    harness.counters.cleanup().await;
}

async fn cleanup_stops_the_periodic_task() {
    let harness = Harness::with_store(settings(true, Some(Duration::from_millis(50))));
    harness.counters.start();
    tokio::task::yield_now().await;
    harness.counters.cleanup().await;
    harness
        .counters
        .increment_window_value(RPM_KEY, Decimal::from(1), TTL)
        .await
        .unwrap();
    tokio::time::advance(Duration::from_secs(10)).await;
    tokio::task::yield_now().await;
    assert_eq!(harness.store.calls(), 0);
}

async fn retire_pushes_and_stops_the_task() {
    let harness = Harness::with_store(settings(true, Some(Duration::from_millis(50))));
    harness.counters.start();
    tokio::task::yield_now().await;
    harness
        .counters
        .increment_window_value(RPM_KEY, Decimal::from(2), TTL)
        .await
        .unwrap();
    harness.counters.retire().await;
    assert_eq!(harness.store.amount(RPM_KEY), Some(Decimal::from(2)));
    harness
        .counters
        .increment_window_value(RPM_KEY, Decimal::from(1), TTL)
        .await
        .unwrap();
    tokio::time::advance(Duration::from_secs(1)).await;
    tokio::task::yield_now().await;
    assert_eq!(harness.store.amount(RPM_KEY), Some(Decimal::from(2)));
    harness.counters.cleanup().await;
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
    // the sink cannot await the store, so only an overflow arms a flush and the batch waits for the sync
    for _turn in 0..4 {
        tokio::task::yield_now().await;
    }
    assert_eq!(harness.store.calls(), 0);
    assert_eq!(harness.store.amount(TPM_KEY), None);
    assert!(harness.counters.sync_with_redis().await);
    assert_eq!(harness.store.amount(TPM_KEY), Some(Decimal::from(7)));
    let local_only = Harness::without_store(settings(false, None));
    local_only.counters.increment_pipeline(&increments).unwrap();
    assert_eq!(local_only.counters.read(TPM_KEY), Some(7));
    assert_eq!(local_only.pending_len(), 0);
    assert!(local_only.counters.get_tracked_keys().is_empty());
}

// LiteLLM copies redis's post-increment total back locally (router.py:8198-8202); the merge does it on the sync
async fn usage_key_merges_the_shared_store_total() {
    let harness = Harness::with_store(settings(false, None));
    harness.store.seed(TPM_KEY, Decimal::from(100));
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
    assert_eq!(harness.store.amount(TPM_KEY), Some(Decimal::from(107)));
    assert_eq!(harness.counters.read(TPM_KEY), Some(107));
}

async fn usage_overflow_flushes_within_the_deadline() {
    let harness = Harness::with_store(settings(false, None));
    let filled = vec![
        Increment {
            key: TPM_KEY.to_owned(),
            value: 1,
            ttl: TTL,
        };
        MAX_PENDING_DELTAS
    ];
    harness.counters.increment_pipeline(&filled).unwrap();
    assert_eq!(harness.pending_len(), MAX_PENDING_DELTAS);
    // the queue is full, so the delta waits in the overflow and arms one flush that runs in its own task
    harness
        .counters
        .increment_pipeline(&[Increment {
            key: TPM_KEY.to_owned(),
            value: 1,
            ttl: TTL,
        }])
        .unwrap();
    assert_eq!(harness.overflow_len(), 1);
    for _turn in 0..4 {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        harness.store.amount(TPM_KEY),
        Some(Decimal::from(1_025_u64))
    );
    assert_eq!(harness.pending_len(), 0);
    assert_eq!(harness.overflow_len(), 0);
    assert_eq!(harness.counters.read(TPM_KEY), Some(1_025));
    // the flag clears when the first flush ends, so a second overflow arms another flush
    let refill = vec![
        Increment {
            key: TPM_KEY.to_owned(),
            value: 1,
            ttl: TTL,
        };
        MAX_PENDING_DELTAS
    ];
    harness.counters.increment_pipeline(&refill).unwrap();
    harness
        .counters
        .increment_pipeline(&[Increment {
            key: TPM_KEY.to_owned(),
            value: 1,
            ttl: TTL,
        }])
        .unwrap();
    for _turn in 0..4 {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        harness.store.amount(TPM_KEY),
        Some(Decimal::from(2_050_u64))
    );
}

// the awaited callback path flushes a full queue inside its deadline, so healthy storage drops nothing
async fn usage_saturation_flushes_before_dropping() {
    let harness = Harness::with_store(settings(false, None));
    let one = [Increment {
        key: TPM_KEY.to_owned(),
        value: 1,
        ttl: TTL,
    }];
    for _step in 0..MAX_PENDING_DELTAS {
        harness.counters.increment_usage(&one).await.unwrap();
    }
    assert_eq!(harness.pending_len(), MAX_PENDING_DELTAS);
    harness.counters.increment_usage(&one).await.unwrap();
    assert_eq!(harness.store.calls(), 1);
    assert_eq!(
        harness.store.amount(TPM_KEY),
        Some(Decimal::from(1_024_u64))
    );
    assert_eq!(harness.pending_len(), 1);
    for _step in 0..(MAX_PENDING_DELTAS - 1) {
        harness.counters.increment_usage(&one).await.unwrap();
    }
    assert_eq!(harness.pending_len(), MAX_PENDING_DELTAS);
    harness.counters.increment_usage(&one).await.unwrap();
    assert_eq!(harness.store.calls(), 2);
    assert_eq!(
        harness.store.amount(TPM_KEY),
        Some(Decimal::from(2_048_u64))
    );
    assert_eq!(harness.counters.read(TPM_KEY), Some(2_049));
    assert_eq!(harness.pending_len(), 1);
    assert!(harness.counters.sync_with_redis().await);
    assert_eq!(
        harness.store.amount(TPM_KEY),
        Some(Decimal::from(2_049_u64))
    );
    assert_eq!(harness.counters.read(TPM_KEY), Some(2_049));
    assert_eq!(harness.pending_len(), 0);
    // a failed flush inside the deadline drops only the saturating delta; the local count keeps it
    let down = Harness::with_store(settings(false, None));
    down.store.set_down(true);
    for _step in 0..MAX_PENDING_DELTAS {
        down.counters.increment_usage(&one).await.unwrap();
    }
    down.counters.increment_usage(&one).await.unwrap();
    assert_eq!(down.pending_len(), 0);
    assert_eq!(down.store.amount(TPM_KEY), None);
    assert_eq!(down.counters.read(TPM_KEY), Some(1_025));
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
    let one = [Increment {
        key: TPM_KEY.to_owned(),
        value: 1,
        ttl: TTL,
    }];
    for _step in 0..MAX_PENDING_DELTAS {
        counters.increment_usage(&one).await.unwrap();
    }
    counters.increment_usage(&one).await.unwrap();
    assert_eq!(counters.read(TPM_KEY), Some(1_025));
    assert_eq!(pending_len(&counters), 0);
}

struct HangingStore;

#[async_trait]
impl DeltaStore for HangingStore {
    async fn increment_pipeline(
        &self,
        _deltas: &[Delta],
    ) -> Result<Vec<Decimal>, RoutingStateError> {
        std::future::pending::<Result<Vec<Decimal>, RoutingStateError>>().await
    }
}

// the failure callback's increment runs on another thread while a sync reconciles the same key
async fn failure_callback_increment_survives_the_merge() {
    let (counters, store, _local) = racing_counters();
    store.seed(Decimal::from(100));
    counters
        .increment_window_value(RPM_KEY, Decimal::from(7), TTL)
        .await
        .unwrap();
    *store.failure_during_push.lock().unwrap() = true;
    assert!(counters.sync_with_redis().await);
    // redis holds 107, and the merge adds its growth to the 8 the callback left behind
    assert_eq!(store.amount(), Decimal::from(107));
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
        .increment_window_value(RPM_KEY, Decimal::from(2), first_ttl)
        .await
        .unwrap();
    harness
        .counters
        .increment_window_value(RPM_KEY, Decimal::from(3), later_ttl)
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
    assert_eq!(pushed[0].value, Decimal::from(5));
    assert_eq!(pushed[0].ttl, first_ttl);
    assert_eq!(pushed[1].key.as_ref(), TPM_KEY);
    assert_eq!(pushed[1].value, Decimal::ONE);
    assert_eq!(pushed[1].ttl, other_ttl);
}

async fn concurrent_increment_during_the_push_is_not_double_counted() {
    let (counters, store, local) = racing_counters();
    counters
        .increment_window_value(RPM_KEY, Decimal::from(2), TTL)
        .await
        .unwrap();
    let _armed = store.during_push.lock().unwrap().replace(Decimal::ONE);
    assert!(counters.sync_with_redis().await);
    assert_eq!(store.amount(), Decimal::from(2));
    // the concurrent increment landed after the batch was detached, so it is queued and local keeps it once
    assert_eq!(local_amount(&local, &route_key(RPM_KEY)), Decimal::from(3));
    assert_eq!(pending_len(&counters), 1);
    assert!(counters.sync_with_redis().await);
    assert_eq!(store.amount(), Decimal::from(3));
    assert_eq!(local_amount(&local, &route_key(RPM_KEY)), Decimal::from(3));
    assert_eq!(pending_len(&counters), 0);
}

async fn concurrent_increment_under_a_leading_store_is_merged_once() {
    let (counters, store, local) = racing_counters();
    store.seed(Decimal::from(100));
    counters
        .increment_window_value(RPM_KEY, Decimal::from(7), TTL)
        .await
        .unwrap();
    let _armed = store.during_push.lock().unwrap().replace(Decimal::ONE);
    assert!(counters.sync_with_redis().await);
    // redis holds 107, the merge adds the one increment still queued, and the next push lands it once
    assert_eq!(store.amount(), Decimal::from(107));
    assert_eq!(
        local_amount(&local, &route_key(RPM_KEY)),
        Decimal::from(108)
    );
    assert_eq!(pending_len(&counters), 1);
    assert!(counters.sync_with_redis().await);
    assert_eq!(store.amount(), Decimal::from(108));
    assert_eq!(
        local_amount(&local, &route_key(RPM_KEY)),
        Decimal::from(108)
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
    assert!(crate::routing_state::KEY_PATTERN_TO_SYNC.is_none());
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
        .increment_window_value(RPM_KEY, Decimal::from(2), TTL)
        .await
        .unwrap();
    assert!(counters.sync_with_redis().await);
    assert!(same_f64(stored(&store, &key).await, 2.0_f64));
    server.set_down(true);
    counters
        .increment_window_value(RPM_KEY, Decimal::from(4), TTL)
        .await
        .unwrap();
    assert!(!counters.sync_with_redis().await);
    server.set_down(false);
    assert!(!counters.sync_with_redis().await);
    assert!(same_f64(stored(&store, &key).await, 2.0_f64));
    counters
        .increment_window_value(RPM_KEY, Decimal::ONE, TTL)
        .await
        .unwrap();
    assert!(counters.sync_with_redis().await);
    assert!(same_f64(stored(&store, &key).await, 3.0_f64));
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
    state: Mutex<Option<Weak<SyncState<RacingStore>>>>,
    counters: Mutex<Option<Weak<RouterCounters<RacingStore>>>>,
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
            let state = self.state.lock().unwrap().as_ref().and_then(Weak::upgrade);
            if let Some(state) = state {
                let key = route_key(RPM_KEY);
                let _recorded = state.record(&key, amount, TTL).unwrap();
            }
        }
        if mem::take(&mut *self.failure_during_push.lock().unwrap()) {
            let counters = self
                .counters
                .lock()
                .unwrap()
                .as_ref()
                .and_then(Weak::upgrade);
            if let Some(counters) = counters {
                std::thread::scope(|scope| {
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

fn pending_len<S: DeltaStore + 'static>(counters: &RouterCounters<S>) -> usize {
    counters.state.batches.lock().unwrap().pending.len()
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
