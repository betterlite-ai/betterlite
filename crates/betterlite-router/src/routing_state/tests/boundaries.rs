use std::io;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use async_trait::async_trait;
use rust_decimal::Decimal;
use tokio::task::yield_now;
use tokio::time::advance;

use crate::router_observations::counter::{CounterError, CounterSink, Increment};
use crate::routing_state::counter::RouterCounters;
use crate::routing_state::delta::{Delta, DeltaStore};
use crate::routing_state::error::RoutingStateError;
use crate::routing_state::metadata::{
    CallbackChannels, deployment_channel, get_counted_usage_tokens,
};
use crate::routing_state::{FLUSH_DEADLINE, MAX_PENDING_DELTAS, MAX_TRACKED_KEYS, Settings};
use crate::routing_value_store::{
    MAX_KEY_BYTES, PinValue, RouteValue, RoutingValueStore, StoreRead,
};

use super::support::{
    Harness, TestClock, key_limit_error, local_store, oversized_key, route_key, settings,
    usage_batch,
};

const TTL: Duration = Duration::from_secs(60);
const RPM_KEY: &str = "global_router:dep-a:gpt-4o:rpm:00-00";
const TPM_KEY: &str = "global_router:dep-a:gpt-4o:tpm:00-00";

pub(crate) async fn run(name: &str) {
    match name {
        "scheduled_gate_timeout" => super::flush::gate_timeout().await,
        "queue_cap_is_1024" => queue_cap_is_1024().await,
        "queue_cap_is_2048" => queue_cap_is_2048().await,
        "sync_past_the_deadline_drops_queued_callbacks" => {
            sync_past_the_deadline_drops_queued_callbacks(false).await;
        }
        "periodic_sync_past_the_deadline_drops_queued_callbacks" => {
            sync_past_the_deadline_drops_queued_callbacks(true).await;
        }
        "callback_saturation_without_a_runtime_keeps_the_delta" => {
            callback_saturation_without_a_runtime_keeps_the_delta().await;
        }
        "tracked_cap_is_1024" => tracked_cap_is_1024(),
        "window_increment_rejects_an_oversized_key" => {
            window_increment_rejects_an_oversized_key().await;
        }
        "counter_sink_rejects_an_oversized_key" => counter_sink_rejects_an_oversized_key(),
        "outcome_counter_expiry_boundaries" => outcome_counter_expiry_boundaries(),
        "cancelled_sync_task_does_not_tick" => cancelled_sync_task_does_not_tick().await,
        "sync_without_a_store_returns_false" => sync_without_a_store_returns_false().await,
        "metadata_first_bucket_decides" => metadata_first_bucket_decides(),
        "sync_merges_at_the_equality_boundary" => sync_merges_at_the_equality_boundary().await,
        "sync_interval_resolution_boundaries" => sync_interval_resolution_boundaries(),
        "local_counter_reads_only_whole_values" => local_counter_reads_only_whole_values(),
        _ => panic!("unknown case {name}"),
    }
}

async fn queue_cap_is_1024() {
    let harness = Harness::with_store(settings(false, None));
    for _step in 0..MAX_PENDING_DELTAS {
        harness
            .counters
            .increment_window_value(RPM_KEY, Decimal::ONE, TTL)
            .await
            .unwrap();
    }
    assert_eq!(harness.pending_len(), 1024);
    // the caller that fills the queue flushes it, and its own delta rides that one push
    let overflow = harness
        .counters
        .increment_window_value(RPM_KEY, Decimal::ONE, TTL)
        .await;
    assert_eq!(overflow.unwrap(), Decimal::from(1025_i64));
    assert_eq!(harness.store.amount(RPM_KEY), Some(Decimal::from(1025_i64)));
    assert_eq!(harness.store.pushed().len(), 1);
    assert_eq!(harness.pending_len(), 0);
    // the queue is empty now, so a sync has nothing to push
    assert!(!harness.counters.sync_with_redis().await);

    // the same overflow with the store down loses the batch and the observation, and a later sync starts over
    let down = Harness::with_store(settings(false, None));
    down.store.set_down(true);
    for _step in 0..MAX_PENDING_DELTAS {
        down.counters
            .increment_window_value(RPM_KEY, Decimal::ONE, TTL)
            .await
            .unwrap();
    }
    let down_overflow = down
        .counters
        .increment_window_value(RPM_KEY, Decimal::ONE, TTL)
        .await;
    assert_eq!(down_overflow.unwrap(), Decimal::from(1025_i64));
    assert_eq!(down.pending_len(), 0);
    assert_eq!(down.store.calls(), 1);
    assert_eq!(down.store.amount(RPM_KEY), None);
    down.store.set_down(false);
    down.counters
        .increment_window_value(RPM_KEY, Decimal::ONE, TTL)
        .await
        .unwrap();
    assert!(down.counters.sync_with_redis().await);
    assert_eq!(down.store.amount(RPM_KEY), Some(Decimal::ONE));
    assert_eq!(down.pending_len(), 0);
}

