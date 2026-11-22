use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Write};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_auth::agent_policy::AgentResponse;
use betterlite_auth::rejection_provider::ProviderResolution;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_config::sections::rate_limiter_settings;
use betterlite_core::api::request_destination::RequestData;
use betterlite_core::routes::CallType;
use betterlite_error::rate_limit::{ProxyRateLimitError, ProxyRateLimitErrorInit};
use betterlite_models::decisions::Json;
use betterlite_models::resolution::{ResolutionError, ResolvedModel};
use betterlite_redact::structured::Payload;
use betterlite_redis::cache_primitives::pipeline_increment::IncrementOperation;
use tokio::sync::{Mutex as AsyncMutex, Semaphore};
use tokio_util::task::TaskTracker;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt;

use crate::concurrency_gauges::ConcurrencyGauges;
use crate::concurrency_gauges::cache::{GaugeCache, GaugeCacheValue};
use crate::fail_closed::{FailClosedPolicy, RateLimitUnverifiableError};
use crate::identity_descriptors::agent_registry::AgentPolicyRegistry;
use crate::limiter_assembly::{CallTypeRateLimitError, CallTypeRateLimiter, RateLimiterAssembly};
use crate::limiter_records::{
    CacheCounterValue, LimiterClock, ModelGroupResolver, ParallelSlotAcquisition,
    RateLimitResponse, RateLimitType, RateLimitedModel, TagRateLimitResolver,
    create_rate_limit_keys,
};
use crate::pre_call_hook::{PreCallHookError, PreCallHookRequest, RateLimitPreCallHook};
use crate::rate_limit_window_store::{CounterCache, CounterValues, RedisCacheKind, WindowStore};
use crate::request_scopes::deployment_health::DeploymentHealthSource;
use crate::request_stash::{RequestStash, RequestStashSlot};
use crate::token_adjustments::store::{TokenAdjustments, UsageIncrements};
use crate::token_estimation::output_cap::CapEndpoint;
use crate::token_estimation::services::{
    GenAiTranslation, GenAiTranslationRequest, InputTokenCounter, ResponsesInput,
    ResponsesInputRequest, TokenCounterRequest, TranslatedChat,
};
use crate::windowed_admission::{SlotIdGenerator, WindowedAdmission};

pub(crate) use crate::request_scopes::tests::support::{
    FixedRegistry, MapResolver, RecordingHealth, attach, auth, data, policy,
};

pub(crate) const NOW: i64 = 1_700_000_000;
// thirty seconds before NOW, so every seeded window is still open
pub(crate) const START: i64 = 1_699_999_970;
pub(crate) const WINDOW: i64 = 60;

pub(crate) struct Clock(SystemTime);

impl Clock {
    pub(crate) fn at(seconds: i64) -> Self {
        let elapsed = Duration::from_secs(u64::try_from(seconds).expect("a non-negative clock"));
        Self(UNIX_EPOCH.checked_add(elapsed).expect("a valid clock"))
    }
}

impl LimiterClock for Clock {
    fn current_time(&self) -> SystemTime {
        self.0
    }
}

pub(crate) struct FixedSlotIds;

