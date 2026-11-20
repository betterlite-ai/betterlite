use std::collections::{BTreeMap, VecDeque};
use std::mem::take;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_core::routes::CallType;
use betterlite_core::rules::{RuleDecision, RuleEvaluator};
use betterlite_core::schema::SchemaLimits;
use betterlite_error::error::details::{ApiDetails, RateLimitDetails, ResponseDetails};
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_error::rate_limit::RateLimitCategory;
use betterlite_logging::callback_logger::CallbackLogger;
use betterlite_logging::callback_logger::resolver::LoggerResolver;
use betterlite_logging::logging_state::correlation::CorrelationSettings;
use betterlite_logging::logging_success::types::SuccessResult;
use betterlite_logging::registry::error::RegistryError;
use betterlite_logging::worker::Admission;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset, TimeDelta};
use indexmap::IndexMap;
use tokio::runtime::Runtime;

use crate::call_setup::RequestMap;
use crate::call_setup::callbacks::CallbackLists;
use crate::call_setup::correlation::{CallOutcome, ChatStream};
use crate::call_setup::credentials::{Credential, CredentialLookup};
use crate::call_setup::hooks::{DeploymentHook, HookError};
use crate::call_setup::providers::ProviderRegistry;
use crate::call_wrapper::budget::BudgetState;
use crate::call_wrapper::cache::{CacheHandler, CacheMode, CacheOutcome};
use crate::call_wrapper::error::{CallError, plain_error};
use crate::call_wrapper::logging::{AsyncSuccessLog, CallLogging, SuccessLogScheduler};
use crate::call_wrapper::max_tokens::MaxTokensModifier;
use crate::call_wrapper::metadata::{MetadataUpdate, ResponseMetadataUpdater};
use crate::call_wrapper::ports::{CallTimes, IdSource, WrapperClock};
use crate::call_wrapper::retry::WrapperRetryDefaults;
use crate::call_wrapper::state::{KnownCallback, LoggingHandle, WrapperOptions};
use crate::call_wrapper::stream::StreamAssembly;
use crate::call_wrapper::target::CallTarget;
use crate::call_wrapper::{CallWrapper, SetupState, WrapperFlags};

pub(crate) fn moment() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00").expect("the fixture is RFC 3339")
}

pub(crate) fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor compiles")
}

pub(crate) fn request(source: &str) -> RequestMap {
    serde_json::from_str(source).expect("the fixture request is a JSON object")
}

pub(crate) fn value(source: &str) -> Json {
    serde_json::from_str(source).expect("the fixture value is JSON")
}

pub(crate) fn chat_request(extra: &str) -> RequestMap {
    let mut base = request(r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}"#);
    base.extend(request(extra));
    base
}

pub(crate) fn context(message: &str) -> ErrorContext {
    ErrorContext {
        message: message.to_owned(),
        model: Some(String::from("gpt-4o")),
        provider: Some(String::from("openai")),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    }
}

pub(crate) fn api_error(message: &str) -> CallError {
    CallError::from(ErrorClass::Api(Box::new(ApiDetails {
        context: context(message),
        status: 500,
    })))
}

pub(crate) fn bad_request(message: &str) -> CallError {
    CallError::from(ErrorClass::BadRequest(Box::new(ResponseDetails::from(
        context(message),
    ))))
}

pub(crate) fn not_found(message: &str) -> CallError {
    CallError::from(ErrorClass::NotFound(Box::new(ResponseDetails::from(
        context(message),
    ))))
}

pub(crate) fn context_window(message: &str) -> CallError {
    CallError::from(ErrorClass::ContextWindowExceeded(Box::new(
        ResponseDetails::from(context(message)),
    )))
}

pub(crate) fn rate_limit(message: &str) -> CallError {
    rate_limited(message, RateLimitCategory::VendorRateLimit)
}

