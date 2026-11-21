use std::collections::BTreeMap;
use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_core::api::batch_enqueued_refund::BatchEnqueuedRefundError;
use betterlite_core::api::batch_enqueued_refund::record::ReservationRecord;
use betterlite_core::api::batch_enqueued_reserve::BatchEnqueuedTokenReservation;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_logging::fields::payload::records::StandardLoggingMetadata;
use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_models::decisions::Json;
use betterlite_redis::cache_primitives::pipeline_increment::IncrementOperation;
use indexmap::IndexMap;
use tokio::sync::Mutex as AsyncMutex;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::{Context, SubscriberExt as _};
use tracing_subscriber::registry;

use crate::concurrency_gauges::ConcurrencyGauges;
use crate::concurrency_gauges::cache::{GaugeCache, GaugeCacheValue};
use crate::dynamic_priority::settings::{PriorityPolicy, PrioritySettings};
use crate::fail_closed::FailClosedPolicy;
use crate::limiter_records::{
    CacheCounterValue, LimiterClock, ParallelSlotAcquisition, RateLimitResponse, RateLimitStatus,
};
use crate::post_call_settlement::batch_enqueued::BatchEnqueuedTokenStore;
use crate::post_call_settlement::settlement::PostCallSettlement;
use crate::provider_attribution::PriorityValue;
use crate::rate_limit_window_store::{CounterCache, CounterValues, RedisCacheKind, WindowStore};
use crate::request_stash::RequestStashSlot;
use crate::token_adjustments::store::{TokenAdjustments, UsageIncrements};
use crate::usage_resolution::Usage;

pub(crate) const WINDOW: i64 = 60;

#[derive(Default)]
pub(crate) struct RecordingUsage {
    pipelines: Mutex<Vec<Vec<IncrementOperation>>>,
}

impl RecordingUsage {
    pub(crate) fn pipelines(&self) -> Vec<Vec<IncrementOperation>> {
        self.pipelines
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait::async_trait]
impl UsageIncrements for RecordingUsage {
    async fn increment(&self, _operation: &IncrementOperation) {}

    async fn increment_pipeline(&self, operations: &[IncrementOperation]) {
        self.pipelines
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(operations.to_vec());
    }

    async fn set_local_counter(&self, _key: &str, _value: f64, _ttl: Option<i64>) {}
}

pub(crate) struct EmptyCounters;

#[async_trait::async_trait]
impl CounterCache for EmptyCounters {
    async fn get_cache(&self, _key: &str, _local_only: bool) -> Option<CacheCounterValue> {
        None
    }

    async fn set_cache(&self, _key: &str, _value: CacheCounterValue, _ttl: i64, _local_only: bool) {
    }

    async fn batch_get_cache(&self, keys: &[String], _local_only: bool) -> Option<CounterValues> {
        Some(keys.iter().map(|_key| None).collect())
    }

    fn redis_cache_kind(&self) -> Option<RedisCacheKind> {
        None
    }
}

pub(crate) struct EmptyGauges;

#[async_trait::async_trait]
impl GaugeCache for EmptyGauges {
    async fn get(&self, _key: &str, _local_only: bool) -> Option<GaugeCacheValue> {
        None
    }

    async fn set(&self, _key: &str, _value: GaugeCacheValue, _ttl: Duration, _local_only: bool) {}

    async fn batch_get(
        &self,
        keys: &[String],
        _local_only: bool,
    ) -> Option<Vec<Option<GaugeCacheValue>>> {
        Some(keys.iter().map(|_key| None).collect())
    }
}

pub(crate) struct EpochClock;

impl LimiterClock for EpochClock {
    fn current_time(&self) -> SystemTime {
        UNIX_EPOCH
    }
}

#[derive(Default)]
pub(crate) struct RecordingStore;

#[async_trait::async_trait]
impl BatchEnqueuedTokenStore for RecordingStore {
    async fn save_reservation(
        &self,
        _batch_id: &str,
        _reservation: &BatchEnqueuedTokenReservation,
    ) -> Result<(), BatchEnqueuedRefundError> {
        Ok(())
    }

    async fn pop_reservation(
        &self,
        _batch_id: &str,
    ) -> Result<Option<ReservationRecord>, BatchEnqueuedRefundError> {
        Ok(None)
    }