impl SlotIdGenerator for FixedSlotIds {
    fn next_slot_id(&self) -> String {
        String::from("slot-1")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Effect {
    Read(String),
    Write(String, CacheCounterValue),
    GaugeRead(String),
    GaugeWrite(String, GaugeCacheValue),
    Pipeline(Vec<(String, f64)>),
}

pub(crate) struct Recorder {
    values: Mutex<BTreeMap<String, CacheCounterValue>>,
    gauges: Mutex<BTreeMap<String, GaugeCacheValue>>,
    effects: Mutex<Vec<Effect>>,
    gate: Option<Arc<Semaphore>>,
}

impl Recorder {
    fn shared(parts: &Parts) -> Arc<Self> {
        Arc::new(Self {
            values: Mutex::new(parts.entries.iter().cloned().collect()),
            gauges: Mutex::new(parts.gauges.iter().cloned().collect()),
            effects: Mutex::new(Vec::new()),
            gate: parts.gate.clone(),
        })
    }

    pub(crate) fn effects(&self) -> Vec<Effect> {
        self.effects
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn gauge_writes(&self) -> Vec<(String, GaugeCacheValue)> {
        self.effects()
            .into_iter()
            .filter_map(|effect| match effect {
                Effect::GaugeWrite(key, value) => Some((key, value)),
                Effect::Read(_)
                | Effect::Write(_, _)
                | Effect::GaugeRead(_)
                | Effect::Pipeline(_) => None,
            })
            .collect()
    }

    pub(crate) fn counter(&self, key: &str) -> Option<CacheCounterValue> {
        self.values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .cloned()
    }

    fn gauge_value(&self, key: &str) -> Option<GaugeCacheValue> {
        self.gauges
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .cloned()
    }

    pub(crate) fn registry(&self, key: &str) -> Option<BTreeMap<String, f64>> {
        let gauge = self.gauge_value(key)?;
        let GaugeCacheValue::Registry(slots) = gauge else {
            return None;
        };
        Some(slots)
    }

    fn note(&self, effect: Effect) {
        self.effects
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(effect);
    }

    fn lookup(&self, key: &str) -> Option<CacheCounterValue> {
        self.note(Effect::Read(key.to_owned()));
        self.counter(key)
    }

    fn store(&self, key: &str, value: CacheCounterValue) {
        self.note(Effect::Write(key.to_owned(), value.clone()));
        self.values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), value);
    }
}

#[async_trait::async_trait]
impl CounterCache for Recorder {
    async fn get_cache(&self, key: &str, _local_only: bool) -> Option<CacheCounterValue> {
        self.lookup(key)
    }

    async fn set_cache(&self, key: &str, value: CacheCounterValue, _ttl: i64, _local_only: bool) {
        self.store(key, value);
    }

    async fn batch_get_cache(&self, _keys: &[String], _local_only: bool) -> Option<CounterValues> {
        None
    }

    fn redis_cache_kind(&self) -> Option<RedisCacheKind> {
        None
    }
}

#[async_trait::async_trait]
impl GaugeCache for Recorder {
    async fn get(&self, key: &str, _local_only: bool) -> Option<GaugeCacheValue> {
        if let Some(gate) = &self.gate {
            let _permit = gate.acquire().await.expect("the gauge gate opens");
        }
        self.note(Effect::GaugeRead(key.to_owned()));
        self.gauge_value(key)
    }

    async fn set(&self, key: &str, value: GaugeCacheValue, _ttl: Duration, _local_only: bool) {
        self.note(Effect::GaugeWrite(key.to_owned(), value.clone()));
        self.gauges
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), value);
    }

    async fn batch_get(
        &self,
        _keys: &[String],
        _local_only: bool,
    ) -> Option<Vec<Option<GaugeCacheValue>>> {
        None
    }
}

#[async_trait::async_trait]
impl UsageIncrements for Recorder {
    async fn increment(&self, operation: &IncrementOperation) {
        self.note(Effect::Pipeline(vec![(
            operation.key.clone(),
            operation.increment_value,
        )]));
    }

    async fn increment_pipeline(&self, operations: &[IncrementOperation]) {
        let increments = operations
            .iter()
            .map(|operation| (operation.key.clone(), operation.increment_value))
            .collect();
        self.note(Effect::Pipeline(increments));
    }

    async fn set_local_counter(&self, key: &str, value: f64, _ttl: Option<i64>) {
        self.note(Effect::Pipeline(vec![(key.to_owned(), value)]));
    }
}

pub(crate) struct Counter {
    tokens: Option<i64>,
}

#[async_trait::async_trait]
impl InputTokenCounter for Counter {
    async fn count_tokens(&self, _request: &TokenCounterRequest<'_>) -> Option<i64> {
        self.tokens
    }
}

pub(crate) struct NoTranslation;