fn rate_limited(message: &str, category: RateLimitCategory) -> CallError {
    CallError::from(ErrorClass::RateLimit(Box::new(RateLimitDetails {
        context: context(message),
        category,
        dimension: None,
        headers: BTreeMap::new(),
    })))
}

pub(crate) fn plain(message: &str) -> CallError {
    CallError::from(plain_error(message))
}

pub(crate) fn response(body: Json) -> CallOutcome {
    CallOutcome::Response(body)
}

pub(crate) fn chat_stream(body: Json, converted_from_mock: bool) -> CallOutcome {
    CallOutcome::ChatStream(ChatStream {
        model_response: body,
        json_mode: false,
        converted_from_mock,
    })
}

pub(crate) fn responses_stream(body: Json) -> CallOutcome {
    CallOutcome::ResponsesStream(body)
}

fn cache_outcome(
    cached: Option<CallOutcome>,
    final_embedding: Option<Json>,
    all_embeddings_cached: bool,
    uncached_input: Option<Json>,
    preset_cache_key: Option<String>,
) -> CacheOutcome {
    CacheOutcome {
        cached,
        final_embedding,
        all_embeddings_cached,
        uncached_input,
        preset_cache_key,
    }
}

pub(crate) fn cache_hit(cached: CallOutcome) -> CacheOutcome {
    cache_outcome(Some(cached), None, false, None, None)
}

pub(crate) fn cache_all_embeddings(final_embedding: Json) -> CacheOutcome {
    cache_outcome(None, Some(final_embedding), true, None, None)
}

pub(crate) fn cache_partial_embedding(final_embedding: Json, uncached_input: Json) -> CacheOutcome {
    cache_outcome(
        None,
        Some(final_embedding),
        false,
        Some(uncached_input),
        None,
    )
}

pub(crate) fn cache_with_preset_key(key: &str) -> CacheOutcome {
    cache_outcome(None, None, false, None, Some(key.to_owned()))
}

pub(crate) struct NoRules;

impl RuleEvaluator for NoRules {
    fn evaluate(&self, _index: usize, _input: &str) -> Option<RuleDecision> {
        None
    }
}

pub(crate) struct UnknownCallbacks;

impl KnownCallback for UnknownCallbacks {
    fn is_known(&self, _name: &str) -> bool {
        false
    }
}

pub(crate) struct NoLoggers;

impl LoggerResolver for NoLoggers {
    fn resolve(
        &self,
        _name: &str,
        _params: &BTreeMap<String, String>,
    ) -> Result<Option<Arc<dyn CallbackLogger>>, RegistryError> {
        Ok(None)
    }
}

#[derive(Default)]
pub(crate) struct StepClock {
    ticks: Mutex<i64>,
}

impl StepClock {
    fn next_tick(&self) -> i64 {
        let mut ticks = self.ticks.lock().expect("the step clock lock is free");
        *ticks = ticks.saturating_add(1);
        *ticks
    }
}

impl WrapperClock for StepClock {
    fn now(&self) -> DateTime<FixedOffset> {
        let tick = self.next_tick();
        moment()
            .checked_add_signed(TimeDelta::seconds(tick))
            .expect("the step clock stays in range")
    }

    fn monotonic(&self) -> Duration {
        Duration::from_millis(0)
    }
}

#[derive(Default)]
pub(crate) struct SequenceIds {
    next: Mutex<u64>,
}

impl IdSource for SequenceIds {
    fn next_call_id(&self) -> String {
        let mut next = self.next.lock().expect("the id lock is free");
        *next = next.saturating_add(1);
        let id = *next;
        drop(next);
        format!("generated-{id}")
    }
}

fn default_outcome() -> CallOutcome {
    CallOutcome::Response(Json::Object(IndexMap::new()))
}

#[derive(Default)]
pub(crate) struct Stub {
    name: String,
    outcomes: Mutex<VecDeque<Result<CallOutcome, CallError>>>,
    retry_outcomes: Mutex<VecDeque<Result<CallOutcome, CallError>>>,
    calls: Mutex<Vec<RequestMap>>,
    retries: Mutex<Vec<RequestMap>>,
    handles: Mutex<Vec<Option<LoggingHandle>>>,
    capture: Mutex<Option<Json>>,
}