    async fn refund(
        &self,
        _reservation: &ReservationRecord,
    ) -> Result<(), BatchEnqueuedRefundError> {
        Ok(())
    }
}

pub(crate) struct CallbackFixture {
    pub(crate) usage: Arc<RecordingUsage>,
    pub(crate) store: RecordingStore,
    pub(crate) gauges: ConcurrencyGauges,
    pub(crate) adjustments: TokenAdjustments,
}

impl Default for CallbackFixture {
    fn default() -> Self {
        Self::new()
    }
}

impl CallbackFixture {
    pub(crate) fn new() -> Self {
        let usage = Arc::new(RecordingUsage::default());
        let incremented: Arc<dyn UsageIncrements> = Arc::<RecordingUsage>::clone(&usage);
        Self {
            usage,
            store: RecordingStore::default(),
            gauges: ConcurrencyGauges {
                cache: Arc::new(EmptyGauges),
                clock: Arc::new(EpochClock),
                acquire_script: None,
                release_script: None,
                count_script: None,
                release_batch: None,
                fail_closed: FailClosedPolicy { enabled: false },
                check_and_increment_lock: AsyncMutex::new(()),
            },
            adjustments: TokenAdjustments {
                windows: WindowStore {
                    cache: Arc::new(EmptyCounters),
                    rate_limiter_script: None,
                    counter_read_script: None,
                    request_batch: None,
                    check_and_increment_lock: AsyncMutex::new(()),
                },
                usage: incremented,
                post_call_batch: None,
                token_increment_script: None,
                guarded_token_script: None,
            },
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

pub(crate) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

pub(crate) fn number(value: i64) -> Json {
    Json::Number(serde_json::Number::from(value))
}

pub(crate) fn headers(entries: &[(&str, Json)]) -> AdditionalHeaders {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

pub(crate) fn stash_with_statuses(statuses: Vec<RateLimitStatus>) -> RequestStashSlot {
    let slot = RequestStashSlot::default();
    let stash = slot.get_or_create();
    let mut state = stash.state.lock().unwrap_or_else(PoisonError::into_inner);
    state.rate_limit_response = Some(RateLimitResponse {
        overall_code: "OK".to_owned(),
        statuses,
        reservation_windows: None,
    });
    state.parallel_slot = Some(ParallelSlotAcquisition {
        slot_id: "slot-1".to_owned(),
        counter_keys: vec!["counter-1".to_owned()],
    });
    drop(state);
    slot
}

pub(crate) fn slot_released(slot: &RequestStashSlot) -> bool {
    slot.get().is_none_or(|stash| {
        stash
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .parallel_slot
            .is_none()
    })
}

pub(crate) fn metadata_map(entries: &[(&str, JsonData)]) -> BTreeMap<String, JsonData> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

pub(crate) fn logging_metadata(priority: Option<&str>) -> StandardLoggingMetadata {
    let mut auth_metadata = IndexMap::new();
    if let Some(name) = priority {
        auth_metadata.insert("priority".to_owned(), name.to_owned());
    }
    StandardLoggingMetadata {
        user_api_key_hash: None,
        user_api_key_alias: None,
        user_api_key_spend: None,
        user_api_key_max_budget: None,
        user_api_key_budget_reset_at: None,
        user_api_key_user_spend: None,
        user_api_key_user_max_budget: None,
        user_api_key_team_spend: None,
        user_api_key_team_max_budget: None,
        user_api_key_org_id: None,
        user_api_key_org_alias: None,
        user_api_key_team_id: None,
        user_api_key_project_id: None,
        user_api_key_project_alias: None,
        user_api_key_user_id: None,
        user_api_key_user_email: None,
        user_api_key_team_alias: None,
        user_api_key_end_user_id: None,
        user_api_key_request_route: None,
        user_api_key_auth_metadata: Some(auth_metadata),
        spend_logs_metadata: None,
        requester_ip_address: None,
        user_agent: None,
        requester_metadata: None,
        requester_custom_headers: None,
        prompt_management_metadata: None,
        mcp_tool_call_metadata: None,
        vector_store_request_metadata: None,
        routing_decision: None,
        applied_guardrails: None,
        usage_object: None,
        cold_storage_object_key: None,
        team_alias: None,
        team_id: None,
        used_client_oauth_token: None,
    }
}

pub(crate) fn policy(entries: &[(&str, f64)]) -> PriorityPolicy {
    PriorityPolicy {
        settings: PrioritySettings::default(),
        reservation: Some(
            entries
                .iter()
                .map(|(name, weight)| ((*name).to_owned(), PriorityValue::Percent(*weight)))
                .collect(),
        ),
    }
}

pub(crate) fn usage(prompt_tokens: u64, completion_tokens: u64, total_tokens: u64) -> Usage {
    Usage {
        prompt_tokens,
        completion_tokens,
        input_tokens: 0,
        output_tokens: 0,
        total_tokens,
        prompt_tokens_details: None,
        input_tokens_details: None,
        cached_tokens: None,
        cache_read_input_tokens: None,
    }
}

#[derive(Clone, Default)]
struct MessageLog(Arc<Mutex<Vec<String>>>);

impl<S: Subscriber> Layer<S> for MessageLog {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let mut fields = MessageFields::default();
        event.record(&mut fields);
        if !fields.0.is_empty()
            && let Ok(mut messages) = self.0.lock()
        {
            messages.push(fields.0);
        }
    }
}

#[derive(Default)]
struct MessageFields(String);

impl MessageFields {
    fn push(&mut self, name: &str, rendered: &str) {
        if !self.0.is_empty() {
            self.0.push(' ');
        }
        if name == "message" {
            self.0.push_str(rendered);
        } else {
            self.0.push_str(name);
            self.0.push('=');
            self.0.push_str(rendered);
        }
    }
}

impl Visit for MessageFields {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.push(field.name(), value);
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        self.push(field.name(), &format!("{value:?}"));
    }
}

pub(crate) struct Capture {
    log: MessageLog,
    _guard: DefaultGuard,
}

impl Capture {
    pub(crate) fn start() -> Self {
        let log = MessageLog::default();
        let subscriber = registry().with(log.clone());
        Self {
            log,
            _guard: set_default(subscriber),
        }
    }

    pub(crate) fn messages(&self) -> Vec<String> {
        self.log
            .0
            .lock()
            .map_or_else(|_| Vec::new(), |messages| messages.clone())
    }
}
