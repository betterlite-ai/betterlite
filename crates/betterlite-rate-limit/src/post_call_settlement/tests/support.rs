use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_config::sections::rate_limit_usage::TokenRateLimitType;
use betterlite_core::api::batch_enqueued_refund::BatchEnqueuedRefundError;
use betterlite_core::api::batch_enqueued_refund::record::ReservationRecord;
use betterlite_core::api::batch_enqueued_reserve::{
    BatchEnqueuedTokenReservation, BatchEnqueuedTokenScope, ReservationBackend as BatchBackend,
    ScopeKey,
};
use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_models::call_helpers::metadata::MetadataMap;
use betterlite_models::decisions::Json;
use betterlite_redis::cache_primitives::pipeline_increment::IncrementOperation;
use tokio::sync::Mutex as AsyncMutex;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt;

use crate::concurrency_gauges::ConcurrencyGauges;
use crate::concurrency_gauges::cache::{GaugeCache, GaugeCacheValue};
use crate::fail_closed::FailClosedPolicy;
use crate::limiter_records::ModelGroupResolver;
use crate::limiter_records::{
    CacheCounterValue, LimiterClock, RateLimitResponse, RateLimitStatus, RateLimitType,
    ReservationAwareIncrement, ReservationBackend, ReservationWindowIdentity,
};
use crate::model_descriptors::ModelDescriptorBuilder;
use crate::post_call_settlement::settlement::PostCallSettlement;
use crate::post_call_settlement::success_event::SuccessCallback;
use crate::rate_limit_window_store::{CounterCache, CounterValues, RedisCacheKind, WindowStore};
use crate::reconciliation_operations::io_token::{IoTokenReconciliation, IoTokenResponse};
use crate::reconciliation_operations::scope_targets::ScopeMetadata;
use crate::reconciliation_operations::success_event::{SuccessEvent, SuccessResponse};
use crate::request_stash::{RequestStash, RequestStashSlot, StashState};
use crate::token_adjustments::store::{TokenAdjustments, UsageIncrements};
use crate::usage_resolution::Usage;

use super::super::batch_enqueued::BatchEnqueuedTokenStore;

pub(crate) const WINDOW: i64 = 60;
pub(crate) const NOW: f64 = 1_700_000_000.0;

pub(crate) fn status(
    descriptor_key: &str,
    rate_limit_type: RateLimitType,
    current_limit: i64,
    limit_remaining: i64,
) -> RateLimitStatus {
    RateLimitStatus {
        code: "OK".to_owned(),
        current_limit,
        limit_remaining,
        rate_limit_type,
        descriptor_key: descriptor_key.to_owned(),
        descriptor_value: None,
    }
}

pub(crate) fn rate_limit_response(statuses: Vec<RateLimitStatus>) -> RateLimitResponse {
    RateLimitResponse {
        overall_code: "OK".to_owned(),
        statuses,
        reservation_windows: None,
    }
}

