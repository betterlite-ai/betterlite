use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, pending, ready};
use std::io;
use std::slice::from_ref;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_error::error::details::RateLimitDetails;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_error::rate_limit::RateLimitCategory;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_models::decisions::Json;
use betterlite_models::resolution::{ResolutionError, ResolutionRequest};
use betterlite_models::usage_sum::Usage;
use betterlite_openai::api::responses_shapes::websocket::{
    PresidioGuardrailCallback, ResponsesClientWebSocket, ResponsesWebSocketMessage,
    WsRequestDefaults,
};
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::value::RawValue;
use thiserror::Error;
use tokio::task::yield_now;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use crate::api::responses_provider_selection::provider_config::{
    ConfigLookup, NativeConfig, ResponsesApiConfig,
};
use crate::api::responses_stream_events::ProjectQuotaCallback;
use crate::api::responses_ws_frame_policy::InputGuardrail;
use crate::api::responses_ws_native::call::{
    NativeWsCall, NativeWsDeps, NativeWsProviderHandler, NativeWsResolution, ProviderWsRequest,
    WsResolved, responses_websocket,
};
use crate::api::responses_ws_native::logging::{WsCallUpdate, WsNativeLogging};
use crate::api::responses_ws_native::relay::RelaySetup;
use crate::api::responses_ws_native::relay::ResponsesWsRelay;
use crate::api::responses_ws_native::relay::backend::{BackendError, NativeWsBackend};
use crate::api::responses_ws_native::relay::forward::ForwardRelay as _;
use crate::cost::realtime::RealtimeBilling;

pub(super) type Event = IndexMap<String, Json>;
pub(super) type Received = Result<ResponsesWebSocketMessage, BackendError<TestError>>;
pub(super) type ReceiveScript = Mutex<VecDeque<Received>>;

#[derive(Debug, Error)]
#[error("{0}")]
pub(super) struct TestError(pub(super) String);

pub(super) fn lock<Value>(shared: &Mutex<Value>) -> MutexGuard<'_, Value> {
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(super) fn object(text: &str) -> Event {
    serde_json::from_str(text).unwrap()
}

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

pub(super) fn unmask_tokens(text: &str, pii_tokens: &IndexMap<String, String>) -> String {
    let mut unmasked = text.to_owned();
    for (token, original) in pii_tokens {
        unmasked = unmasked.replace(token.as_str(), original);
    }
    unmasked
}

pub(super) fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap()
}

pub(super) fn rate_limited(message: &str) -> ErrorClass {
    ErrorClass::RateLimit(Box::new(RateLimitDetails {
        context: ErrorContext {
            message: message.to_owned(),
            model: None,
            provider: None,
            max_retries: None,
            num_retries: None,
            body_type: None,
            body_param: None,
            body_code: None,
        },
        category: RateLimitCategory::LitellmRateLimit,
        dimension: None,
        headers: BTreeMap::new(),
    }))
}

#[derive(Default)]
pub(super) struct TestClient {
    pub(super) sent: Mutex<Vec<String>>,
    pub(super) incoming: Mutex<VecDeque<Result<String, TestError>>>,
    pub(super) fail_sends: bool,
    pub(super) yield_first: bool,
}

impl TestClient {
    pub(super) fn new(frames: Vec<&str>) -> Self {
        Self {
            sent: Mutex::new(Vec::new()),
            incoming: Mutex::new(
                frames
                    .into_iter()
                    .map(|frame| Ok(frame.to_owned()))
                    .collect(),
            ),
            fail_sends: false,
            yield_first: false,
        }
    }

    pub(super) fn sent(&self) -> Vec<String> {
        lock(&self.sent).clone()
    }
}

impl ResponsesClientWebSocket for TestClient {
    type Error = TestError;

