#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::backend::{CacheResult, Ttl};
use betterlite_cache::cache_key::derive::get_cache_key;
use betterlite_cache::cache_key::settings::CacheKeySettings;
use betterlite_cache::cache_key::value::{KeyObject, KeyValue};
use betterlite_cache::dual_cache::DualCacheSettings;
use betterlite_cache::envelope::backend::EnvelopeBackend;
use betterlite_cache::envelope::facade::{Clock, EnvelopeSettings, ResponseCache};
use betterlite_cache::envelope::lookup::CacheLookup as EnvelopeLookup;
use betterlite_cache::handler::embedding::{embedding_base_request, embedding_entry_key};
use betterlite_cache::handler::kwargs::key_request;
use betterlite_cache::handler::retrieve::{AsyncSupport, CacheSource};
use betterlite_cache::handler::state::{CachingHandler, HandlerDependencies, OriginalFunction};
use betterlite_cache::replay::{
    CacheHitMetadata, CachedDict, ReplayConversions, ReplayError, Replayed, ReplayedBy,
    ResponseKind, TranscriptionHidden,
};
use betterlite_cache::types::cache_type::{CacheMode, LiteLlmCacheType};
use betterlite_cache::types::call_type::CachingSupportedCallType;
use betterlite_cache::types::semantic::SemanticCacheScope;
use betterlite_config::sections::logging_privacy::Settings;
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_logging::callback_logger::CallbackLogger;
use betterlite_logging::callback_logger::error::LoggerError;
use betterlite_logging::callback_logger::event::LoggerEvent;
use betterlite_logging::exporter_options::failure_counter::CallbackFailureCounter;
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_logging::logging_success::owned::OwnedSuccessHandler;
use betterlite_logging::logging_success::ports::{
    StandardPayloadBuilder, StreamAssembler, SuccessClock,
};
use betterlite_logging::logging_success::types::{SuccessMode, SuccessResult};
use betterlite_logging::registry::identity::CallbackIdentity;
use betterlite_logging::registry::phases::CallbackList;
use betterlite_logging::registry::sink::CallbackSink;
use betterlite_logging::registry::{CallbackRegistry, Registration};
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_logging::worker::{LoggingWorker, WorkerSettings};
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_redact::structured::Payload;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use tokio::task::yield_now;

use crate::caching::hit_callbacks::context::CACHING_DETAILS;
use crate::caching::hit_callbacks::job::CacheHitSuccessJob;
use crate::caching::lookup::async_lookup::{AsyncLookup, async_get_cache};
use crate::caching::lookup::error::TokenCountError;
use crate::caching::lookup::hit::{HitLogging, HitResponse};
use crate::caching::lookup::json::cache_value_json;
use crate::caching::lookup::request::CacheLookup;
use crate::caching::lookup::response::LookupResponse;
use crate::caching::lookup::services::{LookupServices, PerfClock, ResponseTokens};
use crate::caching::lookup::sync_lookup::sync_get_cache;

pub(super) const MODEL: &str = "gpt-4o";
pub(super) const EMBEDDING_MODEL: &str = "text-embedding-3-small";
const CATALOG: &str = r#"{
    "gpt-4o": {"litellm_provider": "openai", "mode": "chat"},
    "text-embedding-3-small": {"litellm_provider": "openai", "mode": "embedding"}
}"#;
const PAYLOAD: &str = r#"{
    "id": "chatcmpl-1",
    "trace_id": "trace-1",
    "session_id": "session-1",
    "call_type": "completion",
    "response_cost": 0.5,
    "status": "success",
    "status_fields": {"llm_api_status": "success", "guardrail_status": "not_run"},
    "total_tokens": 5,
    "prompt_tokens": 2,
    "completion_tokens": 3,
    "startTime": 1704067200.0,
    "endTime": 1704067201.5,
    "completionStartTime": 1704067200.5,
    "response_time": 1.0,
    "model_map_information": {"model_map_key": "", "model_map_value": null},
    "model": "gpt-4o",
    "api_base": "https://api.openai.com",
    "metadata": {},
    "saved_cache_cost": 0.0,
    "request_tags": [],
    "model_parameters": {},
    "hidden_params": {},
    "messages": [{"role": "user", "content": "hi"}],
    "response": {"id": "chatcmpl-1", "object": "chat.completion", "choices": []}
}"#;