pub(crate) fn headers(entries: &[(&str, Json)]) -> AdditionalHeaders {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

pub(crate) fn number(value: i64) -> Json {
    Json::Number(serde_json::Number::from(value))
}

pub(crate) fn scopes(pairs: &[(&str, &str)]) -> BTreeSet<(String, String)> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

pub(crate) fn window_identity(
    counter_key: &str,
    backend: ReservationBackend,
) -> ReservationWindowIdentity {
    ReservationWindowIdentity {
        counter_key: counter_key.to_owned(),
        window_start: "100".to_owned(),
        backend,
    }
}

fn reservation_increment(
    key: &str,
    increment_value: f64,
    backend: Option<ReservationBackend>,
) -> ReservationAwareIncrement {
    ReservationAwareIncrement {
        operation: IncrementOperation {
            key: key.to_owned(),
            increment_value,
            ttl: Some(WINDOW),
        },
        window_key: backend.map(|_| "{scope:value}:window".to_owned()),
        expected_window_start: backend.map(|_| "100".to_owned()),
        reservation_backend: backend,
    }
}

pub(crate) fn unguarded_operation(key: &str, increment_value: f64) -> ReservationAwareIncrement {
    reservation_increment(key, increment_value, None)
}

pub(crate) struct NoModels;

impl ModelGroupResolver for NoModels {
    fn resolve(&self, _model: &str) -> Option<String> {
        None
    }
}

pub(crate) fn model_builder() -> ModelDescriptorBuilder {
    ModelDescriptorBuilder {
        window_size: WINDOW,
        resolver: Box::new(NoModels),
    }
}

pub(crate) fn install<F>(slot: &RequestStashSlot, configure: F) -> Arc<RequestStash>
where
    F: FnOnce(&mut StashState),
{
    let stash = slot.get_or_create();
    let mut state = stash.state.lock().unwrap_or_else(PoisonError::into_inner);
    configure(&mut state);
    drop(state);
    stash
}

pub(crate) fn stash_state(slot: &RequestStashSlot) -> StashState {
    let stash = slot.get().expect("the stash is installed");
    let state = stash.state.lock().unwrap_or_else(PoisonError::into_inner);
    StashState {
        owner_litellm_call_id: state.owner_litellm_call_id.clone(),
        rate_limit_response: state.rate_limit_response.clone(),
        parallel_slot: state.parallel_slot.clone(),
        reserved_tokens: state.reserved_tokens,
        reserved_model: state.reserved_model.clone(),
        reserved_scopes: state.reserved_scopes.clone(),
        itpm_reserved_tokens: state.itpm_reserved_tokens,
        itpm_reserved_scopes: state.itpm_reserved_scopes.clone(),
        itpm_reserved_windows: state.itpm_reserved_windows.clone(),
        otpm_reserved_tokens: state.otpm_reserved_tokens,
        otpm_reserved_scopes: state.otpm_reserved_scopes.clone(),
        otpm_reserved_windows: state.otpm_reserved_windows.clone(),
        batch_enqueued_reservation: state.batch_enqueued_reservation.clone(),
        batch_tpd_refund_ops: state.batch_tpd_refund_ops.clone(),
        reservation_released: state.reservation_released,
        tpm_limited_tags: state.tpm_limited_tags.clone(),
    }
}

pub(crate) fn reservation(
    tokens: i64,
    scope_pairs: &[(&str, &str)],
) -> BatchEnqueuedTokenReservation {
    BatchEnqueuedTokenReservation {
        tokens,
        scopes: scope_pairs
            .iter()
            .map(|(_key, value)| BatchEnqueuedTokenScope {
                key: ScopeKey::ApiKey,
                value: (*value).to_owned(),
                limit: 0,
            })
            .collect(),
        backend: BatchBackend::Redis,
        owner: "worker".to_owned(),
        reserved_at_monotonic: 0.0,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BatchCall {
    pub(crate) kind: &'static str,
    pub(crate) batch_id: Option<String>,
    pub(crate) tokens: Option<i64>,
}

#[derive(Default)]
pub(crate) struct FakeBatchStore {
    calls: Mutex<Vec<BatchCall>>,
    failure: Mutex<Option<&'static str>>,
    popped: Mutex<Option<ReservationRecord>>,
}

impl FakeBatchStore {
    pub(crate) fn fail_only(&self, kind: &'static str) {
        *self.failure.lock().unwrap_or_else(PoisonError::into_inner) = Some(kind);
    }

    pub(crate) fn pop_returns(&self, reservation: &BatchEnqueuedTokenReservation) {
        *self.popped.lock().unwrap_or_else(PoisonError::into_inner) =
            Some(ReservationRecord::from(reservation));
    }

    pub(crate) fn calls(&self) -> Vec<BatchCall> {
        self.calls.lock().expect("the batch lock").clone()
    }

    fn record_reservation(
        &self,
        kind: &'static str,
        batch_id: Option<&str>,
        tokens: i64,
    ) -> Result<(), BatchEnqueuedRefundError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(BatchCall {
                kind,
                batch_id: batch_id.map(str::to_owned),
                tokens: Some(tokens),
            });
        self.recorded(kind)
    }

    fn recorded(&self, kind: &'static str) -> Result<(), BatchEnqueuedRefundError> {
        let failing = *self.failure.lock().unwrap_or_else(PoisonError::into_inner);
        let error =
            serde_json::from_str::<i64>("the fake store refused").expect_err("not a number");
        match failing {
            Some(failed) if failed == kind => {
                return Err(BatchEnqueuedRefundError::Encode {
                    source: Box::new(error),
                });
            }
            Some(_) | None => {}
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl BatchEnqueuedTokenStore for FakeBatchStore {
    async fn save_reservation(
        &self,
        batch_id: &str,
        reservation: &BatchEnqueuedTokenReservation,
    ) -> Result<(), BatchEnqueuedRefundError> {
        self.record_reservation("save", Some(batch_id), reservation.tokens)
    }

    async fn pop_reservation(
        &self,
        batch_id: &str,
    ) -> Result<Option<ReservationRecord>, BatchEnqueuedRefundError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(BatchCall {
                kind: "pop",
                batch_id: Some(batch_id.to_owned()),
                tokens: None,
            });
        self.recorded("pop")?;
        Ok(self
            .popped
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone())
    }

    async fn refund(
        &self,
        reservation: &ReservationRecord,
    ) -> Result<(), BatchEnqueuedRefundError> {
        self.record_reservation("refund", None, reservation.tokens)
    }
}

#[derive(Default)]
pub(crate) struct FakeUsage {
    increments: Mutex<Vec<IncrementOperation>>,
    pipelines: Mutex<Vec<Vec<IncrementOperation>>>,
    local_sets: Mutex<Vec<(String, f64, Option<i64>)>>,
}

impl FakeUsage {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub(crate) fn increments(&self) -> Vec<IncrementOperation> {
        self.increments.lock().expect("the usage lock").clone()
    }

    pub(crate) fn pipelines(&self) -> Vec<Vec<IncrementOperation>> {
        self.pipelines.lock().expect("the usage lock").clone()
    }
}

#[async_trait::async_trait]
impl UsageIncrements for FakeUsage {
    async fn increment(&self, operation: &IncrementOperation) {
        self.increments
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(operation.clone());
    }

    async fn increment_pipeline(&self, operations: &[IncrementOperation]) {
        self.pipelines
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(operations.to_vec());
    }

    async fn set_local_counter(&self, key: &str, value: f64, ttl: Option<i64>) {
        self.local_sets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((key.to_owned(), value, ttl));
    }
}

#[derive(Default)]
pub(crate) struct FakeCounterCache {
    values: Mutex<BTreeMap<String, CacheCounterValue>>,
}

fn seed_map<T>(entries: &[(&str, T)]) -> BTreeMap<String, T>
where
    T: Clone,
{
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

impl FakeCounterCache {
    pub(crate) fn seeded(entries: &[(&str, CacheCounterValue)]) -> Arc<Self> {
        Arc::new(Self {
            values: Mutex::new(seed_map(entries)),
        })
    }
}

#[async_trait::async_trait]
impl CounterCache for FakeCounterCache {
    async fn get_cache(&self, key: &str, _local_only: bool) -> Option<CacheCounterValue> {
        self.values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .cloned()
    }

    async fn set_cache(&self, key: &str, value: CacheCounterValue, _ttl: i64, _local_only: bool) {
        self.values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), value);
    }

    async fn batch_get_cache(&self, keys: &[String], _local_only: bool) -> Option<CounterValues> {
        let values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        Some(keys.iter().map(|key| values.get(key).cloned()).collect())
    }

    fn redis_cache_kind(&self) -> Option<RedisCacheKind> {
        None
    }
}

pub(crate) fn adjustments_with(
    usage: Arc<FakeUsage>,
    cache: Arc<FakeCounterCache>,
) -> TokenAdjustments {
    TokenAdjustments {
        windows: WindowStore {
            cache,
            rate_limiter_script: None,
            counter_read_script: None,
            request_batch: None,
            check_and_increment_lock: AsyncMutex::new(()),
        },
        usage,
        post_call_batch: None,
        token_increment_script: None,
        guarded_token_script: None,
    }
}

pub(crate) struct FixedClock {
    at: SystemTime,
}

impl FixedClock {
    pub(crate) fn at(seconds: f64) -> Arc<Self> {
        let elapsed = Duration::try_from_secs_f64(seconds).expect("a valid clock time");
        Arc::new(Self {
            at: UNIX_EPOCH
                .checked_add(elapsed)
                .expect("a representable time"),
        })
    }
}

impl LimiterClock for FixedClock {
    fn current_time(&self) -> SystemTime {
        self.at
    }
}

pub(crate) struct RecordingGaugeCache {
    values: Mutex<BTreeMap<String, GaugeCacheValue>>,
    reads: Mutex<Vec<(String, bool)>>,
    writes: Mutex<Vec<(String, GaugeCacheValue, u64, bool)>>,
}

impl RecordingGaugeCache {
    pub(crate) fn new(entries: &[(&str, GaugeCacheValue)]) -> Arc<Self> {
        Arc::new(Self {
            values: Mutex::new(seed_map(entries)),
            reads: Mutex::new(Vec::new()),
            writes: Mutex::new(Vec::new()),
        })
    }
}

#[async_trait::async_trait]
impl GaugeCache for RecordingGaugeCache {
    async fn get(&self, key: &str, local_only: bool) -> Option<GaugeCacheValue> {
        self.reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((key.to_owned(), local_only));
        self.values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .cloned()
    }

    async fn set(&self, key: &str, value: GaugeCacheValue, ttl: Duration, local_only: bool) {
        self.writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((key.to_owned(), value.clone(), ttl.as_secs(), local_only));
        self.values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), value);
    }

    async fn batch_get(
        &self,
        keys: &[String],
        _local_only: bool,
    ) -> Option<Vec<Option<GaugeCacheValue>>> {
        let values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        Some(keys.iter().map(|key| values.get(key).cloned()).collect())
    }
}

pub(crate) fn gauges_with(cache: Arc<RecordingGaugeCache>) -> ConcurrencyGauges {
    ConcurrencyGauges {
        cache,
        clock: FixedClock::at(NOW),
        acquire_script: None,
        release_script: None,
        count_script: None,
        release_batch: None,
        fail_closed: FailClosedPolicy { enabled: false },
        check_and_increment_lock: AsyncMutex::new(()),
    }
}

fn operation_tuple(operation: &IncrementOperation) -> (String, u64, Option<i64>) {
    (
        operation.key.clone(),
        operation.increment_value.to_bits(),
        operation.ttl,
    )
}

pub(crate) fn operation_tuples(
    operations: &[IncrementOperation],
) -> Vec<(String, u64, Option<i64>)> {
    operations.iter().map(operation_tuple).collect()
}

pub(crate) fn pipeline_tuples(
    pipelines: &[Vec<IncrementOperation>],
) -> Vec<Vec<(String, u64, Option<i64>)>> {
    pipelines
        .iter()
        .map(|operations| operations.iter().map(operation_tuple).collect())
        .collect()
}

struct Captured(Arc<Mutex<Vec<u8>>>);

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) struct LogCapture {
    buffer: Arc<Mutex<Vec<u8>>>,
    _guard: DefaultGuard,
}

