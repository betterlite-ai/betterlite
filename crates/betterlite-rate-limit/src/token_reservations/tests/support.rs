use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_auth::rejection_provider::ProviderResolution;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::resolution::{ResolutionError, ResolvedModel};
use betterlite_redis::cache_primitives::pipeline_increment::IncrementOperation;
use betterlite_redis::cache_primitives::protocol::RedisScalar;
use betterlite_redis::redis_counters::RedisCountersError;
use tokio::sync::Mutex as AsyncMutex;

use crate::fail_closed::FailClosedPolicy;
use crate::limiter_records::{
    CacheCounterValue, LimiterClock, ModelGroupResolver, RateLimitDescriptor,
    RateLimitDescriptorRateLimit, RateLimitType, ReservationBackend, ReservationWindowIdentity,
    ScriptReply, create_rate_limit_keys,
};
use crate::model_descriptors::ModelDescriptorBuilder;
use crate::provider_attribution::RateLimitErrorAttribution;
use crate::rate_limit_window_store::{
    CounterCache, CounterScript, CounterValues, RedisCacheKind, WindowStore,
};
use crate::settlement_and_rollback::AtomicSettlement;
use crate::token_adjustments::store::{TokenAdjustments, UsageIncrements};
use crate::token_reservations::TokenReservations;