pub(super) const SUPPORTED: [CachingSupportedCallType; 14] = [
    CachingSupportedCallType::Completion,
    CachingSupportedCallType::Acompletion,
    CachingSupportedCallType::Embedding,
    CachingSupportedCallType::Aembedding,
    CachingSupportedCallType::Transcription,
    CachingSupportedCallType::Atranscription,
    CachingSupportedCallType::TextCompletion,
    CachingSupportedCallType::AtextCompletion,
    CachingSupportedCallType::Rerank,
    CachingSupportedCallType::Arerank,
    CachingSupportedCallType::Responses,
    CachingSupportedCallType::Aresponses,
    CachingSupportedCallType::AnthropicMessages,
    CachingSupportedCallType::AanthropicMessages,
];

pub(super) fn time(value: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(value).expect("the fixture time parses")
}

pub(super) fn text(value: &str) -> KeyValue {
    KeyValue::String(value.to_owned())
}

pub(super) fn kwargs(entries: &[(&str, KeyValue)]) -> KeyObject {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

pub(super) fn object(entries: &[(&str, KeyValue)]) -> KeyValue {
    KeyValue::Object(kwargs(entries))
}

pub(super) fn chat_kwargs() -> KeyObject {
    kwargs(&[
        ("model", text(MODEL)),
        ("messages", KeyValue::Array(Vec::new())),
    ])
}

pub(super) fn embedding_kwargs(inputs: &[&str]) -> KeyObject {
    let mut request = kwargs(&[("model", text(EMBEDDING_MODEL))]);
    let values = inputs.iter().map(|value| text(value)).collect();
    request.insert("input".to_owned(), KeyValue::Array(values));
    request
}

pub(super) fn cached_chat() -> CacheValue {
    CacheValue::Object(BTreeMap::from([
        ("id".to_owned(), CacheValue::Text("chatcmpl-1".to_owned())),
        (
            "object".to_owned(),
            CacheValue::Text("chat.completion".to_owned()),
        ),
        ("choices".to_owned(), CacheValue::Array(Vec::new())),
    ]))
}

pub(super) fn cached_detail() -> CacheValue {
    let mut fields = BTreeMap::new();
    fields.insert(
        "detail".to_owned(),
        CacheValue::Text("the deployment is gone".to_owned()),
    );
    fields.insert("code".to_owned(), CacheValue::Integer(404));
    CacheValue::Object(fields)
}

pub(super) fn stored_embedding(
    vector: &[f64],
    prompt_tokens: Option<i64>,
    format_version: i64,
) -> CacheValue {
    let fields = BTreeMap::from([
        (
            "embedding".to_owned(),
            CacheValue::Array(
                vector
                    .iter()
                    .map(|value| CacheValue::Float(*value))
                    .collect(),
            ),
        ),
        ("index".to_owned(), CacheValue::Integer(0)),
        (
            "object".to_owned(),
            CacheValue::Text("embedding".to_owned()),
        ),
        (
            "model".to_owned(),
            CacheValue::Text(EMBEDDING_MODEL.to_owned()),
        ),
        (
            "prompt_tokens".to_owned(),
            prompt_tokens.map_or(CacheValue::Null, CacheValue::Integer),
        ),
        ("prompt_tokens_details".to_owned(), CacheValue::Null),
        (
            "format_version".to_owned(),
            CacheValue::Integer(format_version),
        ),
    ]);
    CacheValue::Object(fields)
}

pub(super) struct SilentLogger;

impl LoggerSink for SilentLogger {
    fn debug(&self, _payload: &Payload) -> bool {
        false
    }

    fn info(&self, _payload: &Payload) -> bool {
        false
    }

    fn error(&self, _payload: &Payload) -> bool {
        false
    }

    fn debug_enabled(&self) -> bool {
        false
    }
}

pub(super) struct SilentConsole;

impl ConsoleSink for SilentConsole {
    fn print(&self, _payload: &Payload) -> bool {
        false
    }
}

pub(super) type TestCache = ResponseCache<SilentLogger, SilentConsole>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ReadKind {
    Sync,
    Async,
}

#[derive(Default)]
pub(super) struct RecordingBackend {
    stored: Mutex<BTreeMap<String, CacheValue>>,
    reads: Mutex<Vec<ReadKind>>,
}

impl RecordingBackend {
    pub(super) fn seed(&self, key: &str, value: CacheValue) {
        self.stored
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), value);
    }

    pub(super) fn reads(&self) -> Vec<ReadKind> {
        self.reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn read(&self, key: &str, kind: ReadKind) -> Option<CacheValue> {
        self.reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(kind);
        self.stored
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .cloned()
    }
}