// the single queue stops at twice the threshold before polling and while the store stalls
async fn queue_cap_is_2048() {
    let harness = Harness::with_store(settings(false, None));
    harness.store.set_stalled(true);
    let sink: &dyn CounterSink = harness.counters.as_ref();
    sink.increment_pipeline(&usage_batch(TPM_KEY, MAX_PENDING_DELTAS))
        .unwrap();
    let one = usage_batch(TPM_KEY, 1);
    // repeated callbacks share one scheduled task before it is polled
    for _step in 0..MAX_PENDING_DELTAS {
        sink.increment_pipeline(&one).unwrap();
    }
    assert_eq!(harness.store.calls(), 0);
    assert_eq!(harness.pending_len(), 2 * MAX_PENDING_DELTAS);
    assert_eq!(harness.counters.flushes.lock().unwrap().len(), 1);
    assert_overflow_warning(&harness);
    // the flush detaches the queue and stalls at the store
    yield_now().await;
    assert_eq!(harness.store.calls(), 1);
    assert_eq!(harness.pending_len(), 0);
    // the queue fills again behind the stalled flush up to its hard cap
    sink.increment_pipeline(&usage_batch(TPM_KEY, MAX_PENDING_DELTAS))
        .unwrap();
    for _step in 0..MAX_PENDING_DELTAS {
        sink.increment_pipeline(&one).unwrap();
    }
    assert_eq!(harness.pending_len(), 2 * MAX_PENDING_DELTAS);
    // past the cap an observation is dropped, so the owner's staging stays bounded
    assert_overflow_warning(&harness);
    assert_eq!(harness.pending_len(), 2 * MAX_PENDING_DELTAS);
    // the accepted batches reach storage once; the two overflow observations do not
    harness.store.set_stalled(false);
    harness.store.release();
    harness.drain_flushes().await;
    assert_eq!(harness.store.calls(), 2);
    assert_eq!(
        harness.store.amount(TPM_KEY),
        Some(Decimal::from(4_096_u64))
    );
    assert_eq!(harness.pending_len(), 0);
    assert_eq!(harness.counters.read(TPM_KEY), Some(4_098));
}

#[derive(Clone)]
struct WarningLog(Arc<Mutex<Vec<u8>>>);

