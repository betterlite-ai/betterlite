use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::types::{LiteLlmParams, ModelDeployment, ModelInfo};
use betterlite_context::agentic_callback::error::AgenticHookError;
use betterlite_context::agentic_callback::{
    AgenticCall, AgenticCallback, AgenticGate, AgenticLoopCall, AgenticOverrides,
};
use betterlite_context::agentic_loop_plan::JsonMap;
use betterlite_context::non_default_params::mapper::OptionalParamsMapper;
use betterlite_core::api::chat_agentic_loop::fake_stream::FakeStreamPort;
use betterlite_core::api::chat_agentic_loop::{
    ChatLoopResult, LoggingHandle as AgenticLoggingHandle,
};
use betterlite_core::dispatch::log::TracingCallLog;
use betterlite_core::dispatch::{Dispatch, NoRules};
use betterlite_core::operation::chat::{
    ChatCall, ChatCallInput, ChatCompletionAnswer, ChatFailure, ChatStreamAnswer,
};
use betterlite_core::responses_bridge::{
    ModelInfoError, ReasoningEffort, ResponsesBridgeCapabilities,
};
use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_models::controls::call_params::CallParams;
use betterlite_models::controls::control_options::ControlOptions;
use betterlite_models::controls::optional_params::OptionalParams;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::params::{ParameterError, ParameterMapping};
use betterlite_models::request_pricing::register::CustomPricingRequest;
use betterlite_models::resolution::{ProviderIdentity, ResolutionError, ResolvedModel};
use betterlite_openai::chat::ChatRequest as ClientChatRequest;
use betterlite_openai::chat::MockSettings;
use betterlite_openai::chat::deployment::{ChatDeployment, OpenAiDefaults};
use betterlite_openai::openai_base_config::Object;
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use futures_util::{StreamExt as _, stream};
use http::HeaderMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::value::RawValue;

use crate::call_setup::RequestMap;
use crate::call_setup::correlation::CallOutcome;
use crate::call_wrapper::error::{CallError, plain_error};
use crate::call_wrapper::ports::{IdSource, WrapperClock};
use crate::call_wrapper::state::LoggingHandle;
use crate::call_wrapper::target::CallTarget;
use crate::completion_call::ports::{
    FoundryRules, LocalTokenCountRequest, LocalTokenCounter, ModelFacts, NoMcpLoop,
    ProviderTokenCountRequest, ProviderTokenCounter, Sleeper,
};
use crate::completion_call::target::CompletionTarget;
use crate::completion_call::target_async::AsyncCompletionTarget;
use crate::completion_call::tokens::TokenCountResponse;
use crate::completion_request::args::{
    CompletionArgs, CompletionFlags, CompletionSettings, JsonObject,
};
use crate::completion_request::call::{PreparedCall, PreparedOptionalParams, PreparedTimeout};
use crate::completion_request::error::{CompletionRequestError, PricingFailure};
use crate::completion_request::ports::{
    CacheSeedRequest, ChatPolicy, CompletionLogging, CompletionPorts, CompletionTimeout,
    EnvironmentUpdate, ModelMetadata, ParameterMappingRequest, PricingRegistration,
    PromptManagement, PromptManagementError, PromptOutcome, PromptRequest, ProviderLookup,
    TimeoutRequest,
};

pub(super) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

pub(super) fn request(text: &str) -> RequestMap {
    serde_json::from_str(text).unwrap()
}

pub(super) fn args_from(text: &str) -> CompletionArgs {
    serde_json::from_str(text).unwrap()
}

pub(super) fn prepared_call(model: &str) -> PreparedCall {
    PreparedCall {
        model: model.to_owned(),
        custom_llm_provider: None,
        messages: Vec::new(),
        tools: None,
        kwargs: JsonObject::new(),
        options: PreparedOptionalParams::default(),
        non_default_params: JsonObject::new(),
        litellm_params: CallParams::default(),
        headers: AdditionalHeaders::new(),
        api_key: None,
        api_base: None,
        timeout: None,
        max_retries: None,
        stream: false,
        choice_count: None,
        model_info: JsonObject::new(),
        hidden_params: JsonObject::new(),
        custom_prompt_dict: JsonObject::new(),
        base_model: None,
    }
}

