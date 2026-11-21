use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_auth::rejection_provider::ProviderResolution;
use betterlite_models::decisions::Json;
use betterlite_models::resolution::{ResolutionError, ResolvedModel};
use betterlite_redis::cache_primitives::pipeline_increment::IncrementOperation;
use tokio::sync::Mutex as AsyncMutex;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt;

use crate::concurrency_gauges::ConcurrencyGauges;
use crate::concurrency_gauges::cache::{GaugeCache, GaugeCacheValue};
use crate::fail_closed::FailClosedPolicy;
use crate::limiter_records::{
    CacheCounterValue, LimiterClock, ModelGroupResolver, RateLimitDescriptor,
    RateLimitDescriptorRateLimit, RateLimitType, create_rate_limit_keys,
};
use crate::model_descriptors::ModelDescriptorBuilder;
use crate::project_admission::{
    EstimationServices, ProjectAdmission, ProjectAdmissionError, ProjectIoRequest, TpmReservation,
};
use crate::provider_attribution::RateLimitErrorAttribution;
use crate::rate_limit_window_store::{CounterCache, CounterValues, RedisCacheKind, WindowStore};
use crate::request_stash::RequestStashSlot;
use crate::settlement_and_rollback::AtomicSettlement;
use crate::token_adjustments::store::{TokenAdjustments, UsageIncrements};
use crate::token_estimation::RequestData;
use crate::token_estimation::output_cap::CapEndpoint;
use crate::token_estimation::services::{
    GenAiTranslation, GenAiTranslationRequest, InputTokenCounter, ResponsesInput,
    ResponsesInputRequest, TokenCounterRequest, TranslatedChat,
};
use crate::token_reservations::TokenReservations;

pub(crate) const NOW: i64 = 1_700_000_000;
// thirty seconds before NOW, so every seeded window is still open
pub(crate) const START: i64 = 1_699_999_970;
pub(crate) const PROJECT: &str = "p1:gpt-4";
pub(crate) const GAUGE: &str = "{api_key:k1}:max_parallel_requests";

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Effect {
    Read(String),
    Write(String, CacheCounterValue, i64),
    Pipeline(Vec<(String, f64, Option<i64>)>),
}

pub(crate) struct Recorder {
    values: Mutex<BTreeMap<String, CacheCounterValue>>,
    effects: Mutex<Vec<Effect>>,
}

impl Recorder {
    pub(crate) fn effects(&self) -> Vec<Effect> {
        self.effects
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn note(&self, effect: Effect) {
        self.effects
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(effect);
    }

    fn lookup(&self, key: &str) -> Option<CacheCounterValue> {
        self.note(Effect::Read(key.to_owned()));
        self.values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .cloned()
    }

    fn store(&self, key: &str, value: CacheCounterValue, ttl: i64) {
        self.note(Effect::Write(key.to_owned(), value.clone(), ttl));
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

    async fn set_cache(&self, key: &str, value: CacheCounterValue, ttl: i64, _local_only: bool) {
        self.store(key, value, ttl);
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
        self.lookup(key).map(GaugeCacheValue::Counter)
    }

    async fn set(&self, key: &str, value: GaugeCacheValue, ttl: Duration, _local_only: bool) {
        let GaugeCacheValue::Counter(counter) = value else {
            panic!("the admission tests seed plain gauge counters");
        };
        let seconds = i64::try_from(ttl.as_secs()).expect("the gauge TTL fits");
        self.store(key, counter, seconds);
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
            operation.ttl,
        )]));
    }

    async fn increment_pipeline(&self, operations: &[IncrementOperation]) {
        let tuples = operations
            .iter()
            .map(|operation| {
                (
                    operation.key.clone(),
                    operation.increment_value,
                    operation.ttl,
                )
            })
            .collect();
        self.note(Effect::Pipeline(tuples));
    }

    async fn set_local_counter(&self, key: &str, value: f64, ttl: Option<i64>) {
        self.store(key, CacheCounterValue::Float(value), ttl.unwrap_or(-1));
    }
}

pub(crate) struct Counter {
    tokens: Option<i64>,
    models: Mutex<Vec<String>>,
}

impl Counter {
    pub(crate) fn models(&self) -> Vec<String> {
        self.models
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait::async_trait]
impl InputTokenCounter for Counter {
    async fn count_tokens(&self, request: &TokenCounterRequest<'_>) -> Option<i64> {
        self.models
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.model.to_owned());
        self.tokens
    }
}

struct NoTranslation;

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

struct Clock(SystemTime);

impl LimiterClock for Clock {
    fn current_time(&self) -> SystemTime {
        self.0
    }
}

struct NoGroups;

impl ModelGroupResolver for NoGroups {
    fn resolve(&self, _model: &str) -> Option<String> {
        None
    }
}

struct NoProviders;

impl ProviderResolution for NoProviders {
    fn resolve(&self, model: &str) -> Result<ResolvedModel, ResolutionError> {
        Err(ResolutionError::MissingProvider {
            model: model.to_owned(),
        })
    }
}

pub(crate) struct Fixture {
    pub(crate) recorder: Arc<Recorder>,
    pub(crate) counter: Counter,
    pub(crate) slot: RequestStashSlot,
    adjustments: TokenAdjustments,
    gauges: ConcurrencyGauges,
    clock: Arc<Clock>,
}