impl io::Write for WarningLog {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn assert_overflow_warning(harness: &Harness) {
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let writer = WarningLog(Arc::clone(&bytes));
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_target(false)
        .with_writer(move || writer.clone())
        .finish();
    tracing::subscriber::with_default(subscriber, || {
        harness
            .counters
            .increment_pipeline(&usage_batch(TPM_KEY, 1))
            .unwrap();
    });
    let message = String::from_utf8(bytes.lock().unwrap().clone()).unwrap();
    assert_eq!(
        message.trim(),
        format!(
            "WARN the routing delta queue is full; the observation is dropped key=\"{TPM_KEY}\""
        )
    );
    assert_eq!(harness.pending_len(), 2 * MAX_PENDING_DELTAS);
}

// a stalled sync and the successor scheduled behind it both expire without replay
async fn sync_past_the_deadline_drops_queued_callbacks(periodic: bool) {
    let harness = Harness::with_store(settings(periodic, None));
    harness.store.set_stalled(true);
    let sink: &dyn CounterSink = harness.counters.as_ref();
    sink.increment_pipeline(&usage_batch(TPM_KEY, MAX_PENDING_DELTAS))
        .unwrap();
    let sync = if periodic {
        harness.counters.start();
        None
    } else {
        let owner = Arc::clone(&harness.counters);
        Some(tokio::spawn(async move { owner.sync_with_redis().await }))
    };
    yield_now().await;
    assert_eq!(harness.store.calls(), 1);
    // the saturated callback schedules a successor while the sync holds the gate
    sink.increment_pipeline(&usage_batch(TPM_KEY, MAX_PENDING_DELTAS + 1))
        .unwrap();
    assert_eq!(harness.pending_len(), MAX_PENDING_DELTAS + 1);
    yield_now().await;
    advance(FLUSH_DEADLINE).await;
    if let Some(task) = sync {
        assert!(!task.await.unwrap());
    }
    harness.counters.cleanup().await;
    assert_eq!(harness.pending_len(), 0);
    harness.store.set_stalled(false);
    assert!(!harness.counters.sync_with_redis().await);
    assert_eq!(harness.store.amount(TPM_KEY), None);
    assert_eq!(harness.counters.read(TPM_KEY), Some(2_049));
}

// a sink call on a thread outside every runtime cannot hand the flush off, so the delta waits for the next sync
async fn callback_saturation_without_a_runtime_keeps_the_delta() {
    let harness = Harness::with_store(settings(false, None));
    harness
        .counters
        .increment_pipeline(&usage_batch(TPM_KEY, MAX_PENDING_DELTAS))
        .unwrap();
    assert_eq!(harness.pending_len(), MAX_PENDING_DELTAS);
    let counters = Arc::clone(&harness.counters);
    thread::scope(|scope| {
        let _handle = scope.spawn(move || {
            counters
                .increment_pipeline(&usage_batch(TPM_KEY, 1))
                .unwrap();
        });
    });
    assert_eq!(harness.pending_len(), MAX_PENDING_DELTAS + 1);
    assert_eq!(harness.store.calls(), 0);
    assert_eq!(harness.counters.read(TPM_KEY), Some(1_025));
    // the owner's next sync carries all queued observations
    assert!(harness.counters.sync_with_redis().await);
    assert_eq!(
        harness.store.amount(TPM_KEY),
        Some(Decimal::from(1_025_u64))
    );
    assert_eq!(harness.pending_len(), 0);
}

fn tracked_cap_is_1024() {
    let harness = Harness::without_store(settings(false, None));
    for index in 0..MAX_TRACKED_KEYS {
        harness.counters.track_key(&format!("key-{index}"));
    }
    assert_eq!(harness.tracked_len(), 1024);
    harness.counters.track_key("key-overflow");
    assert_eq!(harness.tracked_len(), 1024);
    assert!(!harness.counters.get_tracked_keys().contains("key-overflow"));
    let taken = harness.counters.take_tracked_keys();
    assert_eq!(taken.len(), 1024);
    assert!(harness.counters.get_tracked_keys().is_empty());
}

async fn sync_merges_at_the_equality_boundary() {
    let clock = TestClock::new();
    let local = Arc::new(local_store(&clock));
    let store = Arc::new(DriftingStore {
        value: Mutex::new(Decimal::from(5)),
        local: Arc::clone(&local),
        during_push: Mutex::new(Some(Decimal::from(7))),
    });
    let counters = RouterCounters::new(settings(false, None), Arc::clone(&local), Some(store));
    counters
        .increment_window_value(RPM_KEY, Decimal::from(2), TTL)
        .await
        .unwrap();
    // before 2, push stores 7 and local reaches 7 during it, so after equals redis and the merge writes 12
    assert!(counters.sync_with_redis().await);
    let stored = local.get(&route_key(RPM_KEY)).unwrap();
    match stored {
        StoreRead::Value(RouteValue::Spend(value)) => assert_eq!(value, Decimal::from(12)),
        other => panic!("expected the merged spend, got {other:?}"),
    }
}

// a store whose reply and a write the same instant makes to the local value land between the snapshot and the merge
struct DriftingStore {
    value: Mutex<Decimal>,
    local: Arc<RoutingValueStore>,
    during_push: Mutex<Option<Decimal>>,
}

#[async_trait]
impl DeltaStore for DriftingStore {
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
        let drift = self.during_push.lock().unwrap().take();
        if let Some(amount) = drift {
            self.local
                .set(&route_key(RPM_KEY), &RouteValue::Spend(amount), None)
                .unwrap();
        }
        Ok(results)
    }
}