impl GenAiTranslation for NoTranslation {
    fn translate_generate_content(
        &self,
        _request: &GenAiTranslationRequest<'_>,
    ) -> Option<TranslatedChat> {
        None
    }
}

impl ResponsesInput for NoTranslation {
    fn transform_responses_api_input(&self, _request: &ResponsesInputRequest<'_>) -> Vec<Json> {
        Vec::new()
    }
}

pub(crate) struct NoProviders;

impl ProviderResolution for NoProviders {
    fn resolve(&self, model: &str) -> Result<ResolvedModel, ResolutionError> {
        Err(ResolutionError::MissingProvider {
            model: model.to_owned(),
        })
    }
}

pub(crate) struct GroupMap(BTreeMap<String, String>);

impl GroupMap {
    fn from_pairs(pairs: &[(String, String)]) -> Self {
        Self(pairs.iter().cloned().collect())
    }
}

impl ModelGroupResolver for GroupMap {
    fn resolve(&self, model: &str) -> Option<String> {
        self.0.get(model).cloned()
    }
}

pub(crate) struct Parts {
    pub(crate) entries: Vec<(String, CacheCounterValue)>,
    pub(crate) gauges: Vec<(String, GaugeCacheValue)>,
    pub(crate) groups: Vec<(String, String)>,
    pub(crate) tags: Vec<(String, Option<i64>, Option<i64>)>,
    pub(crate) agents: BTreeMap<String, AgentResponse>,
    pub(crate) counted_tokens: Option<i64>,
    pub(crate) reservation_off: bool,
    pub(crate) failing_health: bool,
    pub(crate) gate: Option<Arc<Semaphore>>,
    pub(crate) batch_limiter: Option<Arc<dyn CallTypeRateLimiter>>,
}

impl Parts {
    pub(crate) fn new(
        entries: Vec<(String, CacheCounterValue)>,
        gauges: Vec<(String, GaugeCacheValue)>,
        gate: Option<Arc<Semaphore>>,
    ) -> Self {
        Self {
            entries,
            gauges,
            groups: Vec::new(),
            tags: Vec::new(),
            agents: BTreeMap::new(),
            counted_tokens: None,
            reservation_off: false,
            failing_health: false,
            gate,
            batch_limiter: None,
        }
    }
}

struct Wiring {
    settings: rate_limiter_settings::Settings,
    fail_closed: FailClosedPolicy,
    clock: Arc<dyn LimiterClock>,
    windows: Arc<WindowStore>,
    gauges: Arc<ConcurrencyGauges>,
    admission: Arc<WindowedAdmission>,
    adjustments: Arc<TokenAdjustments>,
}

struct Probes {
    model_groups: Arc<dyn ModelGroupResolver>,
    agents: Arc<dyn AgentPolicyRegistry>,
    health: Arc<dyn DeploymentHealthSource>,
    tags: Arc<dyn TagRateLimitResolver>,
    translation: Arc<dyn GenAiTranslation>,
    responses: Arc<dyn ResponsesInput>,
    input_tokens: Arc<dyn InputTokenCounter>,
    native: Arc<dyn ProviderResolution>,
    tag_handle: Arc<MapResolver>,
    health_handle: Arc<RecordingHealth>,
}

pub(crate) struct Fixture {
    pub(crate) hook: RateLimitPreCallHook,
    pub(crate) recorder: Arc<Recorder>,
    pub(crate) tags: Arc<MapResolver>,
    pub(crate) health: Arc<RecordingHealth>,
    pub(crate) tasks: Arc<TaskTracker>,
}

impl Fixture {
    pub(crate) fn new(parts: &Parts) -> Self {
        let recorder = Recorder::shared(parts);
        let wiring = wiring(parts, &recorder);
        let probes = probes(parts);
        let tags = Arc::clone(&probes.tag_handle);
        let health = Arc::clone(&probes.health_handle);
        let tasks = Arc::new(TaskTracker::new());
        let assembly = assembly(parts, wiring);
        let hook = hook(assembly, probes, Arc::clone(&tasks));
        Self {
            hook,
            recorder,
            tags,
            health,
            tasks,
        }
    }

