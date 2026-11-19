use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_core::api::batch_enqueued_refund::record::ReservationRecord;
use betterlite_core::api::batch_enqueued_reserve::{
    BatchEnqueuedTokenReservation, ReservationBackend as CoreBackend,
};
use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_models::call_helpers::metadata::MetadataMap;
use betterlite_rate_limit::concurrency_gauges::ConcurrencyGauges;
use betterlite_rate_limit::concurrency_gauges::cache::GaugeCache;
use betterlite_rate_limit::fail_closed::FailClosedPolicy;
use betterlite_rate_limit::limiter_records::{
    LimiterClock, ModelGroupResolver, RateLimitResponse, ReservationAwareIncrement,
    ReservationWindowIdentity,
};
use betterlite_rate_limit::model_descriptors::ModelDescriptorBuilder;
use betterlite_rate_limit::post_call_settlement::settlement::PostCallSettlement;
use betterlite_rate_limit::rate_limit_window_store::WindowStore;
use betterlite_rate_limit::request_stash::{RequestStashSlot, StashState};
use betterlite_rate_limit::token_adjustments::store::TokenAdjustments;
use betterlite_redis::cache_primitives::pipeline_increment::IncrementOperation;
use serde_json::{Value, json};
use tokio::sync::Mutex as AsyncMutex;

use super::cases::{Case, StashCase, TpdOperationCase, WindowCase};
use super::fakes::{
    FakeBatchStore, FakeUsage, RecordingCounters, RecordingGauges, WINDOW, lock, operation_json,
};

pub(crate) struct FixedClock;

impl LimiterClock for FixedClock {
    fn current_time(&self) -> SystemTime {
        UNIX_EPOCH
    }
}

fn window_identity(case: &WindowCase) -> ReservationWindowIdentity {
    ReservationWindowIdentity {
        counter_key: case.counter_key.clone(),
        window_start: case.window_start.clone(),
        backend: case.backend,
    }
}

fn tpd_operation(case: &TpdOperationCase) -> ReservationAwareIncrement {
    ReservationAwareIncrement {
        operation: IncrementOperation {
            key: case.key.clone(),
            increment_value: case.increment_value,
            ttl: case.ttl,
        },
        window_key: case.window_key.clone(),
        expected_window_start: case.expected_window_start.clone(),
        reservation_backend: case.reservation_backend,
    }
}

fn reservation_of(input: &StashCase) -> Option<BatchEnqueuedTokenReservation> {
    let tokens = input.batch_tokens?;
    Some(BatchEnqueuedTokenReservation {
        tokens,
        scopes: input.batch_scopes.clone(),
        backend: CoreBackend::Redis,
        owner: "worker".to_owned(),
        reserved_at_monotonic: 0.0,
    })
}

fn rate_limit_response(input: &StashCase) -> Option<RateLimitResponse> {
    (!input.statuses.is_empty()).then(|| RateLimitResponse {
        overall_code: "OK".to_owned(),
        statuses: input.statuses.clone(),
        reservation_windows: None,
    })
}

fn install_stash(case: &Case) -> RequestStashSlot {
    let slot = RequestStashSlot::default();
    let Some(input) = case.stash.as_ref() else {
        return slot;
    };
    let stash = slot.get_or_create();
    let state = StashState {
        owner_litellm_call_id: input.owner_call_id.clone(),
        rate_limit_response: rate_limit_response(input),
        parallel_slot: input.parallel_slot.clone(),
        reserved_tokens: input.reserved_tokens,
        reserved_model: None,
        reserved_scopes: input.reserved_scopes.iter().cloned().collect(),
        itpm_reserved_tokens: input.itpm_reserved_tokens,
        itpm_reserved_scopes: input.itpm_reserved_scopes.iter().cloned().collect(),
        itpm_reserved_windows: input
            .itpm_reserved_windows
            .iter()
            .map(window_identity)
            .collect(),
        otpm_reserved_tokens: input.otpm_reserved_tokens,
        otpm_reserved_scopes: input.otpm_reserved_scopes.iter().cloned().collect(),
        otpm_reserved_windows: input
            .otpm_reserved_windows
            .iter()
            .map(window_identity)
            .collect(),
        batch_enqueued_reservation: reservation_of(input),
        batch_tpd_refund_ops: input.batch_tpd_ops.iter().map(tpd_operation).collect(),
        reservation_released: input.reservation_released,
        tpm_limited_tags: BTreeSet::new(),
    };
    *lock(&stash.state) = state;
    slot
}

