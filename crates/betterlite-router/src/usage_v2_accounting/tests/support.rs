use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::router_observations::counter::CounterSink as _;
use crate::router_observations::usage::DeploymentUsage;
use crate::routing_state::Settings;
use crate::routing_state::counter::RouterCounters;
use crate::routing_state::delta::{Delta, DeltaStore};
use crate::routing_state::error::RoutingStateError;
use crate::routing_state_store::error::StateStoreError;
use crate::routing_value_store::{Clock, RoutingValueStore, StoreSettings};
use crate::usage_v2_accounting::UsageV2Accounting;
use crate::usage_v2_accounting::deployment::DeploymentFacts;
use crate::usage_v2_accounting::success::{HiddenParams, StandardLoggingObject, SuccessEvent};

pub(super) const TTL: Duration = Duration::from_secs(60);
pub(super) const MINUTE_MILLIS: u64 = 60_000;
pub(super) const MINUTE_MILLIS_END: u64 = 59_999;
pub(super) const RPM_KEY: &str = "dep-a:gpt-4o:rpm:00-00";
pub(super) const NEXT_MINUTE_RPM_KEY: &str = "dep-a:gpt-4o:rpm:00-01";
pub(super) const TPM_KEY: &str = "dep-a:openai/gpt-4o-mini:tpm:00-00";

pub(super) fn at(millis: u64) -> DateTime<Utc> {
    let millis = i64::try_from(millis).expect("the test time fits an i64");
    DateTime::from_timestamp_millis(millis).expect("the test time is representable")
}

pub(super) fn rpm(limit: Option<i64>) -> DeploymentUsage {
    DeploymentUsage {
        rpm: limit,
        ..DeploymentUsage::default()
    }
}

pub(super) fn deployment(
    model_id: &'static str,
    limits: DeploymentUsage,
) -> DeploymentFacts<'static> {
    DeploymentFacts {
        model_id,
        model: "gpt-4o",
        limits,
        num_retries: Some(2),
    }
}

pub(super) fn payload(
    model_group: Option<&'static str>,
    model_id: Option<&'static str>,
    model: Option<&'static str>,
    total_tokens: i64,
) -> StandardLoggingObject<'static> {
    StandardLoggingObject {
        model_group,
        hidden_params: HiddenParams {
            litellm_model_name: model,
        },
        model_id,
        total_tokens,
    }
}

pub(super) fn success_event(logged: StandardLoggingObject<'static>) -> SuccessEvent<'static> {
    SuccessEvent {
        call_type: Some("acompletion"),
        standard_logging_object: Some(logged),
    }
}

#[derive(Default)]
pub(super) struct MemoryStore {
    values: Mutex<BTreeMap<String, Decimal>>,
    down: AtomicBool,
}

impl MemoryStore {
    pub(super) fn set_down(&self, down: bool) {
        self.down.store(down, Ordering::SeqCst);
    }

    pub(super) fn seed(&self, key: &str, value: Decimal) {
        self.values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), value);
    }

    pub(super) fn value(&self, key: &str) -> Decimal {
        self.values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .copied()
            .unwrap_or_default()
    }
}

#[async_trait]
impl DeltaStore for MemoryStore {
    async fn increment_pipeline(
        &self,
        deltas: &[Delta],
    ) -> Result<Vec<Decimal>, RoutingStateError> {
        if self.down.load(Ordering::SeqCst) {
            return Err(RoutingStateError::Store {
                source: StateStoreError::NotConfigured,
            });
        }
        let mut values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        let mut results = Vec::with_capacity(deltas.len());
        for delta in deltas {
            let total = values.entry(delta.key.as_ref().to_owned()).or_default();
            *total = total.saturating_add(delta.value);
            results.push(*total);
        }
        Ok(results)
    }
}

pub(super) struct Harness {
    pub(super) accounting: UsageV2Accounting<MemoryStore>,
    pub(super) store: Arc<MemoryStore>,
    clock: Arc<AtomicU64>,
}

impl Harness {
    pub(super) fn new() -> Self {
        Self::with_ttl(TTL)
    }

    pub(super) fn with_ttl(ttl: Duration) -> Self {
        let clock = Arc::new(AtomicU64::new(0));
        let store = Arc::new(MemoryStore::default());
        let local = RoutingValueStore::new(store_settings(), value_clock(&clock))
            .expect("the local store builds");
        let counters = RouterCounters::new(
            counter_settings(),
            Arc::new(local),
            Some(Arc::clone(&store)),
        );
        Self {
            accounting: UsageV2Accounting {
                counters: Arc::new(counters),
                ttl,
            },
            store,
            clock,
        }
    }

    pub(super) fn set_millis(&self, millis: u64) {
        self.clock.store(millis, Ordering::SeqCst);
    }

    pub(super) fn local(&self, key: &str) -> Option<i64> {
        self.accounting.counters.read(key)
    }

    pub(super) fn seed_local(&self, key: &str, value: i64) {
        self.accounting.counters.write(key, value, Some(TTL));
    }
}

fn value_clock(clock: &Arc<AtomicU64>) -> Clock {
    let source = Arc::clone(clock);
    let read = move || Duration::from_millis(source.load(Ordering::SeqCst)).as_secs_f64();
    Arc::new(read)
}

const fn counter_settings() -> Settings {
    Settings {
        sync_interval: None,
        default_sync_interval: Duration::from_secs(1),
        should_batch_redis_writes: true,
    }
}

const fn store_settings() -> StoreSettings {
    StoreSettings {
        max_size_in_memory: Some(200),
        default_ttl_seconds: None,
        max_size_per_item: None,
        loaded_max_size_per_item_kib: 1024,
    }
}