impl Fixture {
    pub(crate) fn new(
        entries: &[(String, CacheCounterValue)],
        counted_tokens: Option<i64>,
    ) -> Self {
        let values = entries.iter().cloned().collect();
        let recorder = Arc::new(Recorder {
            values: Mutex::new(values),
            effects: Mutex::new(Vec::new()),
        });
        let elapsed = Duration::from_secs(u64::try_from(NOW).expect("a positive clock"));
        let clock = Arc::new(Clock(
            UNIX_EPOCH.checked_add(elapsed).expect("a valid clock"),
        ));
        let adjustments = TokenAdjustments {
            windows: WindowStore {
                cache: Arc::<Recorder>::clone(&recorder),
                rate_limiter_script: None,
                counter_read_script: None,
                request_batch: None,
                check_and_increment_lock: AsyncMutex::new(()),
            },
            usage: Arc::<Recorder>::clone(&recorder),
            post_call_batch: None,
            token_increment_script: None,
            guarded_token_script: None,
        };
        let gauges = ConcurrencyGauges {
            cache: Arc::<Recorder>::clone(&recorder),
            clock: Arc::<Clock>::clone(&clock),
            acquire_script: None,
            release_script: None,
            count_script: None,
            release_batch: None,
            fail_closed: FailClosedPolicy { enabled: false },
            check_and_increment_lock: AsyncMutex::new(()),
        };
        Self {
            recorder,
            counter: Counter {
                tokens: counted_tokens,
                models: Mutex::new(Vec::new()),
            },
            slot: RequestStashSlot::default(),
            adjustments,
            gauges,
            clock,
        }
    }

    pub(crate) fn stash(&self) -> StashSnapshot {
        let stash = self.slot.get().expect("the admission claimed the stash");
        let state = stash.state.lock().unwrap_or_else(PoisonError::into_inner);
        StashSnapshot {
            owner: state.owner_litellm_call_id.clone(),
            itpm: (
                state.itpm_reserved_tokens,
                state.itpm_reserved_scopes.clone(),
            ),
            otpm: (
                state.otpm_reserved_tokens,
                state.otpm_reserved_scopes.clone(),
            ),
            released: state.reservation_released,
            slot_held: state.parallel_slot.is_some(),
        }
    }

    pub(crate) async fn admit(
        &self,
        request: &ProjectIoRequest<'_>,
        data: &mut RequestData,
    ) -> Result<(), ProjectAdmissionError> {
        let descriptors = ModelDescriptorBuilder {
            window_size: 60,
            resolver: Box::new(NoGroups),
        };
        let admission = ProjectAdmission {
            reservations: TokenReservations {
                settlement: AtomicSettlement {
                    windows: &self.adjustments.windows,
                    clock: self.clock.as_ref(),
                    script: None,
                    refund_script: None,
                    fail_closed: FailClosedPolicy { enabled: false },
                    default_window_size: 60,
                },
                adjustments: &self.adjustments,
                descriptors: &descriptors,
                attribution: RateLimitErrorAttribution {
                    requested_model: None,
                    native: &NoProviders,
                    router: None,
                },
            },
            gauges: &self.gauges,
            estimation: EstimationServices {
                translation: &NoTranslation,
                responses: &NoTranslation,
                counter: &self.counter,
            },
        };
        admission
            .reserve_project_io_tokens(request, data, &self.slot)
            .await
    }
}

#[derive(Debug, PartialEq)]
pub(crate) struct StashSnapshot {
    pub(crate) owner: Option<String>,
    pub(crate) itpm: (i64, BTreeSet<(String, String)>),
    pub(crate) otpm: (i64, BTreeSet<(String, String)>),
    pub(crate) released: bool,
    pub(crate) slot_held: bool,
}

pub(crate) fn request<'request>(
    descriptors: &'request [RateLimitDescriptor],
    tpm_scopes: &'request [(String, String)],
    tpm_amount: i64,
) -> ProjectIoRequest<'request> {
    ProjectIoRequest {
        descriptors,
        requested_model: Some("gpt-4"),
        call_type: Some("acompletion"),
        endpoint: CapEndpoint::Generic,
        tpm_reservation: TpmReservation {
            scopes: tpm_scopes,
            amount: tpm_amount,
        },
    }
}

pub(crate) fn data(text: &str) -> RequestData {
    serde_json::from_str(text).expect("a request body")
}

pub(crate) fn limit(key: &str, value: &str, tokens: i64) -> RateLimitDescriptor {
    RateLimitDescriptor {
        key: key.to_owned(),
        value: value.to_owned(),
        rate_limit: Some(RateLimitDescriptorRateLimit {
            requests_per_unit: None,
            tokens_per_unit: Some(tokens),
            max_parallel_requests: None,
            window_size: Some(60),
        }),
    }
}

pub(crate) fn tokens_key(key: &str, value: &str) -> String {
    create_rate_limit_keys(key, value, RateLimitType::Tokens)
}

// an open window holding `used` tokens for the descriptor
pub(crate) fn window(key: &str, value: &str, used: i64) -> [(String, CacheCounterValue); 2] {
    [
        (
            format!("{{{key}:{value}}}:window"),
            CacheCounterValue::Text(START.to_string()),
        ),
        (tokens_key(key, value), CacheCounterValue::Int(used)),
    ]
}

pub(crate) fn scopes(pairs: &[(&str, &str)]) -> BTreeSet<(String, String)> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

pub(crate) struct LogCapture {
    buffer: Arc<Mutex<Vec<u8>>>,
    _guard: DefaultGuard,
}

struct Sink(Arc<Mutex<Vec<u8>>>);

impl io::Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl LogCapture {
    pub(crate) fn start() -> Self {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&buffer);
        let subscriber = fmt()
            .without_time()
            .with_ansi(false)
            .with_max_level(Level::DEBUG)
            .with_writer(move || Sink(Arc::clone(&sink)))
            .finish();
        Self {
            buffer,
            _guard: set_default(subscriber),
        }
    }

    pub(crate) fn text(&self) -> String {
        let bytes = self
            .buffer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        String::from_utf8(bytes).expect("the log is UTF-8")
    }
}