impl Stub {
    pub(crate) fn new(name: &str) -> Self {
        Self::default().with_name(name)
    }

    fn with_name(mut self, name: &str) -> Self {
        self.name = name.to_owned();
        self
    }

    pub(crate) fn scripted(name: &str, outcomes: Vec<Result<CallOutcome, CallError>>) -> Self {
        let stub = Self::new(name);
        stub.outcomes
            .lock()
            .expect("the outcome lock is free")
            .extend(outcomes);
        stub
    }

    pub(crate) fn responding(name: &str, outcome: CallOutcome) -> Self {
        Self::scripted(name, vec![Ok(outcome)])
    }

    pub(crate) fn failing(name: &str, error: CallError) -> Self {
        Self::scripted(name, vec![Err(error)])
    }

    pub(crate) fn with_retry(self, outcome: CallOutcome) -> Self {
        self.retry_outcomes
            .lock()
            .expect("the retry outcome lock is free")
            .push_back(Ok(outcome));
        self
    }

    pub(crate) fn with_capture(self, value: Json) -> Self {
        *self.capture.lock().expect("the capture lock is free") = Some(value);
        self
    }

    pub(crate) fn with_failed_retry(self, error: CallError) -> Self {
        self.retry_outcomes
            .lock()
            .expect("the retry outcome lock is free")
            .push_back(Err(error));
        self
    }

    pub(crate) fn calls(&self) -> Vec<RequestMap> {
        self.calls.lock().expect("the call lock is free").clone()
    }

    pub(crate) fn call_count(&self) -> usize {
        self.calls.lock().expect("the call lock is free").len()
    }

    pub(crate) fn retries(&self) -> Vec<RequestMap> {
        self.retries.lock().expect("the retry lock is free").clone()
    }

    pub(crate) fn retry_count(&self) -> usize {
        self.retries.lock().expect("the retry lock is free").len()
    }

    pub(crate) fn handle(&self, index: usize) -> LoggingHandle {
        self.handles
            .lock()
            .expect("the handle lock is free")
            .get(index)
            .and_then(Option::clone)
            .expect("the stub saw a logging handle")
    }

    pub(crate) fn last_handle(&self) -> Option<LoggingHandle> {
        self.handles
            .lock()
            .expect("the handle lock is free")
            .last()
            .and_then(Option::clone)
    }
}

fn record_request(
    requests: &Mutex<Vec<RequestMap>>,
    handles: &Mutex<Vec<Option<LoggingHandle>>>,
    request: RequestMap,
    logging: Option<&LoggingHandle>,
) {
    requests
        .lock()
        .expect("the request lock is free")
        .push(request);
    handles
        .lock()
        .expect("the handle lock is free")
        .push(logging.cloned());
}

fn take_outcome(
    outcomes: &Mutex<VecDeque<Result<CallOutcome, CallError>>>,
) -> Option<Result<CallOutcome, CallError>> {
    outcomes
        .lock()
        .expect("the outcome lock is free")
        .pop_front()
}

#[async_trait]
impl CallTarget for Stub {
    fn name(&self) -> &str {
        &self.name
    }

    async fn call(
        &self,
        request: RequestMap,
        logging: Option<&LoggingHandle>,
    ) -> Result<CallOutcome, CallError> {
        record_request(&self.calls, &self.handles, request, logging);
        let capture = self
            .capture
            .lock()
            .expect("the capture lock is free")
            .clone();
        if let (Some(value), Some(handle)) = (capture, logging) {
            let mut state = handle.state.lock().expect("the state lock is free");
            state
                .model_call_details
                .insert("raw_request_typed_dict".to_owned(), value);
        }
        take_outcome(&self.outcomes).unwrap_or_else(|| Ok(default_outcome()))
    }

