use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use rust_decimal::Decimal;

use crate::router_observations::counter::{CounterSink, Increment};
use crate::routing_state::counter::RouterCounters;
use crate::routing_state::metadata::{
    CallbackChannels, deployment_channel, find_deployment_metadata, get_counted_usage_tokens,
};
use crate::routing_state_store::RoutingStateStore;
use crate::routing_state_store::test_support::FakeRedis;
use crate::routing_value_store::RouteKey;

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
        "tracked_key_accessors" => tracked_key_accessors(),
        "success_and_failure_counters" => success_and_failure_counters(),
        "sync_merges_the_stored_value" => sync_merges_the_stored_value().await,
        "sync_keeps_a_value_redis_is_behind_on" => sync_keeps_a_value_redis_is_behind_on().await,
        "failed_push_discards_its_batch" => failed_push_discards_its_batch().await,
        "periodic_sync_writes_on_the_interval" => periodic_sync_writes_on_the_interval().await,
        "no_periodic_task_without_batching" => no_periodic_task_without_batching().await,
        "cleanup_stops_the_periodic_task" => cleanup_stops_the_periodic_task().await,
        "retire_pushes_and_stops_the_task" => retire_pushes_and_stops_the_task().await,
        "usage_pipeline_increments_locally_and_queues" => {
            usage_pipeline_increments_locally_and_queues().await;
        }
        "counter_write_and_read_round_trip" => counter_write_and_read_round_trip(),
        "metadata_bucket_selection" => metadata_bucket_selection(),
        "key_pattern_is_absent" => key_pattern_is_absent(),
        "real_store_smoke" => real_store_smoke().await,
        _ => panic!("unknown case {name}"),
    }
}

async fn window_increment_returns_and_queues() {
    let harness = Harness::with_store(settings(false, None));
    let first = harness
        .counters
        .increment_window_value(RPM_KEY, 2, TTL)
        .await;
    assert_eq!(first.unwrap(), Decimal::from(2));
    let second = harness
        .counters
        .increment_window_value(RPM_KEY, 3, TTL)
        .await;
    assert_eq!(second.unwrap(), Decimal::from(5));
    assert_eq!(harness.local_amount(RPM_KEY), Some(Decimal::from(5)));
    assert_eq!(harness.pending_len(), 2);
    let increments = [(TPM_KEY.to_owned(), 4_i64)];
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
        .increment_window_value(RPM_KEY, 3, TTL)
        .await;
    assert_eq!(value.unwrap(), Decimal::from(3));
    assert_eq!(harness.pending_len(), 0);
    assert_eq!(harness.store.calls(), 0);
    assert_eq!(harness.counters.get_tracked_keys(), set(&[RPM_KEY]));
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
    harness.counters.reset_tracked_keys();
    assert!(harness.counters.get_tracked_keys().is_empty());
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
        .increment_window_value(RPM_KEY, 2, TTL)
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
        .increment_window_value(RPM_KEY, 2, TTL)
        .await
        .unwrap();
    assert!(harness.counters.sync_with_redis().await);
    assert_eq!(harness.store.amount(RPM_KEY), Some(Decimal::from(2)));
    assert_eq!(harness.local_amount(RPM_KEY), Some(Decimal::from(2)));
    assert_eq!(harness.counters.get_tracked_keys(), set(&[RPM_KEY]));
}

async fn failed_push_discards_its_batch() {
    let harness = Harness::with_store(settings(false, None));
    harness.store.set_down(true);
    harness
        .counters
        .increment_window_value(RPM_KEY, 2, TTL)
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
        .increment_window_value(RPM_KEY, 1, TTL)
        .await
        .unwrap();
    assert!(harness.counters.sync_with_redis().await);
    assert_eq!(harness.store.amount(RPM_KEY), Some(Decimal::from(1)));
    assert_eq!(harness.local_amount(RPM_KEY), Some(Decimal::from(3)));
}

async fn periodic_sync_writes_on_the_interval() {
    let harness = Harness::with_store(settings(true, Some(Duration::from_millis(250))));
    tokio::time::pause();
    harness.counters.start();
    harness
        .counters
        .increment_window_value(RPM_KEY, 2, TTL)
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
    tokio::time::pause();
    harness.counters.start();
    harness
        .counters
        .increment_window_value(RPM_KEY, 1, TTL)
        .await
        .unwrap();
    tokio::time::advance(Duration::from_secs(10)).await;
    tokio::task::yield_now().await;
    assert_eq!(harness.store.calls(), 0);
    harness.counters.cleanup().await;
}

async fn cleanup_stops_the_periodic_task() {
    let harness = Harness::with_store(settings(true, Some(Duration::from_millis(50))));
    tokio::time::pause();
    harness.counters.start();
    harness.counters.cleanup().await;
    harness
        .counters
        .increment_window_value(RPM_KEY, 1, TTL)
        .await
        .unwrap();
    tokio::time::advance(Duration::from_secs(10)).await;
    tokio::task::yield_now().await;
    assert_eq!(harness.store.calls(), 0);
}

async fn retire_pushes_and_stops_the_task() {
    let harness = Harness::with_store(settings(true, Some(Duration::from_millis(50))));
    tokio::time::pause();
    harness.counters.start();
    harness
        .counters
        .increment_window_value(RPM_KEY, 2, TTL)
        .await
        .unwrap();
    harness.counters.retire().await;
    assert_eq!(harness.store.amount(RPM_KEY), Some(Decimal::from(2)));
    harness
        .counters
        .increment_window_value(RPM_KEY, 1, TTL)
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
    // the sink cannot await the store, so the pipeline reaches it on the owner's next sync
    assert_eq!(harness.store.calls(), 0);
    assert_eq!(harness.store.amount(TPM_KEY), None);
    assert!(harness.counters.sync_with_redis().await);
    assert_eq!(harness.store.amount(TPM_KEY), Some(Decimal::from(7)));
    let local_only = Harness::without_store(settings(false, None));
    local_only.counters.increment_pipeline(&increments).unwrap();
    assert_eq!(local_only.counters.read(TPM_KEY), Some(7));
    assert_eq!(local_only.pending_len(), 0);
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
    assert!(find_deployment_metadata(&both).is_some());
    let fallback = channels(r#"{"metadata": {"model_info": {"id": "b"}}}"#);
    assert_eq!(channel_name(&fallback), Some("metadata"));
    assert_eq!(get_counted_usage_tokens(&fallback), None);
    assert!(find_deployment_metadata(&fallback).is_some());
    let absent = channels(r#"{"litellm_metadata": "text", "metadata": {"deployment": "x"}}"#);
    assert!(find_deployment_metadata(&absent).is_none());
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
        .increment_window_value(RPM_KEY, 2, TTL)
        .await
        .unwrap();
    assert!(counters.sync_with_redis().await);
    assert!(same_f64(stored(&store, &key).await, 2.0_f64));
    server.set_down(true);
    counters
        .increment_window_value(RPM_KEY, 4, TTL)
        .await
        .unwrap();
    assert!(!counters.sync_with_redis().await);
    server.set_down(false);
    assert!(!counters.sync_with_redis().await);
    assert!(same_f64(stored(&store, &key).await, 2.0_f64));
    counters
        .increment_window_value(RPM_KEY, 1, TTL)
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

fn channels(text: &str) -> CallbackChannels {
    serde_json::from_str(text).unwrap()
}

fn channel_name(channels: &CallbackChannels) -> Option<&'static str> {
    deployment_channel(channels).map(|(name, _bucket)| name)
}

fn set(keys: &[&str]) -> BTreeSet<String> {
    keys.iter().map(|key| (*key).to_owned()).collect()
}