#[async_trait]
impl EnvelopeBackend for RecordingBackend {
    fn get_cache(
        &self,
        key: &str,
        _lookup: &mut EnvelopeLookup,
    ) -> CacheResult<Option<CacheValue>> {
        Ok(self.read(key, ReadKind::Sync))
    }

    fn set_cache(&self, key: &str, value: CacheValue, _ttl: Ttl) -> CacheResult<()> {
        self.seed(key, value);
        Ok(())
    }

    async fn async_get_cache(
        &self,
        key: &str,
        _lookup: &mut EnvelopeLookup,
    ) -> CacheResult<Option<CacheValue>> {
        yield_now().await;
        Ok(self.read(key, ReadKind::Async))
    }

    async fn async_set_cache(&self, key: &str, value: CacheValue, ttl: Ttl) -> CacheResult<()> {
        yield_now().await;
        self.set_cache(key, value, ttl)
    }

    async fn batch_cache_write(&self, key: &str, value: CacheValue, ttl: Ttl) -> CacheResult<()> {
        self.async_set_cache(key, value, ttl).await
    }
}

fn hooks() -> VerboseHooks<SilentLogger, SilentConsole> {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds");
    VerboseHooks {
        logger: SilentLogger,
        console: SilentConsole,
        set_verbose: Payload::Bool(false),
        redactor: Arc::new(redactor),
    }
}

fn key_settings() -> CacheKeySettings {
    CacheKeySettings {
        cache_type: LiteLlmCacheType::Local,
        mode: CacheMode::DefaultOn,
        namespace: None,
        semantic_cache_scope: SemanticCacheScope::Key,
        include_provider_params: false,
    }
}

fn fixture_clock() -> Clock {
    Arc::new(|| 1_700_000_000.0_f64)
}

fn cache(backend: Arc<RecordingBackend>) -> TestCache {
    ResponseCache {
        backend,
        key_settings: key_settings(),
        settings: EnvelopeSettings {
            ttl: None,
            redis_flush_size: None,
        },
        hooks: hooks(),
        clock: fixture_clock(),
    }
}

pub(super) fn computed_key(cache: &TestCache, kwargs: &KeyObject) -> String {
    let mut request = key_request(kwargs).expect("the request derives a key");
    get_cache_key(&mut request, &cache.key_settings, &cache.hooks).expect("the key derives")
}

pub(super) fn embedding_key(cache: &TestCache, kwargs: &KeyObject, input: &str) -> String {
    let base = embedding_base_request(kwargs).expect("the embedding request derives a key");
    embedding_entry_key(cache, &base, &text(input)).expect("the entry key derives")
}

pub(super) struct ResolutionFixture {
    catalog: ModelCatalog,
    known: BTreeSet<String>,
    custom: BTreeSet<String>,
    environment: BTreeMap<String, String>,
}

