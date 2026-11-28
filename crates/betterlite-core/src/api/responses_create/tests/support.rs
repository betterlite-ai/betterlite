use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_context::non_default_params::Params;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::resolution::{
    ProviderIdentity, ResolutionError, ResolutionRequest, ResolvedModel,
};
use betterlite_openai::response_events::{
    ApiKeySources, RawResponse, ResponsesProvider, ResponsesReply, WorkloadIdentity,
    get_complete_url, transform_response, validate_environment,
};
use betterlite_openai::response_input::{ResponsesInput, transform_responses_request};
use betterlite_prompts::arguments::json::{Json as PromptJson, Object};
use betterlite_prompts::file_ids::mapping::FileIdMapping;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::value::RawValue;

use super::builders::env_vars;
use crate::api::response_id_traversal::response_id::transcode;
use crate::api::responses_agentic_execution::executor::{
    AnthropicContinuation, ContinuationExecutor, ResponsesContinuation,
};
use crate::api::responses_compact_http::error::ConfigFailure;
use crate::api::responses_compact_http::service::{ClientSource, SharedSession};
use crate::api::responses_create::bridge::{BridgeCall, ChatBridge, Created, StampedResponse};
use crate::api::responses_create::deps::{
    ConfigRequest, CreateCapabilities, CreateConfigFactory, PromptSource, ResponsesCreateDeps,
};
use crate::api::responses_create::error::{BridgeError, InterceptionError};
use crate::api::responses_create::execute::mock_responses_api_response;
use crate::api::responses_create::interception::{
    CreateInterception, DispatchArgs, FileSearchArgs, Interception, InterceptionResult,
};
use crate::api::responses_create_http::config::{
    CreateConfig, CreateParams, CreateSignRequest, CreateTransformRequest, Headers, Signed,
};
use crate::api::responses_create_http::service::{
    CreateHooks, CreateRequest, CreateTransport, SentReply,
};
use crate::api::responses_hook_policy::callbacks::{
    CallbackLists, DeploymentHookDispatcher, NamedCallbackResolver, PolicyCallback,
};
use crate::api::responses_parameters::error::ResponsesParametersError;
use crate::api::responses_parameters::provider::ResponsesParamProvider;
use crate::api::responses_provider_selection::ParameterMap;
use crate::api::responses_provider_selection::prompting::{
    HookRequest, MergeAnswer, MergeRequest, PromptManager, PromptManagerError,
};
use crate::api::responses_provider_selection::provider_config::{ConfigLookup, NativeConfig};
use crate::api::responses_provider_selection::resolution::ModelResolver;
use crate::api::vector_store_http_write::error::{BaseLlmException, ClientError, HeaderMap};

pub(crate) const OK_RESPONSE: &str = r#"{"id":"resp_a","created_at":100,"object":"response","status":"completed","model":"gpt-4o","output":[]}"#;

#[derive(Clone, Default)]
pub(crate) struct Events(Arc<Mutex<Vec<String>>>);

impl Events {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn push(&self, name: &str) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(name.to_owned());
    }

    pub(crate) fn recorded(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

pub(crate) struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

pub(crate) struct Harness {
    pub(crate) redactor: Redactor,
    pub(crate) sink: Sink,
    pub(crate) now: DateTime<FixedOffset>,
    pub(crate) state: LoggingState,
}

impl Harness {
    pub(crate) fn new() -> Self {
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds");
        let now =
            DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00").expect("the clock parses");
        Self {
            state: logging_state(&redactor),
            redactor,
            sink: Sink,
            now,
        }
    }
}

fn logging_state(redactor: &Redactor) -> LoggingState {
    let init: LoggingInit = serde_json::from_str(
        r#"{"model":"gpt-4o","call_type":"responses","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#,
    )
    .expect("the logging fixture is valid");
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || String::from("generated"),
        Duration::default,
    )
    .expect("the logging state builds")
}

pub(crate) fn environment<'env>(
    redactor: &'env Redactor,
    log: &'env Sink,
    now: DateTime<FixedOffset>,
) -> PrePostEnv<'env> {
    PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now,
    }
}

pub(crate) struct Reply {
    pub(crate) status: u16,
    pub(crate) body: String,
}