    fn send_text<'call>(
        &'call self,
        data: &'call str,
    ) -> impl Future<Output = Result<(), TestError>> + Send + 'call {
        lock(&self.sent).push(data.to_owned());
        ready(if self.fail_sends {
            Err(TestError(String::from("the client socket is closed")))
        } else {
            Ok(())
        })
    }

    async fn receive_text(&self) -> Result<String, TestError> {
        if self.yield_first {
            yield_now().await;
        }
        lock(&self.incoming)
            .pop_front()
            .unwrap_or_else(|| Err(TestError(String::from("the client socket is closed"))))
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum ReceiveMode {
    #[default]
    Scripted,
    UnsupportedDecode,
    Blocked,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum CloseMode {
    #[default]
    Graceful,
    Fails,
}

#[derive(Default)]
pub(super) struct ScriptedBackend {
    pub(super) sent: Mutex<Vec<String>>,
    pub(super) recv_raw_script: ReceiveScript,
    pub(super) recv_script: ReceiveScript,
    pub(super) receive: ReceiveMode,
    pub(super) close: CloseMode,
}

impl ScriptedBackend {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn text(&self, frame: &str) -> &Self {
        lock(&self.recv_raw_script)
            .push_back(Ok(ResponsesWebSocketMessage::Text(frame.to_owned())));
        self
    }

    pub(super) fn bytes(&self, frame: &[u8]) -> &Self {
        lock(&self.recv_raw_script).push_back(Ok(ResponsesWebSocketMessage::Bytes(frame.to_vec())));
        self
    }

    pub(super) fn closing(&self) -> &Self {
        lock(&self.recv_raw_script).push_back(Err(BackendError::Closed(TestError(String::from(
            "the backend connection closed",
        )))));
        self
    }

    pub(super) fn failing(&self, message: &str) -> &Self {
        lock(&self.recv_raw_script)
            .push_back(Err(BackendError::Failed(TestError(message.to_owned()))));
        self
    }

    pub(super) fn sent(&self) -> Vec<String> {
        lock(&self.sent).clone()
    }
}

const CLOSED: &str = "the backend connection closed";

fn next_message(script: &ReceiveScript) -> Received {
    lock(script)
        .pop_front()
        .unwrap_or_else(|| Err(BackendError::Closed(TestError(String::from(CLOSED)))))
}

async fn frame(mode: ReceiveMode, script: &ReceiveScript, raw: bool) -> Received {
    match (mode, raw) {
        (ReceiveMode::Blocked, _) => pending().await,
        (ReceiveMode::UnsupportedDecode, true) => Err(BackendError::UnsupportedDecode),
        (ReceiveMode::UnsupportedDecode | ReceiveMode::Scripted, _) => next_message(script),
    }
}

impl NativeWsBackend for ScriptedBackend {
    type Error = TestError;

    fn recv_raw(&self) -> impl Future<Output = Received> + Send + '_ {
        frame(self.receive, &self.recv_raw_script, true)
    }

    fn recv(&self) -> impl Future<Output = Received> + Send + '_ {
        frame(self.receive, &self.recv_script, false)
    }

    fn send<'call>(
        &'call self,
        message: &'call str,
    ) -> impl Future<Output = Result<(), TestError>> + Send + 'call {
        lock(&self.sent).push(message.to_owned());
        ready(Ok(()))
    }

    fn close(&self) -> impl Future<Output = Result<(), TestError>> + Send + '_ {
        ready(match self.close {
            CloseMode::Graceful => Ok(()),
            CloseMode::Fails => Err(TestError(String::from("the backend close failed"))),
        })
    }
}

pub(super) struct EchoGuard;

impl PresidioGuardrailCallback for EchoGuard {
    type Error = TestError;
    type Settings = ();

    fn presidio_settings(&self, _data: &BTreeMap<String, Box<RawValue>>) -> Option<()> {
        Some(())
    }

    fn check_pii<'call>(
        &'call self,
        text: &'call str,
        _output_parse_pii: bool,
        _presidio_config: Option<&'call ()>,
        _request_data: &'call BTreeMap<String, Box<RawValue>>,
    ) -> impl Future<Output = Result<String, TestError>> + Send + 'call {
        ready(Ok(text.to_owned()))
    }
}

pub(super) struct RejectFirstQuota {
    pub(super) calls: Mutex<usize>,
    pub(super) message: String,
}

impl RejectFirstQuota {
    pub(super) fn new(message: &str) -> Self {
        Self {
            calls: Mutex::new(0),
            message: message.to_owned(),
        }
    }
}

#[async_trait]
impl ProjectQuotaCallback for RejectFirstQuota {
    async fn enforce_frame_quota(
        &self,
        _user_api_key_dict: Option<&UserAPIKeyAuth>,
        _requested_model: Option<&str>,
        _estimated_input_tokens: i64,
        _estimated_output_tokens: i64,
    ) -> Result<(), ErrorClass> {
        let mut calls = lock(&self.calls);
        *calls = calls.saturating_add(1);
        if *calls == 1 {
            Err(rate_limited(&self.message))
        } else {
            Ok(())
        }
    }
}