    pub(crate) fn plain() -> Self {
        Self::new(&Parts::new(Vec::new(), Vec::new(), None))
    }

    pub(crate) async fn run(
        &self,
        slot: Option<&RequestStashSlot>,
        auth: &UserAPIKeyAuth,
        data: &mut RequestData,
        call_type: CallType,
    ) -> Result<(), PreCallHookError> {
        let fallback = RequestStashSlot::default();
        let stash = slot.unwrap_or(&fallback);
        self.hook
            .pre_call_hook(PreCallHookRequest {
                auth,
                data,
                call_type,
                endpoint: CapEndpoint::Generic,
                stash,
            })
            .await
    }

    pub(crate) async fn settle(&self) {
        self.tasks.close();
        self.tasks.wait().await;
    }
}

fn wiring(parts: &Parts, recorder: &Arc<Recorder>) -> Wiring {
    let settings = rate_limiter_settings::Settings {
        fail_closed: false,
        window_size: WINDOW,
        tpm_reservation_enabled: !parts.reservation_off,
    };
    let fail_closed = FailClosedPolicy {
        enabled: settings.fail_closed,
    };
    let clock: Arc<dyn LimiterClock> = Arc::new(Clock::at(NOW));
    let windows = Arc::new(window_store(recorder));
    let (gauges, admission) = admission(recorder, &clock, fail_closed, &windows);
    let adjustments = Arc::new(adjustments(recorder));
    Wiring {
        settings,
        fail_closed,
        clock,
        windows,
        gauges,
        admission: Arc::new(admission),
        adjustments,
    }
}

fn probes(parts: &Parts) -> Probes {
    let tag_pairs: Vec<(&str, Option<i64>, Option<i64>)> = parts
        .tags
        .iter()
        .map(|(tag, rpm, tpm)| (tag.as_str(), *rpm, *tpm))
        .collect();
    let tags = Arc::new(MapResolver::new(&tag_pairs));
    let health = Arc::new(health_source(parts.failing_health));
    let tag_clone = Arc::clone(&tags);
    let tag_handle: Arc<dyn TagRateLimitResolver> = tag_clone;
    let health_clone = Arc::clone(&health);
    let health_handle: Arc<dyn DeploymentHealthSource> = health_clone;
    Probes {
        model_groups: Arc::new(GroupMap::from_pairs(&parts.groups)),
        agents: Arc::new(FixedRegistry(parts.agents.clone())),
        health: Arc::clone(&health_handle),
        tags: tag_handle,
        translation: Arc::new(NoTranslation),
        responses: Arc::new(NoTranslation),
        input_tokens: Arc::new(Counter {
            tokens: parts.counted_tokens,
        }),
        native: Arc::new(NoProviders),
        tag_handle: tags,
        health_handle: health,
    }
}

fn assembly(parts: &Parts, wiring: Wiring) -> Arc<RateLimiterAssembly> {
    Arc::new(RateLimiterAssembly {
        settings: wiring.settings,
        clock: wiring.clock,
        fail_closed: wiring.fail_closed,
        windows: wiring.windows,
        check_and_increment_script: None,
        gauges: wiring.gauges,
        admission: wiring.admission,
        adjustments: wiring.adjustments,
        model_groups: Box::new(GroupMap::from_pairs(&parts.groups)),
        batch_limiter: Mutex::new(parts.batch_limiter.clone()),
    })
}

fn hook(
    assembly: Arc<RateLimiterAssembly>,
    probes: Probes,
    tasks: Arc<TaskTracker>,
) -> RateLimitPreCallHook {
    RateLimitPreCallHook {
        assembly,
        model_groups: probes.model_groups,
        agents: probes.agents,
        health: probes.health,
        error_threshold_per_minute: 1,
        tags: probes.tags,
        translation: probes.translation,
        responses: probes.responses,
        input_tokens: probes.input_tokens,
        native: probes.native,
        router: None,
        tasks,
    }
}