pub(crate) const WINDOW: i64 = 60;
pub(crate) const NOW: i64 = 1_700_000_000;
// NOW minus 30 seconds: a window that started inside the current 60-second window
pub(crate) const START: i64 = 1_699_999_970;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum WriteValue {
    Counter(CacheCounterValue),
    Local(f64),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct WriteRecord {
    pub(crate) key: String,
    pub(crate) value: WriteValue,
    pub(crate) ttl: Option<i64>,
    pub(crate) local_only: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ReadRecord {
    pub(crate) key: String,
    pub(crate) local_only: bool,
}

#[derive(Default)]
pub(crate) struct Effects {
    reads: Mutex<Vec<ReadRecord>>,
    writes: Mutex<Vec<WriteRecord>>,
    increments: Mutex<Vec<IncrementOperation>>,
    pipelines: Mutex<Vec<Vec<IncrementOperation>>>,
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Effects {
    pub(crate) fn reads(&self) -> Vec<ReadRecord> {
        locked(&self.reads).clone()
    }

    pub(crate) fn writes(&self) -> Vec<WriteRecord> {
        locked(&self.writes).clone()
    }

    pub(crate) fn increments(&self) -> Vec<IncrementOperation> {
        locked(&self.increments).clone()
    }

    pub(crate) fn pipelines(&self) -> Vec<Vec<IncrementOperation>> {
        locked(&self.pipelines).clone()
    }

    pub(crate) fn push_read(&self, key: &str, local_only: bool) {
        locked(&self.reads).push(ReadRecord {
            key: key.to_owned(),
            local_only,
        });
    }

    pub(crate) fn push_write(&self, write: WriteRecord) {
        locked(&self.writes).push(write);
    }
}

pub(crate) struct RecordingCache {
    values: Mutex<BTreeMap<String, CacheCounterValue>>,
    effects: Arc<Effects>,
}

impl RecordingCache {
    pub(crate) fn new(entries: &[(&str, CacheCounterValue)], effects: Arc<Effects>) -> Self {
        let values = entries
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect();
        Self {
            values: Mutex::new(values),
            effects,
        }
    }

    pub(crate) fn value(&self, key: &str) -> Option<CacheCounterValue> {
        locked(&self.values).get(key).cloned()
    }
}

#[async_trait::async_trait]
impl CounterCache for RecordingCache {
    async fn get_cache(&self, key: &str, local_only: bool) -> Option<CacheCounterValue> {
        self.effects.push_read(key, local_only);
        locked(&self.values).get(key).cloned()
    }

    async fn set_cache(&self, key: &str, value: CacheCounterValue, ttl: i64, local_only: bool) {
        self.effects.push_write(write_record(
            key,
            WriteValue::Counter(value.clone()),
            Some(ttl),
            local_only,
        ));
        locked(&self.values).insert(key.to_owned(), value);
    }

    async fn batch_get_cache(&self, _keys: &[String], _local_only: bool) -> Option<CounterValues> {
        None
    }

    fn redis_cache_kind(&self) -> Option<RedisCacheKind> {
        None
    }
}

pub(crate) struct RecordingUsage {
    effects: Arc<Effects>,
}

impl RecordingUsage {
    pub(crate) fn new(effects: Arc<Effects>) -> Self {
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
        self.effects
            .push_write(write_record(key, WriteValue::Local(value), ttl, true));
    }
}

#[derive(Default)]
pub(crate) struct RecordingScript {
    calls: Mutex<Vec<(Vec<String>, Vec<RedisScalar>)>>,
}

impl RecordingScript {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub(crate) fn calls(&self) -> Vec<(Vec<String>, Vec<RedisScalar>)> {
        locked(&self.calls).clone()
    }
}

#[async_trait::async_trait]
impl CounterScript for RecordingScript {
    fn source(&self) -> &'static str {
        "token reservations test script"
    }

    async fn run(
        &self,
        keys: &[String],
        args: &[RedisScalar],
    ) -> Result<ScriptReply, RedisCountersError> {
        locked(&self.calls).push((keys.to_vec(), args.to_vec()));
        Ok(ScriptReply::Array(Vec::new()))
    }
}

pub(crate) struct FixedClock {
    at: SystemTime,
}

impl FixedClock {
    pub(crate) fn at(seconds: i64) -> Arc<Self> {
        let elapsed = u64::try_from(seconds).expect("the clock time is non-negative");
        Arc::new(Self {
            at: UNIX_EPOCH
                .checked_add(Duration::from_secs(elapsed))
                .expect("the clock time fits a SystemTime"),
        })
    }
}

impl LimiterClock for FixedClock {
    fn current_time(&self) -> SystemTime {
        self.at
    }
}

pub(crate) struct GroupMap {
    groups: BTreeMap<String, String>,
}

impl GroupMap {
    pub(crate) fn new(pairs: &[(&str, &str)]) -> Self {
        Self {
            groups: pairs
                .iter()
                .map(|(model, group)| ((*model).to_owned(), (*group).to_owned()))
                .collect(),
        }
    }
}

impl ModelGroupResolver for GroupMap {
    fn resolve(&self, model: &str) -> Option<String> {
        self.groups.get(model).cloned()
    }
}

pub(crate) struct UnresolvedProviders;

impl ProviderResolution for UnresolvedProviders {
    fn resolve(&self, model: &str) -> Result<ResolvedModel, ResolutionError> {
        Err(ResolutionError::MissingProvider {
            model: model.to_owned(),
        })
    }
}

pub(crate) fn descriptor(
    key: &str,
    value: &str,
    tokens_per_unit: Option<i64>,
    window_size: Option<i64>,
) -> RateLimitDescriptor {
    RateLimitDescriptor {
        key: key.to_owned(),
        value: value.to_owned(),
        rate_limit: Some(RateLimitDescriptorRateLimit {
            requests_per_unit: None,
            tokens_per_unit,
            max_parallel_requests: None,
            window_size,
        }),
    }
}

pub(crate) fn tokens_key(key: &str, value: &str) -> String {
    create_rate_limit_keys(key, value, RateLimitType::Tokens)
}

pub(crate) fn window_key(key: &str, value: &str) -> String {
    format!("{{{key}:{value}}}:window")
}

pub(crate) fn text(value: i64) -> CacheCounterValue {
    CacheCounterValue::Text(value.to_string())
}

pub(crate) const fn count(value: i64) -> CacheCounterValue {
    CacheCounterValue::Int(value)
}

pub(crate) fn identity(
    counter_key: &str,
    window_start: &str,
    backend: ReservationBackend,
) -> ReservationWindowIdentity {
    ReservationWindowIdentity {
        counter_key: counter_key.to_owned(),
        window_start: window_start.to_owned(),
        backend,
    }
}

pub(crate) fn scope(key: &str, value: &str) -> (String, String) {
    (key.to_owned(), value.to_owned())
}

pub(crate) fn read(key: &str) -> ReadRecord {
    ReadRecord {
        key: key.to_owned(),
        local_only: true,
    }
}

pub(crate) fn write_record(
    key: &str,
    value: WriteValue,
    ttl: Option<i64>,
    local_only: bool,
) -> WriteRecord {
    WriteRecord {
        key: key.to_owned(),
        value,
        ttl,
        local_only,
    }
}

pub(crate) fn counter_write(key: &str, value: CacheCounterValue, ttl: i64) -> WriteRecord {
    write_record(key, WriteValue::Counter(value), Some(ttl), true)
}

pub(crate) fn local_write(key: &str, value: f64, ttl: i64) -> WriteRecord {
    write_record(key, WriteValue::Local(value), Some(ttl), true)
}

pub(crate) fn increment_tuples(
    operations: &[IncrementOperation],
) -> Vec<(String, f64, Option<i64>)> {
    operations
        .iter()
        .map(|operation| {
            (
                operation.key.clone(),
                operation.increment_value,
                operation.ttl,
            )
        })
        .collect()
}

pub(crate) fn pipeline_tuples(
    pipelines: &[Vec<IncrementOperation>],
) -> Vec<Vec<(String, f64, Option<i64>)>> {
    pipelines
        .iter()
        .map(Vec::as_slice)
        .map(increment_tuples)
        .collect()
}

pub(crate) fn auth(fields: &str) -> UserAPIKeyAuth {
    let decoded: BTreeMap<String, JsonData> =
        serde_json::from_str(fields).expect("the auth fields decode");
    UserAPIKeyAuth::construct(AuthInput::Dict(decoded)).expect("the auth fixture constructs")
}

pub(crate) struct ReservationFixture {
    pub(crate) effects: Arc<Effects>,
    pub(crate) cache: Arc<RecordingCache>,
    adjustments: TokenAdjustments,
    descriptors: ModelDescriptorBuilder,
    clock: Arc<FixedClock>,
    providers: UnresolvedProviders,
}

impl ReservationFixture {
    pub(crate) fn seeded(entries: &[(&str, CacheCounterValue)]) -> Self {
        Self::at(entries, NOW, &[])
    }

    pub(crate) fn at(
        entries: &[(&str, CacheCounterValue)],
        now: i64,
        groups: &[(&str, &str)],
    ) -> Self {
        let effects = Arc::new(Effects::default());
        let cache = Arc::new(RecordingCache::new(entries, Arc::clone(&effects)));
        let usage = Arc::new(RecordingUsage::new(Arc::clone(&effects)));
        let adjustments = TokenAdjustments {
            windows: WindowStore {
                cache: Arc::clone(&cache),
                rate_limiter_script: None,
                counter_read_script: None,
                request_batch: None,
                check_and_increment_lock: AsyncMutex::new(()),
            },
            usage,
            post_call_batch: None,
            token_increment_script: None,
            guarded_token_script: None,
        };
        Self {
            effects,
            cache,
            adjustments,
            descriptors: ModelDescriptorBuilder {
                window_size: WINDOW,
                resolver: Box::new(GroupMap::new(groups)),
            },
            clock: FixedClock::at(now),
            providers: UnresolvedProviders,
        }
    }

    pub(crate) fn with_guarded_script(mut self, script: Arc<dyn CounterScript>) -> Self {
        self.adjustments.guarded_token_script = Some(script);
        self
    }

    pub(crate) fn reservations(&self) -> TokenReservations<'_> {
        TokenReservations {
            settlement: AtomicSettlement {
                windows: &self.adjustments.windows,
                clock: self.clock.as_ref(),
                script: None,
                refund_script: None,
                fail_closed: FailClosedPolicy { enabled: false },
                default_window_size: WINDOW,
            },
            adjustments: &self.adjustments,
            descriptors: &self.descriptors,
            attribution: RateLimitErrorAttribution {
                requested_model: None,
                native: &self.providers,
                router: None,
            },
        }
    }
}