fn popped_record(tokens: Option<i64>) -> Option<ReservationRecord> {
    let tokens = tokens?;
    Some(ReservationRecord {
        tokens,
        scopes: Vec::new(),
        backend: CoreBackend::Redis,
        owner: "worker".to_owned(),
        reserved_at_monotonic: 0.0,
    })
}

fn adjustments(counters: Arc<RecordingCounters>, usage: Arc<FakeUsage>) -> TokenAdjustments {
    TokenAdjustments {
        windows: WindowStore {
            cache: counters,
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

pub(crate) struct NoModels;

impl ModelGroupResolver for NoModels {
    fn resolve(&self, _model: &str) -> Option<String> {
        None
    }
}

pub(crate) struct Fixture {
    pub(crate) slot: RequestStashSlot,
    pub(crate) metadata: MetadataMap,
    pub(crate) models: ModelDescriptorBuilder,
    usage: Arc<FakeUsage>,
    gauges: Arc<RecordingGauges>,
    store: Arc<FakeBatchStore>,
    gauge_values: ConcurrencyGauges,
    adjustments: TokenAdjustments,
}

impl Fixture {
    pub(crate) fn new(case: &Case) -> Self {
        let slot = install_stash(case);
        let gauge_keys: Vec<String> = case
            .stash
            .as_ref()
            .and_then(|stash| stash.parallel_slot.as_ref())
            .map(|acquisition| acquisition.counter_keys.clone())
            .unwrap_or_default();
        let usage = Arc::new(FakeUsage::default());
        let counters = Arc::new(RecordingCounters::seeded(&case.seed));
        let gauges = Arc::new(RecordingGauges::seeded(&case.seed, &gauge_keys));
        let store = Arc::new(FakeBatchStore::new(popped_record(case.pop_tokens)));
        let gauge_cache: Arc<dyn GaugeCache> = Arc::<RecordingGauges>::clone(&gauges);
        Self {
            slot,
            metadata: MetadataMap::new(),
            models: ModelDescriptorBuilder {
                window_size: WINDOW,
                resolver: Box::new(NoModels),
            },
            usage: Arc::clone(&usage),
            gauge_values: ConcurrencyGauges {
                cache: gauge_cache,
                clock: Arc::new(FixedClock),
                acquire_script: None,
                release_script: None,
                count_script: None,
                release_batch: None,
                fail_closed: FailClosedPolicy { enabled: false },
                check_and_increment_lock: AsyncMutex::new(()),
            },
            adjustments: adjustments(Arc::clone(&counters), Arc::clone(&usage)),
            gauges,
            store,
        }
    }

    pub(crate) fn settlement(&self) -> PostCallSettlement<'_> {
        PostCallSettlement {
            gauges: &self.gauge_values,
            adjustments: &self.adjustments,
            batch_store: self.store.as_ref(),
        }
    }

    pub(crate) fn record(
        &self,
        slp_headers: Option<AdditionalHeaders>,
        response_headers: Option<AdditionalHeaders>,
    ) -> Value {
        let (parallel_slot, released, batch_tokens, tpd_ops) = match self.slot.get() {
            Some(stash) => {
                let state = lock(&stash.state);
                (
                    state.parallel_slot.clone(),
                    state.reservation_released,
                    state
                        .batch_enqueued_reservation
                        .as_ref()
                        .map(|reservation| reservation.tokens),
                    state.batch_tpd_refund_ops.len(),
                )
            }
            None => (None, false, None, 0),
        };
        let gauge_reads: Vec<Value> = lock(&self.gauges.reads).clone();
        let gauge_writes: Vec<Value> = lock(&self.gauges.writes).clone();
        let increments: Vec<Value> = lock(&self.usage.increments)
            .iter()
            .map(operation_json)
            .collect();
        let pipelines: Vec<Value> = lock(&self.usage.pipelines)
            .iter()
            .map(|operations| operations.iter().map(operation_json).collect())
            .collect();
        let local_sets: Vec<Value> = lock(&self.usage.local_sets)
            .iter()
            .map(|(key, value, ttl)| json!({"key": key, "value": value, "ttl": ttl}))
            .collect();
        let batch_calls: Vec<Value> = lock(&self.store.calls).clone();
        json!({
            "slp_headers": slp_headers,
            "response_headers": response_headers,
            "parallel_slot": parallel_slot,
            "reservation_released": released,
            "batch_tokens": batch_tokens,
            "tpd_ops": tpd_ops,
            "gauge_reads": gauge_reads,
            "gauge_writes": gauge_writes,
            "increments": increments,
            "pipelines": pipelines,
            "local_sets": local_sets,
            "batch_calls": batch_calls,
        })
    }
}