#[derive(Clone)]
pub(super) struct RecordedUpdate {
    pub(super) model: String,
    pub(super) user: Option<String>,
    pub(super) provider: Option<String>,
    pub(super) litellm_params: Event,
    pub(super) kwargs: Event,
}

#[derive(Default)]
pub(super) struct RecordingLogging {
    pub(super) effects: Mutex<Vec<String>>,
    pub(super) updates: Mutex<Vec<RecordedUpdate>>,
    pub(super) pre_calls: Mutex<Vec<(String, String)>>,
    pub(super) inputs: Mutex<Vec<Vec<Event>>>,
    pub(super) success: Mutex<Vec<Vec<Event>>>,
    pub(super) success_prefers_async: Mutex<Vec<bool>>,
    pub(super) partial: Mutex<Vec<(Usage, Decimal)>>,
    pub(super) charged: Mutex<Vec<(Option<String>, Option<Decimal>)>>,
    pub(super) failures: Mutex<Vec<ErrorClass>>,
    pub(super) failure_prefers_async: Mutex<Vec<bool>>,
    pub(super) tracebacks: Mutex<Vec<String>>,
    pub(super) reported: Option<Decimal>,
}

impl RecordingLogging {
    pub(super) fn effects(&self) -> Vec<String> {
        lock(&self.effects).clone()
    }
}

impl WsNativeLogging for RecordingLogging {
    fn update_from_kwargs(&self, update: WsCallUpdate<'_>) -> Result<(), LoggingStateError> {
        lock(&self.effects).push(String::from("update"));
        lock(&self.updates).push(RecordedUpdate {
            model: update.model.to_owned(),
            user: update.user.map(str::to_owned),
            provider: update.custom_llm_provider.map(str::to_owned),
            litellm_params: update.litellm_params.clone(),
            kwargs: update.kwargs.clone(),
        });
        Ok(())
    }

    fn pre_call(&self, input: &str, api_key: &str) {
        lock(&self.effects).push(String::from("pre_call"));
        lock(&self.pre_calls).push((input.to_owned(), api_key.to_owned()));
    }

    fn record_input_messages(&self, messages: &[Event]) {
        lock(&self.effects).push(String::from("input_messages"));
        lock(&self.inputs).push(messages.to_vec());
    }

    fn dispatch_success(&self, messages: Vec<Event>, prefer_async: bool) {
        lock(&self.effects).push(String::from("success"));
        lock(&self.success).push(messages);
        lock(&self.success_prefers_async).push(prefer_async);
    }

    fn response_cost(&self, billing: &RealtimeBilling) -> Option<Decimal> {
        lock(&self.effects).push(String::from("cost"));
        lock(&self.charged).push((billing.service_tier.clone(), self.reported));
        self.reported
    }

    fn record_partial_usage(&self, usage: &Usage, response_cost: Decimal) {
        lock(&self.effects).push(String::from("partial_usage"));
        lock(&self.partial).push((usage.clone(), response_cost));
    }

    fn dispatch_failure(&self, exception: ErrorClass, traceback: String, prefer_async: bool) {
        lock(&self.effects).push(String::from("failure"));
        lock(&self.failures).push(exception);
        lock(&self.failure_prefers_async).push(prefer_async);
        lock(&self.tracebacks).push(traceback);
    }
}

#[derive(Default)]
pub(super) struct FakeResolution {
    pub(super) model: String,
    pub(super) provider: Option<String>,
    pub(super) api_base: Option<String>,
    pub(super) api_key: Option<String>,
    pub(super) echo_request: bool,
}

impl FakeResolution {
    pub(super) fn openai() -> Self {
        Self::named(String::from("gpt-4o"), String::from("openai"))
    }

    fn named(model: String, provider: String) -> Self {
        Self {
            model,
            provider: Some(provider),
            api_base: None,
            api_key: None,
            echo_request: false,
        }
    }
}

impl NativeWsResolution for FakeResolution {
    fn resolve(&self, request: &ResolutionRequest<'_>) -> Result<WsResolved, ResolutionError> {
        if self.echo_request {
            return Ok(WsResolved {
                model: request.model.to_owned(),
                provider: self.provider.clone(),
                api_base: request.api_base.map(str::to_owned),
                api_key: request.api_key.cloned(),
            });
        }
        Ok(WsResolved {
            model: self.model.clone(),
            provider: self.provider.clone(),
            api_base: self.api_base.clone(),
            api_key: self.api_key.clone().map(SecretString::from),
        })
    }
}