    async fn retry(
        &self,
        request: RequestMap,
        logging: Option<&LoggingHandle>,
    ) -> Result<CallOutcome, CallError> {
        record_request(&self.retries, &self.handles, request, logging);
        take_outcome(&self.retry_outcomes).unwrap_or_else(|| Ok(default_outcome()))
    }
}

#[derive(Default)]
pub(crate) struct ScriptedCache {
    found: Mutex<Option<CacheOutcome>>,
    lookups: Mutex<Vec<CacheMode>>,
    stored: Mutex<Vec<CallOutcome>>,
    wrapped: Mutex<bool>,
    combined: Mutex<bool>,
}

impl ScriptedCache {
    pub(crate) fn set_found(&self, found: CacheOutcome) {
        *self.found.lock().expect("the found lock is free") = Some(found);
    }

    pub(crate) fn lookups(&self) -> Vec<CacheMode> {
        self.lookups
            .lock()
            .expect("the lookup lock is free")
            .clone()
    }

    pub(crate) fn stored(&self) -> Vec<CallOutcome> {
        self.stored.lock().expect("the store lock is free").clone()
    }

    pub(crate) fn was_wrapped(&self) -> bool {
        *self.wrapped.lock().expect("the wrap lock is free")
    }

    pub(crate) fn was_combined(&self) -> bool {
        *self.combined.lock().expect("the combine lock is free")
    }
}

#[async_trait]
impl CacheHandler for ScriptedCache {
    async fn lookup(
        &self,
        _request: &RequestMap,
        _call_type: &str,
        mode: CacheMode,
    ) -> Result<Option<CacheOutcome>, CallError> {
        self.lookups
            .lock()
            .expect("the lookup lock is free")
            .push(mode);
        let found = self.found.lock().expect("the found lock is free").clone();
        Ok(found)
    }

    async fn store(
        &self,
        _request: &RequestMap,
        outcome: &CallOutcome,
        _call_type: &str,
        _mode: CacheMode,
    ) -> Result<(), CallError> {
        self.stored
            .lock()
            .expect("the store lock is free")
            .push(outcome.clone());
        Ok(())
    }

    fn wrap_streaming_result(&self, outcome: CallOutcome, _call_type: &str) -> CallOutcome {
        *self.wrapped.lock().expect("the wrap lock is free") = true;
        outcome
    }

    fn combine_cached_embedding(
        &self,
        _final_embedding: &Json,
        fresh: CallOutcome,
        _times: CallTimes,
    ) -> CallOutcome {
        *self.combined.lock().expect("the combine lock is free") = true;
        fresh
    }
}

#[derive(Default)]
pub(crate) struct RecordingLogging {
    events: Mutex<Vec<String>>,
}

impl RecordingLogging {
    fn record(&self, event: String) {
        self.events
            .lock()
            .expect("the event lock is free")
            .push(event);
    }

    fn record_labelled(&self, prefix: &str, result: &SuccessResult) {
        self.record(format!("{prefix}:{}", result_label(result)));
    }

    pub(crate) fn events(&self) -> Vec<String> {
        self.events.lock().expect("the event lock is free").clone()
    }
}

fn result_label(result: &SuccessResult) -> &'static str {
    match result {
        SuccessResult::Chat(_) => "chat",
        SuccessResult::Embeddings(_) => "embeddings",
        SuccessResult::Chunk(_) => "chunk",
        SuccessResult::Other(_) => "other",
    }
}

#[async_trait]
impl CallLogging for RecordingLogging {
    fn success(&self, _handle: &LoggingHandle, result: SuccessResult, _times: CallTimes) {
        self.record_labelled("success", &result);
    }

    async fn async_success(&self, _h: &LoggingHandle, result: SuccessResult, _t: CallTimes) {
        self.record_labelled("async_success", &result);
    }