fn window_store(recorder: &Arc<Recorder>) -> WindowStore {
    WindowStore {
        cache: Arc::<Recorder>::clone(recorder),
        rate_limiter_script: None,
        counter_read_script: None,
        request_batch: None,
        check_and_increment_lock: AsyncMutex::new(()),
    }
}

fn admission(
    recorder: &Arc<Recorder>,
    clock: &Arc<dyn LimiterClock>,
    fail_closed: FailClosedPolicy,
    windows: &Arc<WindowStore>,
) -> (Arc<ConcurrencyGauges>, WindowedAdmission) {
    let gauges = Arc::new(ConcurrencyGauges {
        cache: Arc::<Recorder>::clone(recorder),
        clock: Arc::clone(clock),
        acquire_script: None,
        release_script: None,
        count_script: None,
        release_batch: None,
        fail_closed,
        check_and_increment_lock: AsyncMutex::new(()),
    });
    let admission = WindowedAdmission {
        window_size: WINDOW,
        clock: Arc::clone(clock),
        fail_closed,
        windows: Arc::clone(windows),
        refund_script: None,
        gauges: Arc::clone(&gauges),
        slot_ids: Arc::new(FixedSlotIds),
    };
    (gauges, admission)
}

fn adjustments(recorder: &Arc<Recorder>) -> TokenAdjustments {
    TokenAdjustments {
        windows: window_store(recorder),
        usage: Arc::<Recorder>::clone(recorder),
        post_call_batch: None,
        token_increment_script: None,
        guarded_token_script: None,
    }
}

fn health_source(failing: bool) -> RecordingHealth {
    if failing {
        RecordingHealth::failing()
    } else {
        RecordingHealth::healthy()
    }
}

#[derive(Debug, PartialEq)]
pub(crate) struct StashSnapshot {
    pub(crate) response: Option<RateLimitResponse>,
    pub(crate) slot: Option<ParallelSlotAcquisition>,
    pub(crate) reserved_tokens: i64,
    pub(crate) reserved_model: Option<RateLimitedModel>,
    pub(crate) reserved_scopes: BTreeSet<(String, String)>,
    pub(crate) tpm_limited_tags: BTreeSet<String>,
}

pub(crate) fn snapshot(slot: &RequestStashSlot) -> StashSnapshot {
    let stash: Arc<RequestStash> = slot.get().expect("the hook claimed the stash");
    let state = stash.state.lock().unwrap_or_else(PoisonError::into_inner);
    StashSnapshot {
        response: state.rate_limit_response.clone(),
        slot: state.parallel_slot.clone(),
        reserved_tokens: state.reserved_tokens,
        reserved_model: state.reserved_model.clone(),
        reserved_scopes: state.reserved_scopes.clone(),
        tpm_limited_tags: state.tpm_limited_tags.clone(),
    }
}

pub(crate) const fn count(value: i64) -> CacheCounterValue {
    CacheCounterValue::Int(value)
}

pub(crate) fn text(value: &str) -> CacheCounterValue {
    CacheCounterValue::Text(value.to_owned())
}

pub(crate) fn pair(key: &str, value: CacheCounterValue) -> (String, CacheCounterValue) {
    (key.to_owned(), value)
}

pub(crate) fn window_key(key: &str, value: &str) -> String {
    format!("{{{key}:{value}}}:window")
}

pub(crate) fn requests_key(key: &str, value: &str) -> String {
    create_rate_limit_keys(key, value, RateLimitType::Requests)
}

pub(crate) fn tokens_key(key: &str, value: &str) -> String {
    create_rate_limit_keys(key, value, RateLimitType::Tokens)
}

pub(crate) fn parallel_key(key: &str, value: &str) -> String {
    create_rate_limit_keys(key, value, RateLimitType::MaxParallelRequests)
}

// an open window holding `used` requests for the descriptor
pub(crate) fn request_window(
    key: &str,
    value: &str,
    used: i64,
) -> [(String, CacheCounterValue); 2] {
    [
        pair(&window_key(key, value), text(&START.to_string())),
        pair(&requests_key(key, value), count(used)),
    ]
}