#[derive(Default)]
pub(super) struct FakeConfigs {
    pub(super) native: bool,
    pub(super) seen: Mutex<Vec<(String, String, Option<String>)>>,
}

impl ConfigLookup for FakeConfigs {
    fn responses_api_config(
        &self,
        model: &str,
        provider: &str,
        api_base: Option<&str>,
    ) -> Option<NativeConfig> {
        lock(&self.seen).push((
            model.to_owned(),
            provider.to_owned(),
            api_base.map(String::from),
        ));
        self.native.then_some(NativeConfig {
            encrypted_agent_messages: true,
        })
    }
}

#[derive(Clone)]
pub(super) struct RecordedRequest {
    pub(super) model: String,
    pub(super) api_base: Option<String>,
    pub(super) api_key: Option<String>,
    pub(super) timeout: Option<f64>,
    pub(super) first_message: Option<String>,
    pub(super) litellm_metadata: Event,
    pub(super) custom_llm_provider: Option<String>,
    pub(super) provider_config: Option<ResponsesApiConfig>,
    pub(super) remaining_kwargs: Event,
    pub(super) fill_missing: BTreeMap<String, Json>,
    pub(super) overrides: BTreeMap<String, Json>,
}

pub(super) struct RecordingHandler {
    pub(super) requests: Mutex<Vec<RecordedRequest>>,
    pub(super) forward: bool,
    pub(super) backend: Arc<ScriptedBackend>,
    pub(super) logging: Arc<RecordingLogging>,
    pub(super) outputs: Vec<EchoGuard>,
    pub(super) quota: Option<Arc<dyn ProjectQuotaCallback>>,
    pub(super) authorized_model: Option<String>,
    pub(super) request_data: Option<Event>,
    pub(super) redactor: Redactor,
}

impl RecordingHandler {
    pub(super) fn recording_only() -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            forward: false,
            backend: Arc::new(ScriptedBackend::new()),
            logging: Arc::new(RecordingLogging::default()),
            outputs: Vec::new(),
            quota: None,
            authorized_model: None,
            request_data: None,
            redactor: redactor(),
        }
    }

    pub(super) fn forwarding(
        backend: Arc<ScriptedBackend>,
        logging: Arc<RecordingLogging>,
    ) -> Self {
        Self {
            forward: true,
            backend,
            logging,
            ..Self::recording_only()
        }
    }

    pub(super) fn request(&self, index: usize) -> RecordedRequest {
        lock(&self.requests)[index].clone()
    }
}

fn record_request(request: &ProviderWsRequest<'_, TestClient>) -> RecordedRequest {
    RecordedRequest {
        model: request.model.to_owned(),
        api_base: request.api_base.map(str::to_owned),
        api_key: request.api_key.map(|key| key.expose_secret().to_owned()),
        timeout: request.timeout,
        first_message: request.first_message.clone(),
        litellm_metadata: request.litellm_metadata.clone(),
        custom_llm_provider: request.custom_llm_provider.map(str::to_owned),
        provider_config: request.provider_config,
        remaining_kwargs: request.remaining_kwargs.clone(),
        fill_missing: request.request_defaults.fill_missing.clone(),
        overrides: request.request_defaults.overrides.clone(),
    }
}

impl NativeWsProviderHandler<TestClient> for RecordingHandler {
    type Error = TestError;

    async fn responses_websocket<'call>(
        &'call self,
        request: ProviderWsRequest<'call, TestClient>,
    ) -> Result<Option<ErrorClass>, TestError> {
        lock(&self.requests).push(record_request(&request));
        if !self.forward {
            return Ok(None);
        }
        let logging: &dyn WsNativeLogging = &*self.logging;
        let relay = ResponsesWsRelay::new(RelaySetup {
            websocket: request.websocket,
            backend: self.backend.as_ref(),
            logging: Some(logging),
            user_api_key_dict: request.user_api_key_dict,
            request_data: self.request_data.clone(),
            first_message: request.first_message.clone(),
            guardrail_callbacks: None,
            output_guardrail_callbacks: if self.outputs.is_empty() {
                None
            } else {
                Some(self.outputs.as_slice())
            },
            quota_callbacks: self.quota.as_ref().map(from_ref),
            authorized_model: self.authorized_model.as_deref(),
            custom_llm_provider: request.custom_llm_provider,
            request_defaults: Some(&request.request_defaults),
            redactor: &self.redactor,
        });
        Ok(relay.bidirectional_forward().await)
    }
}