    fn sync_success_callbacks_for_async_calls(
        &self,
        _handle: &LoggingHandle,
        result: SuccessResult,
        _times: CallTimes,
    ) {
        self.record_labelled("sync_callbacks", &result);
    }

    fn failure_sync(&self, _handle: &LoggingHandle, error: &CallError, _times: CallTimes) {
        self.record(format!("failure_sync:{error}"));
    }

    async fn failure_async(&self, _handle: &LoggingHandle, error: &CallError, _times: CallTimes) {
        self.record(format!("failure_async:{error}"));
    }
}

#[derive(Default)]
pub(crate) struct RecordingScheduler {
    jobs: Mutex<Vec<AsyncSuccessLog>>,
}

impl RecordingScheduler {
    pub(crate) fn count(&self) -> usize {
        self.jobs.lock().expect("the scheduler lock is free").len()
    }

    pub(crate) fn take(&self) -> Vec<AsyncSuccessLog> {
        let mut jobs = self.jobs.lock().expect("the scheduler lock is free");
        take(&mut *jobs)
    }
}

impl SuccessLogScheduler for RecordingScheduler {
    fn schedule(&self, job: AsyncSuccessLog) -> Admission {
        self.jobs
            .lock()
            .expect("the scheduler lock is free")
            .push(job);
        Admission::Accepted
    }
}

pub(crate) struct FakeStream;

impl StreamAssembly for FakeStream {
    fn assemble(
        &self,
        outcome: &CallOutcome,
        _messages: Option<&Json>,
        _logging: Option<&LoggingHandle>,
    ) -> Result<Option<Json>, CallError> {
        match outcome {
            CallOutcome::ChatStream(stream) => Ok(Some(stream.model_response.clone())),
            CallOutcome::Response(body) | CallOutcome::ResponsesStream(body) => {
                Ok(Some(body.clone()))
            }
        }
    }
}

#[derive(Default)]
pub(crate) struct FakeTokens {
    calls: Mutex<Vec<(String, Option<String>, i64)>>,
}

impl FakeTokens {
    pub(crate) fn calls(&self) -> Vec<(String, Option<String>, i64)> {
        self.calls.lock().expect("the token lock is free").clone()
    }
}

impl MaxTokensModifier for FakeTokens {
    fn modified_max_tokens(
        &self,
        model: &str,
        hf_model_name: Option<&str>,
        _messages: Option<&Json>,
        user_max_tokens: i64,
    ) -> Option<i64> {
        self.calls.lock().expect("the token lock is free").push((
            model.to_owned(),
            hf_model_name.map(str::to_owned),
            user_max_tokens,
        ));
        Some(user_max_tokens.saturating_sub(1))
    }
}

#[derive(Default)]
pub(crate) struct RecordingMetadata {
    updates: Mutex<Vec<(Option<String>, RequestMap)>>,
}

impl RecordingMetadata {
    pub(crate) fn count(&self) -> usize {
        self.updates
            .lock()
            .expect("the metadata lock is free")
            .len()
    }
}

impl ResponseMetadataUpdater for RecordingMetadata {
    fn update(&self, _logging: &LoggingHandle, update: MetadataUpdate<'_>) {
        self.updates
            .lock()
            .expect("the metadata lock is free")
            .push((update.model.map(str::to_owned), update.request.clone()));
    }
}

pub(crate) struct NoCredentials;

impl CredentialLookup for NoCredentials {
    fn find_credential(&self, _credential_name: &str) -> Option<&Credential> {
        None
    }
}

pub(crate) struct MarkingHook {
    label: String,
    marker: String,
}

impl MarkingHook {
    pub(crate) fn new(label: &str, marker: &str) -> Self {
        Self {
            label: label.to_owned(),
            marker: marker.to_owned(),
        }
    }

    fn mark(&self, mut fields: IndexMap<String, Json>) -> IndexMap<String, Json> {
        fields.insert(self.marker.clone(), Json::Bool(true));
        fields
    }
}