pub(super) fn dig<'json>(json: &'json Json, path: &[&str]) -> Option<&'json Json> {
    let mut current = json;
    for key in path {
        let Json::Object(fields) = current else {
            return None;
        };
        current = fields.get(*key)?;
    }
    Some(current)
}

pub(super) fn settings_default() -> CompletionSettings {
    CompletionSettings {
        drop_params: None,
        model_fallbacks: None,
        model_alias_map: BTreeMap::new(),
        num_retries: None,
        request_timeout: None,
        api_key: None,
        api_base: None,
        route_all_to_responses: false,
        proxy_headers: AdditionalHeaders::new(),
        optional: OptionalParams::default(),
        flags: CompletionFlags {
            add_function_to_prompt: false,
            disable_stop_sequence_limit: false,
        },
        medium_thinking_budget: 2048,
    }
}

fn listed_deployment() -> ModelDeployment {
    ModelDeployment {
        model_name: String::from("gpt-4o"),
        litellm_params: LiteLlmParams {
            ssl_verify: None,
            model: Some(String::from("openai/up")),
            api_base: Some(String::from("http://127.0.0.1:9/v1")),
            api_key: Some(SecretString::from(String::from("configured-key"))),
            api_version: None,
            timeout: None,
            stream_timeout: None,
            max_retries: None,
            rpm: None,
            tpm: None,
            weight: None,
        },
        model_info: ModelInfo::default(),
    }
}

pub(super) fn deployment() -> ChatDeployment {
    let mut resolved = ChatDeployment::resolve(&listed_deployment(), &OpenAiDefaults::default())
        .expect("the fixture deployment resolves");
    resolved.model_id = String::from("gpt-4o");
    resolved
}