impl LogCapture {
    pub(crate) fn start() -> Self {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&buffer);
        let subscriber = fmt()
            .without_time()
            .with_ansi(false)
            .with_max_level(Level::TRACE)
            .with_writer(move || Captured(Arc::clone(&sink)))
            .finish();
        Self {
            buffer,
            _guard: set_default(subscriber),
        }
    }

    pub(crate) fn text(&self) -> String {
        let buffer = self.buffer.lock().unwrap_or_else(PoisonError::into_inner);
        String::from_utf8(buffer.clone()).expect("the capture is UTF-8")
    }
}

pub(crate) struct SettlementFixture {
    pub(crate) usage: Arc<FakeUsage>,
    pub(crate) store: FakeBatchStore,
    gauges: ConcurrencyGauges,
    adjustments: TokenAdjustments,
}

impl Default for SettlementFixture {
    fn default() -> Self {
        Self::with_cache(FakeCounterCache::seeded(&[]))
    }
}

impl SettlementFixture {
    pub(crate) fn with_cache(cache: Arc<FakeCounterCache>) -> Self {
        let usage = FakeUsage::new();
        let adjustments = adjustments_with(Arc::clone(&usage), cache);
        Self {
            usage,
            store: FakeBatchStore::default(),
            gauges: gauges_with(RecordingGaugeCache::new(&[])),
            adjustments,
        }
    }