async fn window_increment_rejects_an_oversized_key() {
    let harness = Harness::with_store(settings(false, None));
    let key = oversized_key();
    let outcome = harness
        .counters
        .increment_window_value(&key, Decimal::ONE, TTL)
        .await;
    let error = outcome.unwrap_err();
    assert_eq!(key_limit_error(&error), (key.len(), MAX_KEY_BYTES));
    assert!(error.to_string().contains("1025"));
    harness.counters.track_key(&key);
    assert_eq!(harness.pending_len(), 0);
    assert!(harness.counters.get_tracked_keys().is_empty());
}

fn counter_sink_rejects_an_oversized_key() {
    let harness = Harness::with_store(settings(false, None));
    let key = oversized_key();
    let error = harness.counters.increment(&key, 1, TTL).unwrap_err();
    match error {
        CounterError::Rejected { key: rejected } => assert_eq!(rejected, key),
    }
    let increments = [
        Increment {
            key: key.clone(),
            value: 1,
            ttl: TTL,
        },
        Increment {
            key: "counter".to_owned(),
            value: 1,
            ttl: TTL,
        },
    ];
    harness.counters.increment_pipeline(&increments).unwrap();
    assert_eq!(harness.counters.read("counter"), Some(1));
    assert_eq!(harness.counters.read(&key), None);
    assert_eq!(harness.pending_len(), 1);
}

fn outcome_counter_expiry_boundaries() {
    let harness = Harness::without_store(settings(false, None));
    harness.clock.set_millis(0);
    harness
        .counters
        .increment_successes_for_minute("dep-a")
        .unwrap();
    harness.clock.set_millis(59_999);
    assert_eq!(harness.counters.successes_for_minute("dep-a"), 1);
    harness.clock.set_millis(60_000);
    assert_eq!(harness.counters.successes_for_minute("dep-a"), 1);
    harness.clock.set_millis(60_001);
    assert_eq!(harness.counters.successes_for_minute("dep-a"), 0);
    harness
        .counters
        .increment_failures_for_minute("dep-a")
        .unwrap();
    assert_eq!(harness.counters.failures_for_minute("dep-a"), 1);
    assert_eq!(harness.counters.successes_for_minute("dep-a"), 0);
}

async fn cancelled_sync_task_does_not_tick() {
    let harness = Harness::with_store(settings(true, Some(Duration::from_millis(50))));
    harness.counters.start();
    yield_now().await;
    harness.counters.cancel_sync_task();
    harness
        .counters
        .increment_window_value(RPM_KEY, Decimal::ONE, TTL)
        .await
        .unwrap();
    advance(Duration::from_secs(10)).await;
    yield_now().await;
    assert_eq!(harness.store.calls(), 0);
    harness.counters.cleanup().await;
}

async fn sync_without_a_store_returns_false() {
    let harness = Harness::without_store(settings(true, None));
    assert!(!harness.counters.sync_with_redis().await);
    harness
        .counters
        .increment_window_value(RPM_KEY, Decimal::ONE, TTL)
        .await
        .unwrap();
    assert!(!harness.counters.sync_with_redis().await);
    assert_eq!(harness.pending_len(), 0);
    assert_eq!(harness.store.calls(), 0);
}