pub(super) fn deployment_named(model_id: &str) -> ChatDeployment {
    let mut resolved = deployment();
    resolved.model_id = model_id.to_owned();
    resolved
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Lookup {
    pub model: String,
    pub api_base: Option<String>,
}

#[derive(Default)]
pub(super) struct FakeMetadata {
    pub resolved_model: Option<String>,
    pub resolved_provider: Option<&'static str>,
    pub resolved_base: Option<String>,
    pub dynamic_key: Option<String>,
    pub lookups: Arc<Mutex<Vec<Lookup>>>,
}

impl FakeMetadata {
    fn resolving_with(
        resolved_model: Option<String>,
        resolved_base: Option<String>,
        dynamic_key: Option<String>,
    ) -> Self {
        Self {
            resolved_model,
            resolved_provider: None,
            resolved_base,
            dynamic_key,
            lookups: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(super) fn resolving(model: &str) -> Self {
        Self::resolving_with(Some(String::from(model)), None, None)
    }

    pub(super) fn resolving_credentials(base: &str, key: &str) -> Self {
        Self::resolving_with(None, Some(String::from(base)), Some(String::from(key)))
    }
}

fn provider_identity(name: Option<&str>) -> ProviderIdentity {
    match name {
        Some("anthropic") => ProviderIdentity::Known(ProviderId::Anthropic),
        Some("azure") => ProviderIdentity::Known(ProviderId::Azure),
        Some(other) => ProviderIdentity::Custom(other.to_owned()),
        None => ProviderIdentity::Known(ProviderId::Openai),
    }
}

impl ModelMetadata for FakeMetadata {
    fn resolve_provider(
        &self,
        request: &ProviderLookup<'_>,
    ) -> Result<ResolvedModel, ResolutionError> {
        lock(&self.lookups).push(Lookup {
            model: request.model.to_owned(),
            api_base: request.api_base.map(str::to_owned),
        });
        Ok(ResolvedModel {
            model: self
                .resolved_model
                .clone()
                .unwrap_or_else(|| request.model.to_owned()),
            provider: provider_identity(self.resolved_provider.or(request.custom_llm_provider)),
            api_base: self
                .resolved_base
                .clone()
                .or_else(|| request.api_base.map(str::to_owned)),
            api_key: self
                .dynamic_key
                .as_ref()
                .map(|key| SecretString::from(key.clone()))
                .or_else(|| request.api_key.cloned()),
        })
    }

    fn chat_policy<'config>(
        &'config self,
        _model: &str,
        _provider: &str,
        _base_model: Option<&str>,
    ) -> ChatPolicy<'config> {
        ChatPolicy {
            translates_developer_role: false,
            mapper: None::<&'config dyn OptionalParamsMapper>,
        }
    }

    fn parameter_mapping(
        &self,
        _request: &ParameterMappingRequest<'_>,
    ) -> Result<ParameterMapping, ParameterError> {
        Ok(ParameterMapping::default())
    }
}

#[derive(Default)]
pub(super) struct FakePrompt;

#[async_trait]
impl PromptManagement for FakePrompt {
    fn seed_cache_control(
        &self,
        _request: &CacheSeedRequest<'_>,
        _non_default_params: &mut JsonObject,
    ) {
    }

    fn should_run(
        &self,
        _prompt_id: Option<&str>,
        _non_default_params: &JsonObject,
        _tools: Option<&[Json]>,
    ) -> bool {
        false
    }

    async fn get_chat_completion_prompt(
        &self,
        request: PromptRequest,
    ) -> Result<PromptOutcome, PromptManagementError> {
        Ok(prompt_outcome(request))
    }

    async fn async_completion_prompt(
        &self,
        request: PromptRequest,
    ) -> Result<PromptOutcome, PromptManagementError> {
        Ok(prompt_outcome(request))
    }
}

fn prompt_outcome(request: PromptRequest) -> PromptOutcome {
    PromptOutcome {
        model: request.model,
        messages: request.messages,
        non_default_params: request.non_default_params,
    }
}

#[derive(Default)]
pub(super) struct FakeBridge {
    pub mode: Option<String>,
}

impl ResponsesBridgeCapabilities for FakeBridge {
    fn model_info(&self, _model: &str, _provider: &str) -> Result<Object, ModelInfoError> {
        let text = self.mode.as_ref().map_or_else(
            || String::from("{}"),
            |mode| format!("{{\"mode\":\"{mode}\"}}"),
        );
        serde_json::from_str(&text).map_err(|_error| ModelInfoError::Unknown {
            model: String::from("fixture"),
        })
    }

    fn resolved_openai_api_base(&self, api_base: Option<&str>) -> String {
        api_base.unwrap_or("https://api.openai.com/v1").to_owned()
    }

    fn foundry_supports_responses(&self, _model: &str, _api_base: Option<&str>) -> bool {
        false
    }

    fn foundry_chat_rejects_tools(
        &self,
        _model: &str,
        _reasoning_effort: Option<&ReasoningEffort>,
    ) -> bool {
        false
    }

    fn is_gpt_5_model(&self, _model: &str) -> bool {
        false
    }

    fn is_gpt_5_4_plus_model(&self, _model: &str) -> bool {
        false
    }

    fn is_gpt_5_search_model(&self, _model: &str) -> bool {
        false
    }
}

#[derive(Default)]
pub(super) struct FakePricing;

#[async_trait]
impl PricingRegistration for FakePricing {
    async fn register_custom_pricing(
        &self,
        _request: &CustomPricingRequest<'_>,
    ) -> Result<(), PricingFailure> {
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct FakeLogging;

impl CompletionLogging for FakeLogging {
    fn update_environment(&self, _update: &EnvironmentUpdate<'_>) {}
}

#[derive(Default)]
pub(super) struct FakeTimeout;

impl CompletionTimeout for FakeTimeout {
    fn resolve(
        &self,
        _request: &TimeoutRequest<'_>,
    ) -> Result<PreparedTimeout, CompletionRequestError> {
        Ok(PreparedTimeout::Seconds(600.0))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SeenCall {
    pub model: Option<String>,
    pub stream: bool,
    pub max_tokens: Option<u32>,
    pub body: String,
}

pub(super) struct FakeChat {
    completion: Mutex<String>,
    frames: Mutex<Vec<String>>,
    null_models: Mutex<Vec<String>>,
    failures: Mutex<VecDeque<ChatFailure>>,
    seen: Arc<Mutex<Vec<SeenCall>>>,
}

const DEFAULT_COMPLETION: &str = r#"{"id":"chatcmpl-1","object":"chat.completion","created":1700000000,"model":"gpt-4o","choices":[{"index":0,"message":{"role":"assistant","content":"hello"},"finish_reason":"stop"}]}"#;

fn fake_chat() -> FakeChat {
    FakeChat {
        completion: Mutex::new(String::from(DEFAULT_COMPLETION)),
        frames: Mutex::new(Vec::new()),
        null_models: Mutex::new(Vec::new()),
        failures: Mutex::new(VecDeque::new()),
        seen: Arc::new(Mutex::new(Vec::new())),
    }
}

impl FakeChat {
    pub(super) fn answers_null(&self, model: &str) {
        lock(&self.null_models).push(model.to_owned());
    }

    pub(super) fn fails_with(&self, failure: ChatFailure) {
        lock(&self.failures).push_back(failure);
    }

    pub(super) fn streams(&self, frames: &[&str]) {
        *lock(&self.frames) = frames.iter().map(|frame| (*frame).to_owned()).collect();
    }

    pub(super) fn seen(&self) -> Vec<SeenCall> {
        lock(&self.seen).clone()
    }

    fn answers_empty(&self, request: &ClientChatRequest) -> bool {
        request
            .model
            .as_deref()
            .is_some_and(|model| lock(&self.null_models).iter().any(|null| null == model))
    }

    fn record(&self, call: &ChatCallInput<'_>) {
        lock(&self.seen).push(SeenCall {
            model: call.request.model.clone(),
            stream: call.request.stream,
            max_tokens: call.request.max_tokens,
            body: serde_json::to_string(call.request).unwrap_or_default(),
        });
    }
}

#[async_trait]
impl ChatCall for FakeChat {
    fn prepare(&self, _request: &ClientChatRequest) -> Result<ControlOptions, ChatFailure> {
        Ok(ControlOptions::default())
    }

    async fn complete(
        &self,
        call: ChatCallInput<'_>,
        _control: ControlOptions,
    ) -> Result<ChatCompletionAnswer, ChatFailure> {
        self.record(&call);
        let queued = lock(&self.failures).pop_front();
        if let Some(failure) = queued {
            return Err(failure);
        }
        let body = if self.answers_empty(call.request) {
            String::from("null")
        } else {
            lock(&self.completion).clone()
        };
        Ok(ChatCompletionAnswer {
            body: body.into_bytes(),
            upstream_headers: HeaderMap::new(),
            upstream_time: Duration::ZERO,
        })
    }

    async fn stream(
        &self,
        call: ChatCallInput<'_>,
        _control: ControlOptions,
    ) -> Result<ChatStreamAnswer, ChatFailure> {
        self.record(&call);
        let frames: Vec<String> = lock(&self.frames).clone();
        let chunks = frames.into_iter().map(|frame| Ok(frame.into_bytes()));
        Ok(ChatStreamAnswer {
            body: stream::iter(chunks).boxed(),
            upstream_headers: HeaderMap::new(),
            upstream_time: Duration::ZERO,
        })
    }
}

#[derive(Default)]
pub(super) struct FakeSleeper {
    pub slept: Arc<Mutex<Vec<Duration>>>,
}

#[async_trait]
impl Sleeper for FakeSleeper {
    async fn sleep(&self, duration: Duration) {
        lock(&self.slept).push(duration);
    }
}

impl FakeSleeper {
    pub(super) fn slept(&self) -> Vec<Duration> {
        lock(&self.slept).clone()
    }
}

pub(super) struct FakeIds;

impl IdSource for FakeIds {
    fn next_call_id(&self) -> String {
        String::from("call-1")
    }
}

pub(super) struct FakeClock;

impl WrapperClock for FakeClock {
    fn now(&self) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339("2023-11-14T22:13:20+00:00").expect("a fixed timestamp")
    }

    fn monotonic(&self) -> Duration {
        Duration::ZERO
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct FoundryCall {
    pub model: String,
    pub api_base: Option<String>,
    pub rejects: bool,
}

#[derive(Default)]
pub(super) struct FakeFoundry {
    pub supports: bool,
    pub rejects: bool,
    pub calls: Arc<Mutex<Vec<FoundryCall>>>,
}

impl FoundryRules for FakeFoundry {
    fn supports_responses(&self, model: &str, api_base: Option<&str>) -> bool {
        lock(&self.calls).push(FoundryCall {
            model: model.to_owned(),
            api_base: api_base.map(str::to_owned),
            rejects: false,
        });
        self.supports
    }

    fn chat_rejects_tools(&self, model: &str, _reasoning_effort: Option<&ReasoningEffort>) -> bool {
        lock(&self.calls).push(FoundryCall {
            model: model.to_owned(),
            api_base: None,
            rejects: true,
        });
        self.rejects
    }
}

#[derive(Default)]
pub(super) struct FakeFacts {
    pub info: Option<Object>,
    pub seen: Arc<Mutex<Vec<String>>>,
}

impl ModelFacts for FakeFacts {
    fn model_info(&self, model: &str, _custom_llm_provider: &str) -> Option<Object> {
        lock(&self.seen).push(model.to_owned());
        self.info.clone()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ProviderSeen {
    pub model_to_use: String,
    pub request_model: String,
    pub api_base: Option<String>,
    pub key: Option<String>,
    pub system: Option<String>,
}

pub(super) struct FakeProviderCounter {
    pub use_all: bool,
    pub answer: Mutex<Option<TokenCountResponse>>,
    pub seen: Arc<Mutex<Vec<ProviderSeen>>>,
}

impl FakeProviderCounter {
    pub(super) fn answering(answer: TokenCountResponse) -> Self {
        Self {
            use_all: true,
            answer: Mutex::new(Some(answer)),
            seen: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(super) fn unavailable() -> Self {
        Self {
            use_all: false,
            answer: Mutex::new(None),
            seen: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(super) fn silent() -> Self {
        Self {
            use_all: true,
            answer: Mutex::new(None),
            seen: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(super) fn seen(&self) -> Vec<ProviderSeen> {
        lock(&self.seen).clone()
    }
}

#[async_trait]
impl ProviderTokenCounter for FakeProviderCounter {
    fn should_use(&self, _custom_llm_provider: &str) -> bool {
        self.use_all
    }

    async fn count(&self, request: &ProviderTokenCountRequest<'_>) -> Option<TokenCountResponse> {
        lock(&self.seen).push(ProviderSeen {
            model_to_use: request.model_to_use.to_owned(),
            request_model: request.request_model.to_owned(),
            api_base: request.api_base.map(str::to_owned),
            key: request
                .api_key
                .as_ref()
                .map(|key| key.expose_secret().to_owned()),
            system: request.system.map(str::to_owned),
        });
        lock(&self.answer).clone()
    }
}

pub(super) struct FakeLocalCounter {
    pub total: usize,
    pub seen: Arc<Mutex<Vec<Vec<Json>>>>,
}

impl FakeLocalCounter {
    pub(super) fn new(total: usize) -> Self {
        Self {
            total,
            seen: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(super) fn seen(&self) -> Vec<Vec<Json>> {
        lock(&self.seen).clone()
    }
}

#[async_trait]
impl LocalTokenCounter for FakeLocalCounter {
    async fn count(&self, request: &LocalTokenCountRequest<'_>) -> usize {
        lock(&self.seen).push(request.messages.to_vec());
        self.total
    }
}

pub(super) struct RecordingTarget {
    pub calls: Arc<Mutex<Vec<RequestMap>>>,
    pub outcomes: Mutex<VecDeque<Result<CallOutcome, CallError>>>,
    pub fail_all: bool,
}

fn ok_outcome() -> CallOutcome {
    CallOutcome::Response(Json::Object(JsonObject::from([(
        String::from("id"),
        Json::Text(String::from("ok")),
    )])))
}

impl RecordingTarget {
    pub(super) fn failing_then_ok(failures: usize) -> Self {
        let mut outcomes = VecDeque::new();
        for _ in 0..failures {
            outcomes.push_back(Err(CallError::from(plain_error("first down"))));
        }
        outcomes.push_back(Ok(ok_outcome()));
        Self::queued(outcomes, false)
    }

    pub(super) fn always_failing() -> Self {
        Self::queued(VecDeque::new(), true)
    }

    pub(super) fn empty_then_ok() -> Self {
        let mut outcomes = VecDeque::new();
        outcomes.push_back(Ok(CallOutcome::Response(Json::Null)));
        outcomes.push_back(Ok(ok_outcome()));
        Self::queued(outcomes, false)
    }

    fn queued(outcomes: VecDeque<Result<CallOutcome, CallError>>, fail_all: bool) -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            outcomes: Mutex::new(outcomes),
            fail_all,
        }
    }

    pub(super) fn call_map(&self) -> Vec<RequestMap> {
        lock(&self.calls).clone()
    }
}

#[async_trait]
impl CallTarget for RecordingTarget {
    fn name(&self) -> &'static str {
        "recording"
    }

    async fn call(
        &self,
        request: RequestMap,
        _logging: Option<&LoggingHandle>,
    ) -> Result<CallOutcome, CallError> {
        lock(&self.calls).push(request);
        let queued = lock(&self.outcomes).pop_front();
        if let Some(outcome) = queued {
            return outcome;
        }
        if self.fail_all {
            return Err(CallError::from(plain_error("first down")));
        }
        Ok(CallOutcome::Response(Json::Text(String::from("ok"))))
    }

    async fn retry(
        &self,
        request: RequestMap,
        logging: Option<&LoggingHandle>,
    ) -> Result<CallOutcome, CallError> {
        self.call(request, logging).await
    }
}

pub(super) struct FakeStream;

impl FakeStreamPort for FakeStream {
    fn wrap_model_response(
        &self,
        response: Box<RawValue>,
        _model: &str,
        _provider: &str,
        _logging: &AgenticLoggingHandle<'_>,
    ) -> ChatLoopResult {
        ChatLoopResult::Response(response)
    }
}

const AGENTIC_OVERRIDES: AgenticOverrides = AgenticOverrides {
    should_run: true,
    run_loop: false,
    build_plan: false,
    post_response: false,
    cleanup: false,
};

fn gate(run: bool) -> AgenticGate {
    AgenticGate {
        run,
        tools: JsonMap::new(),
    }
}

fn reply_of(reply: &str) -> Box<RawValue> {
    RawValue::from_string(reply.to_owned()).expect("the fixture reply is JSON")
}

pub(super) struct FakeCallback {
    pub reply: String,
}

#[async_trait]
impl AgenticCallback for FakeCallback {
    fn callback_name(&self) -> &'static str {
        "fake"
    }

    fn overrides(&self) -> AgenticOverrides {
        AGENTIC_OVERRIDES
    }

    async fn should_run_agentic_loop(
        &self,
        _call: &AgenticCall<'_>,
    ) -> Result<AgenticGate, AgenticHookError> {
        Ok(gate(true))
    }

    async fn run_agentic_loop(
        &self,
        _call: &AgenticLoopCall<'_>,
    ) -> Result<Option<Box<RawValue>>, AgenticHookError> {
        Ok(Some(reply_of(&self.reply)))
    }
}

// runs the loop only when the call details the tail built reach it intact
pub(super) struct CheckingCallback {
    pub reply: String,
}

#[async_trait]
impl AgenticCallback for CheckingCallback {
    fn callback_name(&self) -> &'static str {
        "checking"
    }

    fn overrides(&self) -> AgenticOverrides {
        AGENTIC_OVERRIDES
    }

    async fn should_run_agentic_loop(
        &self,
        call: &AgenticCall<'_>,
    ) -> Result<AgenticGate, AgenticHookError> {
        let message = call
            .messages
            .first()
            .is_some_and(|message| message.0.get().contains("\"hi\""));
        let marker = call.kwargs.contains_key("agentic_marker");
        Ok(gate(message && marker && !call.stream))
    }

    async fn run_agentic_loop(
        &self,
        call: &AgenticLoopCall<'_>,
    ) -> Result<Option<Box<RawValue>>, AgenticHookError> {
        if !call.optional_params.contains_key("temperature") {
            return Ok(None);
        }
        Ok(Some(reply_of(&self.reply)))
    }
}

pub(super) struct Harness {
    pub settings: CompletionSettings,
    pub metadata: FakeMetadata,
    pub prompt: FakePrompt,
    pub bridge: FakeBridge,
    pub pricing: FakePricing,
    pub logging: FakeLogging,
    pub timeout: FakeTimeout,
    pub chat: Arc<FakeChat>,
    pub dispatch: Arc<Dispatch>,
    pub sleeper: FakeSleeper,
    pub mcp: NoMcpLoop,
    pub callbacks: Vec<Arc<dyn AgenticCallback>>,
    pub fake_stream: FakeStream,
}

impl Harness {
    pub(super) fn new() -> Self {
        let chat = Arc::new(fake_chat());
        Self {
            settings: settings_default(),
            metadata: FakeMetadata::default(),
            prompt: FakePrompt,
            bridge: FakeBridge::default(),
            pricing: FakePricing,
            logging: FakeLogging,
            timeout: FakeTimeout,
            chat,
            dispatch: Arc::new(Dispatch::new(
                vec![deployment(), deployment_named("empty-model")],
                Arc::new(NoRules),
                Arc::new(TracingCallLog),
            )),
            sleeper: FakeSleeper::default(),
            mcp: NoMcpLoop,
            callbacks: Vec::new(),
            fake_stream: FakeStream,
        }
    }

    pub(super) fn ports(&self) -> CompletionPorts<'_> {
        let Self {
            metadata,
            prompt,
            bridge,
            pricing,
            logging,
            timeout,
            ..
        } = self;
        CompletionPorts {
            model_metadata: metadata,
            prompt,
            bridge,
            pricing,
            logging,
            timeout,
        }
    }

    pub(super) fn target(&self) -> CompletionTarget<'_> {
        let calls: Arc<dyn ChatCall> = Arc::<FakeChat>::clone(&self.chat);
        let settings = self.settings.clone();
        let dispatch = Arc::clone(&self.dispatch);
        let redactor = Redactor::new(16).expect("the default redactor");
        CompletionTarget {
            settings,
            ports: self.ports(),
            dispatch,
            calls,
            mock: MockSettings::default(),
            ids: Arc::new(FakeIds),
            clock: Arc::new(FakeClock),
            redactor,
            mcp: &self.mcp,
            sleeper: &self.sleeper,
        }
    }

    pub(super) fn async_target(&self) -> AsyncCompletionTarget<'_> {
        AsyncCompletionTarget {
            inner: self.target(),
            callbacks: &self.callbacks,
            fake_stream: &self.fake_stream,
        }
    }
}

pub(super) fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

pub(super) fn text_of(value: Option<&Json>) -> Option<&str> {
    match value {
        Some(Json::Text(text)) => Some(text),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            None
        }
    }
}

pub(super) fn chat_body(content: &str) -> String {
    format!(
        r#"{{"id":"chatcmpl-agentic","object":"chat.completion","created":1700000000,"model":"gpt-4o","choices":[{{"index":0,"message":{{"role":"assistant","content":"{content}"}},"finish_reason":"stop"}}]}}"#
    )
}