#[async_trait]
impl CallbackLogger for MarkingHook {
    fn callback_name(&self) -> &str {
        &self.label
    }
}

#[async_trait]
impl DeploymentHook for MarkingHook {
    async fn pre_call_deployment_hook(
        &self,
        request: &mut RequestMap,
        _call_type: Option<CallType>,
    ) -> Result<Option<RequestMap>, HookError> {
        let modified = self.mark(request.clone());
        Ok(Some(modified))
    }

    async fn success_deployment_hook(
        &self,
        _request_data: &RequestMap,
        response: &Json,
        _call_type: Option<CallType>,
    ) -> Result<Option<Json>, HookError> {
        let Json::Object(fields) = response.clone() else {
            return Ok(None);
        };
        Ok(Some(Json::Object(self.mark(fields))))
    }
}

pub(crate) struct Fixture {
    pub(crate) wrapper: CallWrapper<NoRules>,
    pub(crate) cache: Arc<ScriptedCache>,
    pub(crate) logging: Arc<RecordingLogging>,
    pub(crate) scheduler: Arc<RecordingScheduler>,
    pub(crate) tokens: Arc<FakeTokens>,
    pub(crate) metadata: Arc<RecordingMetadata>,
}

impl Fixture {
    pub(crate) fn with_hooks(mut self, hooks: Vec<Arc<dyn DeploymentHook>>) -> Self {
        self.wrapper.setup.callbacks = hooks;
        self
    }

    pub(crate) fn with_cache(mut self) -> Self {
        let cache: Arc<ScriptedCache> = Arc::clone(&self.cache);
        self.wrapper.cache = Some(cache);
        self
    }
}

pub(crate) fn run_sync(
    fixture: &mut Fixture,
    stub: &Stub,
    request: RequestMap,
) -> Result<CallOutcome, CallError> {
    let runtime = Runtime::new().map_err(|error| plain_error(&error.to_string()))?;
    fixture
        .wrapper
        .call_blocking(stub, request, WrapperOptions::default(), &runtime)
}

pub(crate) fn fixture() -> Fixture {
    let cache = Arc::new(ScriptedCache::default());
    let logging = Arc::new(RecordingLogging::default());
    let scheduler = Arc::new(RecordingScheduler::default());
    let tokens = Arc::new(FakeTokens::default());
    let metadata = Arc::new(RecordingMetadata::default());
    let wrapper = fixture_wrapper(
        Arc::clone(&logging),
        Arc::clone(&scheduler),
        Arc::clone(&tokens),
        Arc::clone(&metadata),
    );
    Fixture {
        wrapper,
        cache,
        logging,
        scheduler,
        tokens,
        metadata,
    }
}

fn fixture_wrapper(
    logging: Arc<RecordingLogging>,
    scheduler: Arc<RecordingScheduler>,
    tokens: Arc<FakeTokens>,
    metadata: Arc<RecordingMetadata>,
) -> CallWrapper<NoRules> {
    CallWrapper {
        rules: NoRules,
        setup: SetupState {
            lists: CallbackLists::default(),
            providers: ProviderRegistry::default(),
            callbacks: Vec::new(),
            known: Arc::new(UnknownCallbacks),
            resolver: Arc::new(NoLoggers),
            correlation_settings: CorrelationSettings::default(),
            redactor: redactor(),
            litellm_params: IndexMap::new(),
            turn_off_message_logging: false,
            breadcrumbs: None,
        },
        cache: None,
        budget: BudgetState::default(),
        retry: WrapperRetryDefaults::default(),
        clock: Arc::new(StepClock::default()),
        ids: Arc::new(SequenceIds::default()),
        max_tokens: tokens,
        stream_assembly: Arc::new(FakeStream),
        metadata,
        credentials: Arc::new(NoCredentials),
        logging,
        scheduler,
        debugging: false,
        flags: WrapperFlags {
            modify_params: true,
            enable_json_schema_validation: false,
            schema_limits: SchemaLimits::default(),
        },
    }
}