#[derive(Clone, Default)]
pub(crate) struct Wire {
    replies: Arc<Mutex<VecDeque<Reply>>>,
    requests: Arc<Mutex<Vec<Arc<CreateRequest>>>>,
    events: Events,
}

impl Wire {
    pub(crate) fn new(events: &Events) -> Self {
        Self {
            replies: Arc::default(),
            requests: Arc::default(),
            events: events.clone(),
        }
    }

    pub(crate) fn queue(&self, status: u16, body: &str) {
        self.replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(Reply {
                status,
                body: body.to_owned(),
            });
    }

    pub(crate) fn requests(&self) -> Vec<Arc<CreateRequest>> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl CreateTransport for Wire {
    fn send(
        &self,
        request: CreateRequest,
        _stream: bool,
    ) -> impl Future<Output = Result<SentReply, TransportError>> + Send {
        self.events.push("send");
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Arc::new(request));
        let queued = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        let Some(reply) = queued else {
            panic!("the test must queue a reply per request");
        };
        ready(Ok(SentReply::Buffered(UpstreamResponse {
            status: reply.status,
            headers: http::HeaderMap::new(),
            body: reply.body.into_bytes(),
        })))
    }
}

type ClientCreations = Vec<(Option<bool>, bool)>;

#[derive(Clone)]
pub(crate) struct Source {
    wire: Wire,
    created: Arc<Mutex<ClientCreations>>,
}

impl Source {
    pub(crate) fn new(wire: &Wire) -> Self {
        Self {
            wire: wire.clone(),
            created: Arc::default(),
        }
    }