    pub(crate) fn settlement(&self) -> PostCallSettlement<'_> {
        PostCallSettlement {
            gauges: &self.gauges,
            adjustments: &self.adjustments,
            batch_store: &self.store,
        }
    }
}

pub(crate) struct SuccessFixture {
    pub(crate) slot: RequestStashSlot,
    pub(crate) logging: ScopeMetadata,
    pub(crate) metadata: MetadataMap,
    pub(crate) models: ModelDescriptorBuilder,
    pub(crate) usage: Option<Usage>,
    pub(crate) rate_limit_type: TokenRateLimitType,
    pub(crate) call_id: Option<String>,
}

impl Default for SuccessFixture {
    fn default() -> Self {
        Self::bare(None, None)
    }
}

pub(crate) fn token_usage(prompt_tokens: u64, completion_tokens: u64, total_tokens: u64) -> Usage {
    Usage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        input_tokens: 0,
        output_tokens: 0,
        prompt_tokens_details: None,
        input_tokens_details: None,
        cached_tokens: None,
        cache_read_input_tokens: None,
    }
}

impl SuccessFixture {
    fn bare(usage: Option<Usage>, call_id: Option<String>) -> Self {
        Self {
            slot: RequestStashSlot::default(),
            logging: ScopeMetadata::default(),
            metadata: MetadataMap::new(),
            models: model_builder(),
            usage,
            rate_limit_type: TokenRateLimitType::Total,
            call_id,
        }
    }