impl ResolutionFixture {
    pub(super) fn context(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.catalog,
            known_model_names: &self.known,
            custom_providers: &self.custom,
            environment: &self.environment,
            registry: JSONProviderRegistry::default(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }
}

pub(super) fn handler_named(name: &str, parameters: &[&str], kwargs: KeyObject) -> CachingHandler {
    CachingHandler::new(
        HandlerDependencies {
            original_function: OriginalFunction {
                name: name.to_owned(),
                parameters: parameters.iter().copied().map(str::to_owned).collect(),
            },
            start_time: 0.0,
            redis_cache: None,
            in_memory_cache: Arc::new(
                LocalMemoryOps::new(1024, fixture_clock()).expect("the memory tier builds"),
            ),
            dual_settings: DualCacheSettings {
                default_in_memory_ttl: None,
                default_redis_ttl: None,
                batch_cache_expiry: None,
                max_redis_batch_cache_size: 1000,
                redis_delete_chunk_size: 1000,
            },
        },
        kwargs,
    )
}

pub(super) struct FixedClock {
    pub(super) now: DateTime<FixedOffset>,
}

impl SuccessClock for FixedClock {
    fn now(&self) -> DateTime<FixedOffset> {
        self.now
    }
}

pub(super) fn wall() -> Arc<FixedClock> {
    Arc::new(FixedClock {
        now: time("2024-01-01T00:00:02+00:00"),
    })
}

pub(super) fn perf_clock(times: &[Duration]) -> PerfClock {
    let scripted = Arc::new(Mutex::new(times.to_vec()));
    Arc::new(move || {
        let mut remaining = scripted.lock().unwrap_or_else(PoisonError::into_inner);
        if remaining.is_empty() {
            return Duration::ZERO;
        }
        remaining.remove(0)
    })
}

#[derive(Default)]
pub(super) struct Tokens {
    pub(super) calls: Mutex<Vec<String>>,
    pub(super) count: i64,
}

impl Tokens {
    pub(super) const fn counting(count: i64) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            count,
        }
    }

    pub(super) fn counted(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl ResponseTokens for Tokens {
    async fn count_response_tokens(&self, text: &str) -> Result<i64, TokenCountError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(text.to_owned());
        Ok(self.count)
    }
}