// an open window holding `used` tokens for the descriptor
pub(crate) fn token_window(key: &str, value: &str, used: i64) -> [(String, CacheCounterValue); 2] {
    [
        pair(&window_key(key, value), text(&START.to_string())),
        pair(&tokens_key(key, value), count(used)),
    ]
}

pub(crate) fn entries<const N: usize>(
    windows: [[(String, CacheCounterValue); 2]; N],
) -> Vec<(String, CacheCounterValue)> {
    windows.into_iter().flatten().collect()
}

pub(crate) fn scopes(pairs: &[(&str, &str)]) -> BTreeSet<(String, String)> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

pub(crate) fn occupied_registry(slots: &[&str]) -> GaugeCacheValue {
    GaugeCacheValue::Registry(
        slots
            .iter()
            .map(|slot| ((*slot).to_owned(), 1_700_000_000.0_f64))
            .collect(),
    )
}

enum BatchFailure {
    RateLimited(ProxyRateLimitError),
    Unverifiable(RateLimitUnverifiableError),
}

pub(crate) struct BatchLimiter {
    seen: Mutex<Vec<CallType>>,
    failure: Option<BatchFailure>,
}

impl BatchLimiter {
    pub(crate) fn succeeding() -> Arc<Self> {
        Arc::new(Self {
            seen: Mutex::new(Vec::new()),
            failure: None,
        })
    }

    pub(crate) fn failing(error: ProxyRateLimitError) -> Arc<Self> {
        Arc::new(Self {
            seen: Mutex::new(Vec::new()),
            failure: Some(BatchFailure::RateLimited(error)),
        })
    }

    pub(crate) fn unverifiable(error: RateLimitUnverifiableError) -> Arc<Self> {
        Arc::new(Self {
            seen: Mutex::new(Vec::new()),
            failure: Some(BatchFailure::Unverifiable(error)),
        })
    }

    pub(crate) fn seen(&self) -> Vec<CallType> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait::async_trait]
impl CallTypeRateLimiter for BatchLimiter {
    async fn async_pre_call_hook(
        &self,
        _user_api_key_dict: &UserAPIKeyAuth,
        _data: &RequestData,
        call_type: CallType,
        _stash: &RequestStashSlot,
    ) -> Result<(), CallTypeRateLimitError> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call_type);
        match &self.failure {
            Some(BatchFailure::RateLimited(error)) => {
                Err(CallTypeRateLimitError::RateLimited(error.clone()))
            }
            Some(BatchFailure::Unverifiable(error)) => {
                Err(CallTypeRateLimitError::Unverifiable(error.clone()))
            }
            None => Ok(()),
        }
    }
}

pub(crate) fn proxy_rate_limit(message: &str) -> ProxyRateLimitError {
    ProxyRateLimitError::new(ProxyRateLimitErrorInit {
        detail: Payload::String(message.to_owned()),
        headers: None,
        category: None,
        rate_limit_type: None,
        model: None,
        llm_provider: None,
    })
    .expect("the test rate limit error is valid")
}

// a test's own tracing sink, so a debug line is asserted like any other output
#[derive(Clone, Default)]
pub(crate) struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    pub(crate) fn recording() -> (DefaultGuard, Self) {
        let captured = Self::default();
        let guard = set_default(captured.subscriber());
        (guard, captured)
    }

    fn subscriber(&self) -> impl tracing::Subscriber + Send + Sync + 'static {
        let sink = self.clone();
        fmt()
            .with_max_level(Level::DEBUG)
            .with_ansi(false)
            .with_writer(move || sink.clone())
            .finish()
    }

    pub(crate) fn text(&self) -> String {
        let bytes = self.0.lock().map(|bytes| bytes.clone()).unwrap_or_default();
        String::from_utf8(bytes).unwrap_or_default()
    }
}

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|poisoned| io::Error::other(poisoned.to_string()))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