#[derive(Default)]
pub(super) struct Entry {
    pub(super) model: String,
    pub(super) api_base: Option<String>,
    pub(super) api_key: Option<String>,
    pub(super) timeout: Option<f64>,
    pub(super) kwargs: Event,
    pub(super) resolution: FakeResolution,
    pub(super) globals: LiteLlmGlobals,
    pub(super) openai_api_key: Option<String>,
    pub(super) configs: FakeConfigs,
    pub(super) logging: Arc<RecordingLogging>,
}

impl Entry {
    pub(super) fn new() -> Self {
        let model = String::from("gpt-4o");
        let api_base = None;
        let api_key = None;
        let timeout = None;
        let resolution = FakeResolution::openai();
        Self {
            model,
            api_base,
            api_key,
            timeout,
            kwargs: Event::new(),
            resolution,
            globals: LiteLlmGlobals::default(),
            openai_api_key: None,
            configs: FakeConfigs::default(),
            logging: Arc::new(RecordingLogging::default()),
        }
    }

    pub(super) async fn request(&self) -> RecordedRequest {
        let handler = RecordingHandler::recording_only();
        let client = TestClient::new(Vec::new());
        let openai = self.openai_api_key.as_deref().map(SecretString::from);
        let caller_key = self.api_key.as_deref().map(SecretString::from);
        let deps = NativeWsDeps {
            resolution: &self.resolution,
            configs: &self.configs,
            logging: &*self.logging,
            globals: &self.globals,
            openai_api_key: openai.as_ref(),
            auto_summary: false,
            handler: &handler,
        };
        let call = NativeWsCall {
            model: &self.model,
            websocket: &client,
            api_base: self.api_base.as_deref(),
            api_key: caller_key.as_ref(),
            timeout: self.timeout,
            user_api_key_dict: None,
            kwargs: &self.kwargs,
        };
        responses_websocket(&deps, call).await.unwrap();
        handler.request(0)
    }
}

#[derive(Clone, Default)]
pub(super) struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    pub(super) fn text(&self) -> String {
        String::from_utf8(lock(&self.0).clone()).unwrap()
    }
}

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        lock(&self.0).extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for Captured {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

pub(super) struct RelayParts<'relay> {
    pub(super) websocket: &'relay TestClient,
    pub(super) backend: &'relay ScriptedBackend,
    pub(super) logging: &'relay RecordingLogging,
    pub(super) redactor: &'relay Redactor,
    pub(super) first_message: Option<String>,
    pub(super) request_data: Option<Event>,
    pub(super) guardrails: Option<&'relay [InputGuardrail<'relay, EchoGuard>]>,
    pub(super) outputs: Option<&'relay [EchoGuard]>,
    pub(super) quota: Option<&'relay [Arc<dyn ProjectQuotaCallback>]>,
    pub(super) authorized_model: Option<&'relay str>,
    pub(super) provider: Option<&'relay str>,
    pub(super) defaults: Option<&'relay WsRequestDefaults<Json>>,
}

impl<'relay> RelayParts<'relay> {
    pub(super) fn standard(
        websocket: &'relay TestClient,
        backend: &'relay ScriptedBackend,
        logging: &'relay RecordingLogging,
        redactor: &'relay Redactor,
    ) -> Self {
        Self {
            websocket,
            backend,
            logging,
            redactor,
            first_message: None,
            request_data: None,
            guardrails: None,
            outputs: None,
            quota: None,
            authorized_model: None,
            provider: None,
            defaults: None,
        }
    }

    pub(super) fn relay(self) -> ResponsesWsRelay<'relay, EchoGuard, TestClient, ScriptedBackend> {
        let logging: &dyn WsNativeLogging = self.logging;
        ResponsesWsRelay::new(RelaySetup {
            websocket: self.websocket,
            backend: self.backend,
            logging: Some(logging),
            user_api_key_dict: None,
            request_data: self.request_data,
            first_message: self.first_message,
            guardrail_callbacks: self.guardrails,
            output_guardrail_callbacks: self.outputs,
            quota_callbacks: self.quota,
            authorized_model: self.authorized_model,
            custom_llm_provider: self.provider,
            request_defaults: self.defaults,
            redactor: self.redactor,
        })
    }
}

pub(super) fn capture_traces() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(captured.clone())
        .finish();
    let guard = set_default(subscriber);
    (captured, guard)
}