#[derive(Default)]
pub(super) struct Calls(Mutex<Vec<&'static str>>);

impl Calls {
    pub(super) fn record(&self, event: &'static str) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
    }

    pub(super) fn events(&self) -> Vec<&'static str> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

pub(super) struct FakeLogger {
    pub(super) calls: Arc<Calls>,
    pub(super) counter: Option<Arc<RecordingCounter>>,
}

#[async_trait]
impl CallbackLogger for FakeLogger {
    fn callback_name(&self) -> &'static str {
        "langfuse"
    }

    fn failure_counter(&self) -> Option<&dyn CallbackFailureCounter> {
        let counter = self.counter.as_deref()?;
        Some(counter)
    }

    fn log_success_event(&self, _event: &LoggerEvent) -> Result<(), LoggerError> {
        self.calls.record("sync");
        Ok(())
    }

    async fn async_log_success_event(&self, _event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.calls.record("async");
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Snapshot {
    pub(super) model: String,
    pub(super) custom_llm_provider: Option<String>,
    pub(super) litellm_params: IndexMap<String, Json>,
    pub(super) caching_details: Json,
    pub(super) original_response: Json,
    pub(super) logged_response: Json,
}

pub(super) struct FakeBuilder {
    pub(super) payload: Option<StandardLoggingPayload>,
    pub(super) calls: Arc<Calls>,
    pub(super) snapshots: Arc<Mutex<Vec<Snapshot>>>,
}

impl StandardPayloadBuilder for FakeBuilder {
    fn build(
        &self,
        state: &LoggingState,
        response: &Json,
        _start_time: DateTime<FixedOffset>,
        _end_time: DateTime<FixedOffset>,
    ) -> Option<StandardLoggingPayload> {
        let recorded = state.model_call_details.contains_key(CACHING_DETAILS);
        let marked = state.model_call_details.get("cache_hit") == Some(&Json::Bool(true));
        if recorded && marked {
            self.calls.record("build-with-cache-hit");
        } else {
            self.calls.record("build-without-cache-hit");
        }
        let snapshot = Snapshot {
            model: state.model.clone(),
            custom_llm_provider: state.custom_llm_provider.clone(),
            litellm_params: state.litellm_params.clone(),
            caching_details: state
                .model_call_details
                .get(CACHING_DETAILS)
                .cloned()
                .unwrap_or(Json::Null),
            original_response: state
                .model_call_details
                .get("original_response")
                .cloned()
                .unwrap_or(Json::Null),
            logged_response: response.clone(),
        };
        self.snapshots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(snapshot);
        self.payload.clone()
    }
}

pub(super) struct FakeAssembler;

impl StreamAssembler for FakeAssembler {
    fn assemble(
        &self,
        _state: &LoggingState,
        _result: &SuccessResult,
        _mode: SuccessMode,
    ) -> Option<Json> {
        None
    }
}

#[derive(Default)]
pub(super) struct RecordingCounter(Mutex<Vec<String>>);

impl CallbackFailureCounter for RecordingCounter {
    fn increment_callback_logging_failure(&self, callback_name: &str) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(callback_name.to_owned());
    }
}

pub(super) fn logging_state() -> LoggingState {
    let mut context = CorrelationContext::default();
    LoggingState::new(
        LoggingInit {
            model: MODEL.to_owned(),
            call_type: "completion".to_owned(),
            start_time: time("2024-01-01T00:00:00+00:00"),
            litellm_call_id: "call-1".to_owned(),
            function_id: "function-1".to_owned(),
            messages: None,
            stream: None,
            litellm_trace_id: Some("trace-1".to_owned()),
            kwargs: IndexMap::new(),
            litellm_params: IndexMap::new(),
            callback_dynamic_params: IndexMap::new(),
            trusted_callback_vars: Vec::new(),
            standard_built_in_tools_params: IndexMap::new(),
            dynamic_callbacks: DynamicCallbacks::default(),
            applied_guardrails: None,
            request_logging: RequestLoggingFlags::default(),
            callback_policy: CallbackPolicy::default(),
        },
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        &Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds"),
        || "generated-trace".to_owned(),
        || Duration::ZERO,
    )
    .expect("the logging state builds")
}

pub(super) fn worker_settings(max_queue_size: usize, concurrency: usize) -> WorkerSettings {
    WorkerSettings {
        max_queue_size,
        concurrency,
        event_timeout: Duration::from_secs(5),
        shutdown_drain: Duration::from_secs(5),
        timeout_summary_window: Duration::from_secs(1),
    }
}

pub(super) fn worker() -> LoggingWorker<CacheHitSuccessJob> {
    LoggingWorker::start(worker_settings(8, 2)).expect("the worker starts")
}

pub(super) struct Fixture {
    pub(super) backend: Arc<RecordingBackend>,
    pub(super) cache: TestCache,
    pub(super) resolution: ResolutionFixture,
    pub(super) calls: Arc<Calls>,
    pub(super) snapshots: Arc<Mutex<Vec<Snapshot>>>,
    pub(super) counter: Arc<RecordingCounter>,
    pub(super) conversions: FakeConversions,
    pub(super) cache_type: LiteLlmCacheType,
}

impl Fixture {
    pub(super) fn new() -> Self {
        let backend = Arc::new(RecordingBackend::default());
        let cache = cache(Arc::clone(&backend));
        Self {
            backend,
            cache,
            resolution: ResolutionFixture {
                catalog: ModelCatalog::parse(CATALOG.as_bytes(), UnknownFields::Reject)
                    .expect("the catalog parses"),
                known: BTreeSet::from([MODEL.to_owned(), EMBEDDING_MODEL.to_owned()]),
                custom: BTreeSet::new(),
                environment: BTreeMap::new(),
            },
            calls: Arc::new(Calls::default()),
            snapshots: Arc::new(Mutex::new(Vec::new())),
            counter: Arc::new(RecordingCounter::default()),
            conversions: FakeConversions::default(),
            cache_type: LiteLlmCacheType::Local,
        }
    }

    pub(super) fn source(&self) -> CacheSource<'_, SilentLogger, SilentConsole> {
        CacheSource {
            cache: &self.cache,
            async_support: AsyncSupport::Supported,
        }
    }

    pub(super) fn owned_handler(&self) -> Arc<OwnedSuccessHandler> {
        let logger = Arc::new(FakeLogger {
            calls: Arc::clone(&self.calls),
            counter: Some(Arc::clone(&self.counter)),
        });
        let mut registry = CallbackRegistry::new(10);
        let identity = CallbackIdentity {
            class_name: "langfuse".to_owned(),
            attributes: IndexMap::new(),
        };
        let handle: Arc<dyn CallbackLogger> = logger;
        let sink = CallbackSink::Logger(handle);
        let registration = registry.register(identity, sink, CallbackList::Callbacks);
        assert_ne!(registration, Registration::SkippedCapacity);
        Arc::new(OwnedSuccessHandler {
            registry: Arc::new(registry),
            payload_builder: Arc::new(FakeBuilder {
                payload: Some(serde_json::from_str(PAYLOAD).expect("the payload fixture decodes")),
                calls: Arc::clone(&self.calls),
                snapshots: Arc::clone(&self.snapshots),
            }),
            stream_assembler: Arc::new(FakeAssembler),
            clock: Arc::new(FixedClock {
                now: time("2024-01-01T00:00:02+00:00"),
            }),
            excluded_fields: Vec::new(),
            known_callback: Arc::new(|_name: &str| false),
            privacy: Settings::default(),
        })
    }

    pub(super) fn hit_logging(&self) -> HitLogging {
        HitLogging {
            handler: self.owned_handler(),
            state: logging_state(),
            correlation: CorrelationContext::default(),
        }
    }

    pub(super) fn snapshots(&self) -> Vec<Snapshot> {
        self.snapshots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

// A converted response records what the hit path asked of it, so the tests read the stamp and the logged result
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) enum Stamp {
    #[default]
    Unset,
    Cleared,
    Key(String),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Model {
    pub(super) cache_hit: bool,
    pub(super) stamp: Stamp,
}

impl CacheHitMetadata for Model {
    fn set_cache_hit(&mut self) {
        self.cache_hit = true;
    }
}

impl HitResponse for Model {
    fn stamp_cache_key(&mut self, cache_key: Option<&str>) {
        self.stamp = cache_key.map_or(Stamp::Cleared, |key| Stamp::Key(key.to_owned()));
    }

    fn success_result(&self) -> SuccessResult {
        SuccessResult::Chat(Json::Bool(self.cache_hit))
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Recorded {
    pub(super) kinds: Vec<ResponseKind>,
    pub(super) wraps: Vec<Wrap>,
    pub(super) metadata: Vec<Metadata>,
    pub(super) messages_probes: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Wrap {
    pub(super) asynchronous: bool,
    pub(super) model: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Metadata {
    pub(super) model: String,
    pub(super) elapsed: Duration,
}

#[derive(Default)]
pub(super) struct FakeConversions {
    recorded: Arc<Mutex<Recorded>>,
}

impl FakeConversions {
    pub(super) fn recorded(&self) -> Recorded {
        self.recorded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record<Change>(&self, change: Change)
    where
        Change: FnOnce(&mut Recorded),
    {
        let mut recorded = self.recorded.lock().unwrap_or_else(PoisonError::into_inner);
        change(&mut recorded);
    }
}

impl ReplayConversions for FakeConversions {
    type Response = Model;

    fn model_response(
        &self,
        _cached: &CachedDict,
        kind: ResponseKind,
    ) -> Result<Self::Response, ReplayError> {
        self.record(|recorded| recorded.kinds.push(kind));
        Ok(Model::default())
    }

    fn transcription_response(
        &self,
        _cached: &CachedDict,
        _hidden: &TranscriptionHidden<'_>,
    ) -> Result<Self::Response, ReplayError> {
        Ok(Model::default())
    }

    fn responses_response(&self, _cached: &CachedDict) -> Result<Self::Response, ReplayError> {
        Ok(Model::default())
    }

    fn responses_stream(
        &self,
        response: Self::Response,
        _call_type: CachingSupportedCallType,
    ) -> Result<Self::Response, ReplayError> {
        Ok(response)
    }

    fn anthropic_messages_stream(
        &self,
        _cached: &CachedDict,
    ) -> Result<Option<Self::Response>, ReplayError> {
        self.record(|recorded| {
            recorded.messages_probes = recorded.messages_probes.saturating_add(1);
        });
        Ok(None)
    }

    fn stream_wrapper(
        &self,
        _cached: &CachedDict,
        asynchronous: bool,
        model: &str,
        _custom_llm_provider: &str,
    ) -> Result<Self::Response, ReplayError> {
        self.record(|recorded| {
            recorded.wraps.push(Wrap {
                asynchronous,
                model: model.to_owned(),
            });
        });
        Ok(Model::default())
    }

    fn update_response_metadata(
        &self,
        _result: &mut ReplayedBy<Self>,
        model: &str,
        elapsed: Duration,
    ) {
        self.record(|recorded| {
            recorded.metadata.push(Metadata {
                model: model.to_owned(),
                elapsed,
            });
        });
    }
}

pub(super) fn services<'svc>(
    cache: Option<&'svc CacheSource<'svc, SilentLogger, SilentConsole>>,
    resolution: &'svc ResolutionContext<'svc>,
    worker: &'svc LoggingWorker<CacheHitSuccessJob>,
    tokens: &'svc dyn ResponseTokens,
    perf: PerfClock,
) -> LookupServices<'svc, SilentLogger, SilentConsole> {
    LookupServices {
        cache,
        cache_type: LiteLlmCacheType::Local,
        supported_call_types: Some(&SUPPORTED),
        resolution,
        worker,
        tokens,
        perf,
        wall: wall(),
    }
}

// Runs the async lookup with the fixture's services, so a test keeps only its own seeds and assertions
pub(super) async fn async_cache_lookup(
    fixture: &Fixture,
    handler: &mut CachingHandler,
    request: &CacheLookup,
    tokens: &Tokens,
    perf: PerfClock,
) -> (
    AsyncLookup<Replayed<Model>>,
    LoggingWorker<CacheHitSuccessJob>,
) {
    let source = fixture.source();
    let resolution = fixture.resolution.context();
    let worker = worker();
    let mut services = services(Some(&source), &resolution, &worker, tokens, perf);
    services.cache_type = fixture.cache_type;
    let outcome = async_get_cache(
        handler,
        request,
        fixture.hit_logging(),
        &services,
        &fixture.conversions,
    )
    .await
    .expect("the async lookup runs");
    (outcome, worker)
}

// Runs the sync lookup the same way, handing the caller back the logging state it wrote into
pub(super) async fn sync_cache_lookup(
    fixture: &Fixture,
    handler: &mut CachingHandler,
    request: &CacheLookup,
    perf: PerfClock,
    logging: &mut HitLogging,
) -> LookupResponse<Replayed<Model>> {
    let source = fixture.source();
    let resolution = fixture.resolution.context();
    let tokens = Tokens::default();
    let worker = worker();
    let mut services = services(Some(&source), &resolution, &worker, &tokens, perf);
    services.cache_type = fixture.cache_type;
    sync_get_cache(handler, request, logging, &services, &fixture.conversions)
        .await
        .expect("the sync lookup runs")
}

pub(super) fn lookup(call_type: CachingSupportedCallType, kwargs: KeyObject) -> CacheLookup {
    CacheLookup {
        model: MODEL.to_owned(),
        call_type,
        start_time: time("2024-01-01T00:00:00+00:00"),
        kwargs,
        args: Vec::new(),
        converted_stream_requested: false,
        agentic_loop_depth: 0,
    }
}

// The same lookup for an embedding call, whose request names the embedding model
pub(super) fn embedding_lookup(kwargs: KeyObject) -> CacheLookup {
    let mut request = lookup(CachingSupportedCallType::Aembedding, kwargs);
    request.model = EMBEDDING_MODEL.to_owned();
    request
}

pub(super) fn cached_json(value: &CacheValue) -> Json {
    cache_value_json(value)
}

pub(super) fn details_fields(details: &Json) -> &IndexMap<String, Json> {
    let Json::Object(fields) = details else {
        panic!("the caching details are an object");
    };
    fields
}

pub(super) fn hit_field(details: &Json) -> Option<bool> {
    match details_fields(details).get("cache_hit") {
        Some(Json::Bool(flag)) => Some(*flag),
        None | Some(_) => None,
    }
}

pub(super) fn duration_field(details: &Json) -> Option<f64> {
    match details_fields(details).get("cache_duration_ms") {
        Some(Json::Number(number)) => number.as_f64(),
        None | Some(_) => None,
    }
}