fn metadata_first_bucket_decides() {
    let first = channels(
        r#"{"litellm_metadata": {"_litellm_router_usage_counted_tokens": "8"},
            "metadata": {"_litellm_router_usage_counted_tokens": 7}}"#,
    );
    assert_eq!(get_counted_usage_tokens(&first), None);
    let second = channels(
        r#"{"litellm_metadata": {}, "metadata": {"_litellm_router_usage_counted_tokens": 7}}"#,
    );
    assert_eq!(get_counted_usage_tokens(&second), Some(7));
    let float = channels(r#"{"metadata": {"_litellm_router_usage_counted_tokens": 7.0}}"#);
    assert_eq!(get_counted_usage_tokens(&float), None);
    let flag = channels(r#"{"metadata": {"_litellm_router_usage_counted_tokens": true}}"#);
    assert_eq!(get_counted_usage_tokens(&flag), None);
    let largest =
        channels(r#"{"metadata": {"_litellm_router_usage_counted_tokens": 9223372036854775807}}"#);
    assert_eq!(get_counted_usage_tokens(&largest), Some(i64::MAX));
    // below the i64 floor the value is kept, above the ceiling it is not an i64 and counts as absent
    let below =
        channels(r#"{"metadata": {"_litellm_router_usage_counted_tokens": -9223372036854775808}}"#);
    assert_eq!(get_counted_usage_tokens(&below), Some(i64::MIN));
    let above =
        channels(r#"{"metadata": {"_litellm_router_usage_counted_tokens": 9223372036854775808}}"#);
    assert_eq!(get_counted_usage_tokens(&above), None);
    let null_model = channels(
        r#"{"litellm_metadata": {"model_info": null}, "metadata": {"model_info": {"id": "b"}}}"#,
    );
    let name = deployment_channel(&null_model).map(|(name, _bucket)| name);
    assert_eq!(name, Some("litellm_metadata"));
    let empty_model = channels(
        r#"{"litellm_metadata": {"model_info": {}}, "metadata": {"model_info": {"id": "b"}}}"#,
    );
    let empty_name = deployment_channel(&empty_model).map(|(name, _bucket)| name);
    assert_eq!(empty_name, Some("litellm_metadata"));
}

fn sync_interval_resolution_boundaries() {
    assert_eq!(
        resolved(None, Duration::from_secs(7)),
        Duration::from_secs(7)
    );
    assert_eq!(
        resolved(Some(Duration::from_millis(1)), Duration::from_secs(7)),
        Duration::from_millis(1)
    );
    assert_eq!(
        resolved(Some(Duration::ZERO), Duration::from_secs(7)),
        Duration::from_secs(7)
    );
    assert_eq!(resolved(None, Duration::ZERO), Duration::ZERO);
    assert_eq!(
        resolved(Some(Duration::ZERO), Duration::ZERO),
        Duration::ZERO
    );
}

fn resolved(interval: Option<Duration>, default: Duration) -> Duration {
    Settings {
        sync_interval: interval,
        default_sync_interval: default,
        should_batch_redis_writes: false,
    }
    .resolved_sync_interval()
}

fn local_counter_reads_only_whole_values() {
    let harness = Harness::without_store(settings(false, None));
    let whole = route_key("whole");
    harness
        .local
        .set(&whole, &RouteValue::Spend(Decimal::from(3)), None)
        .unwrap();
    assert_eq!(harness.counters.read("whole"), Some(3));
    let counted = route_key("counted");
    harness
        .local
        .set(&counted, &RouteValue::Counter(4), None)
        .unwrap();
    assert_eq!(harness.counters.read("counted"), Some(4));
    let fractional = route_key("fractional");
    harness
        .local
        .set(&fractional, &RouteValue::Spend(Decimal::new(35, 1)), None)
        .unwrap();
    assert_eq!(harness.counters.read("fractional"), None);
    let pinned = route_key("pinned");
    let pin = RouteValue::Pin(PinValue {
        model_id: "m".to_owned(),
    });
    harness.local.set(&pinned, &pin, None).unwrap();
    assert_eq!(harness.counters.read("pinned"), None);
    assert_eq!(harness.counters.read("missing"), None);
}

fn channels(text: &str) -> CallbackChannels {
    serde_json::from_str(text).unwrap()
}