    pub(crate) fn created(&self) -> ClientCreations {
        self.created
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ClientSource for Source {
    type Client = Wire;

    fn client(
        &self,
        ssl_verify: Option<bool>,
        shared_session: Option<SharedSession>,
    ) -> Result<Self::Client, ClientError> {
        self.created
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((ssl_verify, shared_session.is_some()));
        Ok(self.wire.clone())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FakeCall {
    pub(crate) model: String,
    pub(crate) stream: Option<bool>,
    pub(crate) provider: String,
}

pub(crate) struct TestConfig {
    events: Events,
    api_key: Option<SecretString>,
    supported: Option<Vec<String>>,
    native_file_search: bool,
    inputs: Arc<Mutex<Vec<ResponsesInput>>>,
    dropped: Arc<Mutex<Vec<bool>>>,
    fake_calls: Arc<Mutex<Vec<FakeCall>>>,
}

impl TestConfig {
    fn new(
        events: &Events,
        supported: Option<Vec<String>>,
        native_file_search: bool,
        configs: &Configs,
    ) -> Self {
        Self {
            events: events.clone(),
            api_key: Some(SecretString::from("sk-provider")),
            supported,
            native_file_search,
            inputs: Arc::clone(&configs.inputs),
            dropped: Arc::clone(&configs.dropped),
            fake_calls: Arc::clone(&configs.fake_calls),
        }
    }
}

struct NoIdentity;

impl WorkloadIdentity for NoIdentity {
    fn bearer_token(
        &mut self,
        _api_key: Option<&SecretString>,
        _api_base: Option<&str>,
    ) -> Option<SecretString> {
        None
    }
}

impl CreateConfig for TestConfig {
    fn validate_environment(
        &self,
        headers: Headers,
        _model: &str,
        params: &CreateParams,
    ) -> Result<Headers, ConfigFailure> {
        let mut identity = NoIdentity;
        Ok(validate_environment(
            headers,
            ApiKeySources {
                litellm_params: params.api_key.as_ref(),
                litellm: None,
                openai: None,
                environment: self.api_key.as_ref(),
            },
            params.api_base.as_deref(),
            ResponsesProvider::OpenAi,
            &mut identity,
        ))
    }

    fn complete_url(&self, params: &CreateParams) -> Result<String, ConfigFailure> {
        Ok(get_complete_url(
            params.api_base.as_deref(),
            None,
            None,
            None,
        ))
    }

    fn transform_request(
        &self,
        request: &CreateTransformRequest<'_>,
    ) -> Result<Object, ConfigFailure> {
        self.inputs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.input.clone());
        transform_responses_request(
            request.model,
            request.input,
            request.optional_params,
            ProviderId::Openai,
            None,
        )
        .map_err(ConfigFailure::new)
    }

    fn normalize_request(&self, data: Object) -> Result<Object, ConfigFailure> {
        Ok(data)
    }

    fn sign_request(&self, request: &CreateSignRequest<'_>) -> Result<Signed, ConfigFailure> {
        self.events.push("sign");
        Ok(Signed {
            headers: request.headers.clone(),
            body: None,
        })
    }

    fn transform_response(
        &self,
        reply: &RawResponse<'_>,
        state: &mut LoggingState,
        env: &PrePostEnv<'_>,
    ) -> Result<ResponsesReply, ConfigFailure> {
        transform_response(reply, env.now.timestamp(), state, env).map_err(ConfigFailure::new)
    }

    fn error_class(&self, message: &str, status_code: u16, headers: HeaderMap) -> BaseLlmException {
        BaseLlmException {
            status_code,
            message: message.to_owned(),
            headers,
            status_code_is_synthesized: false,
        }
    }
}

impl ResponsesParamProvider for TestConfig {
    type Error = ResponsesParametersError;

    fn name(&self) -> &'static str {
        "openai"
    }

    fn supported_params(&self, _model: &str) -> Result<Option<Vec<String>>, Self::Error> {
        Ok(self.supported.clone())
    }

    fn map_params(
        &self,
        params: &Params,
        _model: &str,
        drop_params: bool,
    ) -> Result<Params, Self::Error> {
        self.dropped
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(drop_params);
        let Some(supported) = &self.supported else {
            return Ok(params.clone());
        };
        if !drop_params {
            return Ok(params.clone());
        }
        Ok(params
            .iter()
            .filter(|(name, _value)| supported.iter().any(|allowed| allowed == *name))
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect())
    }
}

impl CreateCapabilities for TestConfig {
    fn supports_native_file_search(&self) -> bool {
        self.native_file_search
    }

    fn should_fake_stream(
        &self,
        model: &str,
        stream: Option<bool>,
        custom_llm_provider: &str,
    ) -> bool {
        self.fake_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(FakeCall {
                model: model.to_owned(),
                stream,
                provider: custom_llm_provider.to_owned(),
            });
        model == "fake-stream" && stream == Some(true)
    }
}

pub(crate) fn round_trip<Wire, Body>(data: &Body) -> Result<Wire, ConfigFailure>
where
    Wire: DeserializeOwned,
    Body: Serialize + ?Sized,
{
    serde_json::to_value(data)
        .and_then(serde_json::from_value::<Wire>)
        .map_err(ConfigFailure::new)
}

pub(crate) struct Configs {
    pub(crate) events: Events,
    pub(crate) supported: Option<Vec<String>>,
    pub(crate) native_file_search: bool,
    pub(crate) present: bool,
    pub(crate) inputs: Arc<Mutex<Vec<ResponsesInput>>>,
    pub(crate) dropped: Arc<Mutex<Vec<bool>>>,
    pub(crate) fake_calls: Arc<Mutex<Vec<FakeCall>>>,
}

impl Configs {
    fn new(events: &Events) -> Self {
        Self {
            events: events.clone(),
            supported: None,
            native_file_search: true,
            present: true,
            inputs: Arc::default(),
            dropped: Arc::default(),
            fake_calls: Arc::default(),
        }
    }

    pub(crate) fn inputs(&self) -> Vec<ResponsesInput> {
        self.inputs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn dropped(&self) -> Vec<bool> {
        self.dropped
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn fake_calls(&self) -> Vec<FakeCall> {
        self.fake_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl CreateConfigFactory for Configs {
    type Provider = TestConfig;

    fn create_config(&self, request: &ConfigRequest<'_>) -> Option<TestConfig> {
        if !self.present || request.provider != "openai" {
            return None;
        }
        self.events.push("config");
        Some(TestConfig::new(
            &self.events,
            self.supported.clone(),
            self.native_file_search,
            self,
        ))
    }
}

pub(crate) struct Registry {
    pub(crate) encrypted: bool,
}

impl ConfigLookup for Registry {
    fn responses_api_config(
        &self,
        _model: &str,
        provider: &str,
        _api_base: Option<&str>,
    ) -> Option<NativeConfig> {
        (provider == "openai").then_some(NativeConfig {
            encrypted_agent_messages: self.encrypted,
        })
    }
}

#[derive(Default)]
pub(crate) struct ModelNames {
    seen: Mutex<Vec<String>>,
    pub(crate) deployment_api_key: Option<SecretString>,
    pub(crate) deployment_api_base: Option<String>,
}

impl ModelNames {
    pub(crate) fn seen(&self) -> Vec<String> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ModelResolver for ModelNames {
    fn resolve_model(
        &self,
        request: &ResolutionRequest<'_>,
    ) -> Result<ResolvedModel, ResolutionError> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.model.to_owned());
        let (model, provider) = if let Some(given) = request.custom_llm_provider {
            let model = match request.model.split_once('/') {
                Some((prefix, rest)) if prefix == given => rest.to_owned(),
                Some(_) | None => request.model.to_owned(),
            };
            (model, given.to_owned())
        } else {
            let Some((prefix, rest)) = request.model.split_once('/') else {
                return Err(ResolutionError::MissingProvider {
                    model: request.model.to_owned(),
                });
            };
            (rest.to_owned(), prefix.to_owned())
        };
        let api_key = if request.api_key.is_none() {
            self.deployment_api_key.clone()
        } else {
            None
        };
        let api_base = if request.api_base.is_none() {
            self.deployment_api_base.clone()
        } else {
            None
        };
        Ok(ResolvedModel {
            model,
            provider: ProviderIdentity::Custom(provider),
            api_base,
            api_key,
        })
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the test double does not expect this call")]
pub(crate) struct HookFailure;

pub(crate) struct Nothing;

#[async_trait]
impl DeploymentHookDispatcher for Nothing {
    type Error = HookFailure;

    async fn pre_call_deployment_hook(
        &self,
        _request: IndexMap<String, Json>,
        _call_type: &str,
    ) -> Result<Option<IndexMap<String, Json>>, Self::Error> {
        Ok(None)
    }
}

impl NamedCallbackResolver for Nothing {
    fn resolve(&self, _name: &str) -> Option<PolicyCallback> {
        None
    }
}

#[async_trait]
impl ContinuationExecutor for Nothing {
    type Error = HookFailure;

    async fn responses(&self, _call: &ResponsesContinuation) -> Result<Box<RawValue>, Self::Error> {
        Err(HookFailure)
    }

    async fn anthropic_messages(
        &self,
        _call: &AnthropicContinuation,
    ) -> Result<Box<RawValue>, Self::Error> {
        Err(HookFailure)
    }
}

pub(crate) struct NoInterception;

#[async_trait]
impl CreateInterception<()> for NoInterception {
    fn mcp_requested(&self, _tools: Option<&[JsonValue]>) -> bool {
        false
    }

    async fn mcp(&self, _args: &DispatchArgs<'_>, context: ()) -> InterceptionResult<()> {
        Ok(Interception::Continue(context))
    }

    async fn file_search(&self, _args: &FileSearchArgs<'_>, context: ()) -> InterceptionResult<()> {
        Ok(Interception::Continue(context))
    }
}

#[derive(Debug, Clone)]
pub(crate) struct DispatchRecord {
    pub(crate) input: Json,
    pub(crate) reasoning: Option<Json>,
    pub(crate) tools: Option<Vec<JsonValue>>,
    pub(crate) kwargs: ParameterMap,
    pub(crate) extra_headers: Option<Headers>,
    pub(crate) service_tier: Option<String>,
    pub(crate) safety_identifier: Option<String>,
    pub(crate) text_format: Option<Json>,
    pub(crate) allowed_openai_params: Option<Vec<String>>,
    pub(crate) use_chat_completions_api: bool,
}

impl From<&DispatchArgs<'_>> for DispatchRecord {
    fn from(args: &DispatchArgs<'_>) -> Self {
        Self {
            input: transcode(args.input).expect("the dispatch input converts"),
            reasoning: args.reasoning.cloned(),
            tools: args.tools.map(<[JsonValue]>::to_vec),
            kwargs: args.kwargs.clone(),
            extra_headers: args.extra_headers.cloned(),
            service_tier: None,
            safety_identifier: None,
            text_format: None,
            allowed_openai_params: None,
            use_chat_completions_api: false,
        }
    }
}

impl From<&FileSearchArgs<'_>> for DispatchRecord {
    fn from(args: &FileSearchArgs<'_>) -> Self {
        let mut record = Self::from(&args.call);
        record.service_tier = args.service_tier.map(str::to_owned);
        record.safety_identifier = args.safety_identifier.map(str::to_owned);
        record.text_format = args.text_format.cloned();
        record.allowed_openai_params = args.allowed_openai_params.map(<[String]>::to_vec);
        record.use_chat_completions_api = args.use_chat_completions_api;
        record
    }
}

// The gateway double records what each dispatch received and either answers it or passes it on
pub(crate) struct Gateway {
    pub(crate) events: Events,
    pub(crate) mcp_requested: bool,
    pub(crate) handle: bool,
    pub(crate) mcp: Mutex<Option<DispatchRecord>>,
    pub(crate) file_search: Mutex<Option<DispatchRecord>>,
}

impl Gateway {
    pub(crate) fn requesting(events: &Events) -> Self {
        Self {
            events: events.clone(),
            mcp_requested: true,
            handle: true,
            mcp: Mutex::new(None),
            file_search: Mutex::new(None),
        }
    }

    pub(crate) fn requesting_and_passing(events: &Events) -> Self {
        Self {
            mcp_requested: true,
            handle: false,
            ..Self::requesting(events)
        }
    }

    pub(crate) fn passing(events: &Events) -> Self {
        Self {
            mcp_requested: false,
            handle: false,
            ..Self::requesting(events)
        }
    }

    pub(crate) fn answering_file_search(events: &Events) -> Self {
        Self {
            mcp_requested: false,
            handle: true,
            ..Self::requesting(events)
        }
    }

    pub(crate) fn mcp_record(&self) -> Option<DispatchRecord> {
        self.mcp
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn file_search_record(&self) -> Option<DispatchRecord> {
        self.file_search
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn intercepted(&self, label: &str, context: ()) -> InterceptionResult<()> {
        if !self.handle {
            return Ok(Interception::Continue(context));
        }
        handled(label)
            .map(|created| Interception::Handled(Box::new(created)))
            .map_err(|error| InterceptionError {
                message: error.to_string(),
            })
    }

    fn record_mcp(&self, args: &DispatchArgs<'_>) {
        self.events.push("mcp");
        record_into(&self.mcp, DispatchRecord::from(args));
    }

    fn record_file_search(&self, args: &FileSearchArgs<'_>) {
        self.events.push("file_search");
        record_into(&self.file_search, DispatchRecord::from(args));
    }
}

fn record_into<T>(slot: &Mutex<Option<T>>, value: T) {
    *slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(value);
}

#[async_trait]
impl CreateInterception<()> for Gateway {
    fn mcp_requested(&self, _tools: Option<&[JsonValue]>) -> bool {
        self.mcp_requested
    }

    async fn mcp(&self, args: &DispatchArgs<'_>, context: ()) -> InterceptionResult<()> {
        self.record_mcp(args);
        self.intercepted("mcp handled", context)
    }

    async fn file_search(&self, args: &FileSearchArgs<'_>, context: ()) -> InterceptionResult<()> {
        self.record_file_search(args);
        self.intercepted("file search handled", context)
    }
}

fn handled(text: &str) -> Result<Created<()>, serde_json::Error> {
    Ok(Created::Responses(Box::new(StampedResponse {
        response: mock_responses_api_response(Some(text))?,
        hidden: IndexMap::default(),
    })))
}

#[derive(Debug, Clone)]
pub(crate) struct BridgeRecord {
    pub(crate) model: String,
    pub(crate) custom_llm_provider: Option<String>,
    pub(crate) is_async: bool,
    pub(crate) stream: Option<bool>,
    pub(crate) kwargs: ParameterMap,
    pub(crate) request: PromptJson,
    pub(crate) extra_headers: Option<Headers>,
    pub(crate) input: ResponsesInput,
}

#[derive(Default)]
pub(crate) struct RecordingBridge {
    pub(crate) events: Events,
    pub(crate) calls: Mutex<Vec<BridgeRecord>>,
}

impl RecordingBridge {
    pub(crate) fn new(events: &Events) -> Self {
        Self {
            events: events.clone(),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn calls(&self) -> Vec<BridgeRecord> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl ChatBridge<()> for RecordingBridge {
    async fn respond(
        &self,
        call: &BridgeCall<'_>,
        _context: (),
    ) -> Result<Created<()>, BridgeError> {
        self.events.push("bridge");
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(BridgeRecord {
                model: call.model.to_owned(),
                custom_llm_provider: call.custom_llm_provider.map(str::to_owned),
                is_async: call.is_async,
                stream: call.stream,
                kwargs: call.kwargs.clone(),
                request: PromptJson::Object(call.request.clone()),
                extra_headers: call.extra_headers.cloned(),
                input: call.input.clone(),
            });
        handled("bridged").map_err(|error| BridgeError {
            message: error.to_string(),
        })
    }
}

// The prompt step is observable by its mark; the hook itself stays out of this task's answer
pub(crate) struct PromptMark {
    pub(crate) events: Events,
}

impl PromptMark {
    pub(crate) fn new(events: &Events) -> Self {
        Self {
            events: events.clone(),
        }
    }
}

impl PromptManager for PromptMark {
    fn runs_prompt_hooks(&self, _request: &HookRequest<'_>) -> bool {
        self.events.push("prompt");
        false
    }

    fn get_chat_completion_prompt(
        &self,
        _request: &MergeRequest<'_>,
    ) -> Result<MergeAnswer, PromptManagerError> {
        Err(PromptManagerError {
            message: String::from("the test prompt manager does not merge"),
        })
    }
}

impl PromptSource for PromptMark {}

pub(crate) struct Rig {
    pub(crate) events: Events,
    pub(crate) wire: Wire,
    pub(crate) source: Source,
    pub(crate) resolver: ModelNames,
    pub(crate) lookup: Registry,
    pub(crate) configs: Configs,
    pub(crate) variables: BTreeMap<String, String>,
}

impl Rig {
    pub(crate) fn new() -> Self {
        Self::with_env(&[])
    }

    pub(crate) fn with_env(values: &[(&str, &str)]) -> Self {
        let events = Events::new();
        let wire = Wire::new(&events);
        let source = Source::new(&wire);
        let mut rig = Self {
            events: events.clone(),
            wire,
            source,
            resolver: ModelNames::default(),
            lookup: Registry { encrypted: true },
            configs: Configs::new(&events),
            variables: env_vars(&[("OPENAI_API_KEY", "sk-provider")]),
        };
        rig.variables.extend(env_vars(values));
        rig
    }
}

pub(crate) struct Seams<'seams> {
    pub(crate) rig: &'seams Rig,
    pub(crate) interception: &'seams dyn CreateInterception<()>,
    pub(crate) bridge: &'seams dyn ChatBridge<()>,
    pub(crate) prompts: Option<&'seams dyn PromptSource>,
    pub(crate) file_mapping: Option<&'seams FileIdMapping>,
    pub(crate) nothing: &'seams Nothing,
    pub(crate) drop_params: bool,
    pub(crate) reasoning_auto_summary: bool,
}

impl<'seams> Seams<'seams> {
    pub(crate) fn new(
        rig: &'seams Rig,
        nothing: &'seams Nothing,
        interception: &'seams dyn CreateInterception<()>,
        bridge: &'seams dyn ChatBridge<()>,
    ) -> Self {
        Self {
            rig,
            interception,
            bridge,
            prompts: None,
            file_mapping: None,
            nothing,
            drop_params: false,
            reasoning_auto_summary: false,
        }
    }

    pub(crate) fn deps(
        &self,
    ) -> ResponsesCreateDeps<'seams, Source, Configs, Nothing, Nothing, ()> {
        ResponsesCreateDeps {
            resolver: &self.rig.resolver,
            lookup: &self.rig.lookup,
            configs: &self.rig.configs,
            source: &self.rig.source,
            supplied: None,
            environment: &self.rig.variables,
            prompts: self.prompts,
            file_mapping: self.file_mapping,
            hooks: CreateHooks {
                callbacks: CallbackLists {
                    global: &[],
                    dynamic: &[],
                },
                resolver: self.nothing,
                dispatcher: self.nothing,
                executor: self.nothing,
                logging: None,
            },
            interception: self.interception,
            bridge: self.bridge,
            drop_params: self.drop_params,
            reasoning_auto_summary: self.reasoning_auto_summary,
        }
    }
}
