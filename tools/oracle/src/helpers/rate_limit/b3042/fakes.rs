use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_auth::rejection_provider::ProviderResolution;
use betterlite_models::resolution::{ResolutionError, ResolvedModel};
use betterlite_rate_limit::limiter_records::{CacheCounterValue, LimiterClock, ModelGroupResolver};
use betterlite_rate_limit::rate_limit_window_store::{CounterCache, CounterValues, RedisCacheKind};
use betterlite_rate_limit::token_adjustments::store::UsageIncrements;
use betterlite_redis::cache_primitives::pipeline_increment::IncrementOperation;

use super::CounterValueInput;

#[derive(Clone, Debug)]
pub(super) enum RecordedValue {
    Counter(CacheCounterValue),
    Local(f64),
}

#[derive(Clone, Debug)]
pub(super) struct WriteRecord {
    pub(super) key: String,
    pub(super) value: RecordedValue,
    pub(super) ttl: Option<i64>,
    pub(super) local_only: bool,
}

#[derive(Clone, Debug)]
pub(super) struct ReadRecord {
    pub(super) key: String,
    pub(super) local_only: bool,
}

#[derive(Default)]
pub(super) struct Effects {
    reads: Mutex<Vec<ReadRecord>>,
    writes: Mutex<Vec<WriteRecord>>,
    increments: Mutex<Vec<IncrementOperation>>,
    pipelines: Mutex<Vec<Vec<IncrementOperation>>>,
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Effects {
    pub(super) fn reads(&self) -> Vec<ReadRecord> {
        locked(&self.reads).clone()
    }

    pub(super) fn writes(&self) -> Vec<WriteRecord> {
        locked(&self.writes).clone()
    }

    pub(super) fn increments(&self) -> Vec<IncrementOperation> {
        locked(&self.increments).clone()
    }

    pub(super) fn pipelines(&self) -> Vec<Vec<IncrementOperation>> {
        locked(&self.pipelines).clone()
    }
}

pub(super) struct RecordingCache {
    values: Mutex<BTreeMap<String, CacheCounterValue>>,
    effects: Arc<Effects>,
}

impl RecordingCache {
    pub(super) fn new(
        entries: &BTreeMap<String, CounterValueInput>,
        effects: Arc<Effects>,
    ) -> Self {
        let values = entries
            .iter()
            .map(|(key, value)| (key.clone(), value.counter_value()))
            .collect();
        Self {
            values: Mutex::new(values),
            effects,
        }
    }
}

#[async_trait::async_trait]
impl CounterCache for RecordingCache {
    async fn get_cache(&self, key: &str, local_only: bool) -> Option<CacheCounterValue> {
        locked(&self.effects.reads).push(ReadRecord {
            key: key.to_owned(),
            local_only,
        });
        locked(&self.values).get(key).cloned()
    }

    async fn set_cache(&self, key: &str, value: CacheCounterValue, ttl: i64, local_only: bool) {
        locked(&self.effects.writes).push(WriteRecord {
            key: key.to_owned(),
            value: RecordedValue::Counter(value.clone()),
            ttl: Some(ttl),
            local_only,
        });
        locked(&self.values).insert(key.to_owned(), value);
    }

    async fn batch_get_cache(&self, _keys: &[String], _local_only: bool) -> Option<CounterValues> {
        None
    }

    fn redis_cache_kind(&self) -> Option<RedisCacheKind> {
        None
    }
}

pub(super) struct RecordingUsage {
    effects: Arc<Effects>,
}

impl RecordingUsage {
    pub(super) fn new(effects: Arc<Effects>) -> Self {
        Self { effects }
    }
}

#[async_trait::async_trait]
impl UsageIncrements for RecordingUsage {
    async fn increment(&self, operation: &IncrementOperation) {
        locked(&self.effects.increments).push(operation.clone());
    }

    async fn increment_pipeline(&self, operations: &[IncrementOperation]) {
        locked(&self.effects.pipelines).push(operations.to_vec());
    }

    async fn set_local_counter(&self, key: &str, value: f64, ttl: Option<i64>) {
        locked(&self.effects.writes).push(WriteRecord {
            key: key.to_owned(),
            value: RecordedValue::Local(value),
            ttl,
            local_only: true,
        });
    }
}

pub(super) struct FixedClock {
    at: SystemTime,
}

impl FixedClock {
    pub(super) fn at(seconds: u64) -> Option<Arc<Self>> {
        Some(Arc::new(Self {
            at: UNIX_EPOCH.checked_add(Duration::from_secs(seconds))?,
        }))
    }
}

impl LimiterClock for FixedClock {
    fn current_time(&self) -> SystemTime {
        self.at
    }
}

pub(super) struct GroupMap {
    groups: BTreeMap<String, String>,
}

impl GroupMap {
    pub(super) fn new(groups: BTreeMap<String, String>) -> Self {
        Self { groups }
    }
}

impl ModelGroupResolver for GroupMap {
    fn resolve(&self, model: &str) -> Option<String> {
        self.groups.get(model).cloned()
    }
}

pub(super) struct UnresolvedProviders;

impl ProviderResolution for UnresolvedProviders {
    fn resolve(&self, model: &str) -> Result<ResolvedModel, ResolutionError> {
        Err(ResolutionError::MissingProvider {
            model: model.to_owned(),
        })
    }
}