    pub(crate) fn with_usage(usage: Usage) -> Self {
        Self::bare(Some(usage), None)
    }

    pub(crate) fn with_call_id(call_id: &str) -> Self {
        Self::bare(None, Some(call_id.to_owned()))
    }

    pub(crate) fn callback(&self, io: &RerankIo) -> SuccessCallback<'_> {
        SuccessCallback {
            event: self.event(),
            io: IoTokenReconciliation {
                response: IoTokenResponse::Rerank {
                    tokens: io.tokens,
                    billed_total: io.billed_total,
                },
                combined_usage: None,
                window_size: WINDOW,
            },
        }
    }

    pub(crate) fn event(&self) -> SuccessEvent<'_> {
        SuccessEvent {
            response: SuccessResponse::Unmodelled,
            combined_usage: self.usage.as_ref(),
            rate_limit_type: self.rate_limit_type,
            logging: &self.logging,
            request_metadata: &self.metadata,
            request_user: None,
            model_group: None,
            stash: &self.slot,
            call_id: self.call_id.as_deref(),
            models: &self.models,
            window_size: WINDOW,
        }
    }
}

#[derive(Default)]
pub(crate) struct RerankIo {
    pub(crate) tokens: Option<(u64, u64)>,
    pub(crate) billed_total: Option<u64>,
}
