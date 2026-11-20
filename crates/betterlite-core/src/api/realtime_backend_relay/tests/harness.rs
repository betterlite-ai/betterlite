use std::collections::VecDeque;
use std::convert::Infallible;
use std::fmt::Debug;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use betterlite_config::api::realtime_session_state::LoggedEventTypes;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_models::decisions::Json;
use betterlite_openai::realtime::normalizer::RealtimeEventNormalizer;
use futures_util::future::BoxFuture;
use indexmap::IndexMap;
use tokio_util::sync::CancellationToken;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use crate::api::realtime_backend_relay::RealtimeBackendRelay;
use crate::api::realtime_backend_relay::error::RealtimeRelayError;
use crate::api::realtime_backend_relay::relay::BackendToClientRelay as _;
use crate::api::realtime_backend_relay::relay_callbacks;
use crate::api::realtime_backend_relay::source::BackendRefusal;
use crate::api::realtime_backend_relay::source::RealtimeBackendSource;
use crate::api::realtime_backend_relay::source::RealtimeFailureDispatch;
use crate::api::realtime_backend_relay::source::RelayReceiveError;
use crate::api::realtime_contracts::close::BackendClose;
use crate::api::realtime_contracts::close::CloseFrame;
use crate::api::realtime_contracts::close::ConnectionClosed;
use crate::api::realtime_contracts::config::BaseRealtimeConfig;
use crate::api::realtime_contracts::config::Environment;
use crate::api::realtime_contracts::error::HeaderMap;
use crate::api::realtime_contracts::error::WebSocketError;
use crate::api::realtime_contracts::events::AllDeltaType;
use crate::api::realtime_contracts::events::AudioTranscriptionUsage;
use crate::api::realtime_contracts::events::DurationUsage;
use crate::api::realtime_contracts::events::RealtimeResponseTransformInput;
use crate::api::realtime_contracts::events::RealtimeResponseTypedDict;
use crate::api::realtime_contracts::transport::ClientWebSocket;
use crate::api::realtime_contracts::transport::RealtimeBackend;
use crate::api::realtime_contracts::transport::RealtimeMessage;
use crate::api::realtime_contracts::transport::ScopedWebSocket;
use crate::api::realtime_contracts::transport::WebSocketScope;
use crate::api::realtime_guardrail_policy::RealtimeGuardrailCallback;
use crate::api::realtime_guardrails::callback::GuardrailFailure;
use crate::api::realtime_guardrails::callback::GuardrailInput;
use crate::api::realtime_guardrails::callback::GuardrailTexts;
use crate::api::realtime_guardrails::callback::GuardrailViolation;
use crate::api::realtime_guardrails::callback::RealtimeGuardrail;
use crate::api::realtime_guardrails::callback::RealtimeGuardrailPolicy;
use crate::api::realtime_session_state::ConnectionSetup;
use crate::api::realtime_session_state::RealtimeProviderConfig;
use crate::api::realtime_session_state::RealtimeStreaming;
use crate::api::realtime_session_state::logging::RealtimeLogging;
use crate::api::realtime_session_state::logging::RealtimeSuccessQueue;
use crate::api::realtime_session_state::logging::SessionInput;
use crate::api::realtime_session_state::logging::SuccessWork;
use crate::api::request_destination::RequestData;

pub(super) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("json value")
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Flag(pub bool);

#[derive(Clone, Default)]
pub(super) struct SendLog(Arc<Mutex<SendData>>);

#[derive(Default)]
struct SendData {
    frames: Vec<String>,
    attempts: usize,
    failures: Vec<usize>,
}

impl SendLog {
    pub(super) fn failing(indices: Vec<usize>) -> Self {
        let log = Self::default();
        lock(&log.0).failures = indices;
        log
    }

    fn begin(&self) -> Result<(), WebSocketError> {
        let (attempt, refused) = {
            let mut data = lock(&self.0);
            let attempt = data.attempts;
            data.attempts = data.attempts.saturating_add(1);
            (attempt, data.failures.contains(&attempt))
        };
        if refused {
            return Err(WebSocketError {
                message: format!("the scripted send {attempt} failed"),
            });
        }
        Ok(())
    }

    fn push(&self, frame: String) {
        lock(&self.0).frames.push(frame);
    }

    pub(super) fn frames(&self) -> Vec<String> {
        lock(&self.0).frames.clone()
    }

    pub(super) fn parsed(&self) -> Vec<Json> {
        self.frames().iter().map(|text| json(text)).collect()
    }
}

pub(super) struct MockBackend {
    pub(super) log: SendLog,
}

#[async_trait]
impl RealtimeBackend for MockBackend {
    async fn send(&mut self, message: RealtimeMessage) -> Result<(), WebSocketError> {
        let frame = match message {
            RealtimeMessage::Text(text) => text,
            RealtimeMessage::Binary(bytes) => format!("binary:{}", bytes.len()),
        };
        self.log.push(frame);
        Ok(())
    }

    async fn recv(&mut self, _decode: Option<bool>) -> Result<RealtimeMessage, WebSocketError> {
        Err(WebSocketError {
            message: String::from("the scripted backend does not read"),
        })
    }

    async fn close(&mut self) -> Result<(), WebSocketError> {
        Ok(())
    }
}

pub(super) struct MockClient {
    pub(super) log: SendLog,
    pub(super) scope: WebSocketScope,
}

#[async_trait]
impl ClientWebSocket for MockClient {
    async fn send_text(&mut self, data: &str) -> Result<(), WebSocketError> {
        self.log.begin()?;
        self.log.push(data.to_owned());
        Ok(())
    }

    async fn receive_text(&mut self) -> Result<String, WebSocketError> {
        Err(WebSocketError {
            message: String::from("the scripted client does not read"),
        })
    }

    async fn close(&mut self, _code: u16, _reason: Option<&str>) -> Result<(), WebSocketError> {
        Ok(())
    }
}

impl ScopedWebSocket for MockClient {
    fn scope(&self) -> &WebSocketScope {
        &self.scope
    }
}

#[derive(Clone)]
pub(super) struct ScriptedSource(Arc<Mutex<SourceData>>);

#[derive(Default)]
pub(super) struct SourceData {
    pub(super) outcomes: VecDeque<Result<RealtimeMessage, RelayReceiveError>>,
    pub(super) reject_decode: Vec<usize>,
    pub(super) failures: Vec<usize>,
    pub(super) attempts: usize,
    pub(super) decode_calls: Vec<Option<bool>>,
}

impl Default for ScriptedSource {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(SourceData::default())))
    }
}

impl ScriptedSource {
    pub(super) fn push(&self, outcome: Result<RealtimeMessage, RelayReceiveError>) {
        lock(&self.0).outcomes.push_back(outcome);
    }

    pub(super) fn text(&self, text: &str) {
        self.push(Ok(RealtimeMessage::Text(text.to_owned())));
    }

    pub(super) fn binary(&self, bytes: Vec<u8>) {
        self.push(Ok(RealtimeMessage::Binary(bytes)));
    }

    pub(super) fn close(&self, frame: Option<CloseFrame>) {
        let message = frame.as_ref().map_or_else(
            || String::from("the scripted backend closed without a frame"),
            |close| close.reason.clone(),
        );
        self.push(Err(RelayReceiveError::Closed(ConnectionClosed {
            message,
            received: frame,
        })));
    }

    pub(super) fn close_with(&self, code: u16, reason: &str) {
        self.close(Some(CloseFrame {
            code,
            reason: reason.to_owned(),
        }));
    }

    pub(super) fn fail_receive(&self, index: usize) {
        lock(&self.0).failures.push(index);
    }

    pub(super) fn reject_decode(&self, index: usize) {
        lock(&self.0).reject_decode.push(index);
    }

    pub(super) fn decode_calls(&self) -> Vec<Option<bool>> {
        lock(&self.0).decode_calls.clone()
    }
}

#[async_trait]
impl RealtimeBackendSource for ScriptedSource {
    async fn recv(&mut self, decode: Option<bool>) -> Result<RealtimeMessage, RelayReceiveError> {
        let mut data = lock(&self.0);
        data.decode_calls.push(decode);
        let attempt = data.attempts;
        data.attempts = data.attempts.saturating_add(1);
        if decode == Some(false) && data.reject_decode.contains(&attempt) {
            return Err(RelayReceiveError::UnsupportedDecode);
        }
        if data.failures.contains(&attempt) {
            return Err(RelayReceiveError::Failed(WebSocketError {
                message: format!("the scripted receive {attempt} failed"),
            }));
        }
        data.outcomes.pop_front().map_or_else(
            || {
                Err(RelayReceiveError::Closed(ConnectionClosed {
                    received: None,
                    message: String::from("the scripted source has no frames left"),
                }))
            },
            Ok,
        )
    }
}

#[derive(Debug, thiserror::Error)]
pub(super) enum ScriptedFailure {
    #[error("the scripted provider transformation failed")]
    Transform,
    #[error("the scripted provider request failed")]
    Request,
}

pub(super) struct ScriptedResponse {
    pub(super) fail: bool,
    pub(super) response: Json,
    pub(super) current_output_item_id: Option<String>,
    pub(super) current_response_id: Option<String>,
    pub(super) current_delta_chunks: Option<Vec<Json>>,
    pub(super) current_conversation_id: Option<String>,
    pub(super) current_item_chunks: Option<Vec<Json>>,
    pub(super) current_delta_type: Option<AllDeltaType>,
    pub(super) session_configuration_request: Option<String>,
}

impl ScriptedResponse {
    pub(super) fn new(response: Json) -> Self {
        Self {
            fail: false,
            response,
            current_output_item_id: None,
            current_response_id: None,
            current_delta_chunks: None,
            current_conversation_id: None,
            current_item_chunks: None,
            current_delta_type: None,
            session_configuration_request: None,
        }
    }

    #[must_use]
    pub(super) fn output_item(mut self, id: &str) -> Self {
        self.current_output_item_id = Some(String::from(id));
        self
    }

    #[must_use]
    pub(super) fn response_id(mut self, id: &str) -> Self {
        self.current_response_id = Some(String::from(id));
        self
    }

    #[must_use]
    pub(super) fn conversation_id(mut self, id: &str) -> Self {
        self.current_conversation_id = Some(String::from(id));
        self
    }

    #[must_use]
    pub(super) fn delta_type(mut self, delta_type: AllDeltaType) -> Self {
        self.current_delta_type = Some(delta_type);
        self
    }

    #[must_use]
    pub(super) fn session_configuration_request(mut self, request: &str) -> Self {
        self.session_configuration_request = Some(String::from(request));
        self
    }
}

pub(super) fn message_types(messages: &[IndexMap<String, Json>]) -> Vec<String> {
    let frames: Vec<Json> = messages
        .iter()
        .map(|record| Json::Object(record.clone()))
        .collect();
    json_types(&frames)
}

pub(super) fn json_types(frames: &[Json]) -> Vec<String> {
    let mut types = Vec::new();
    for frame in frames {
        if let Json::Object(fields) = frame
            && let Some(Json::Text(kind)) = fields.get("type")
        {
            types.push(kind.clone());
        }
    }
    types
}

pub(super) const COMPLETED: &str = "conversation.item.input_audio_transcription.completed";
pub(super) const COMPLETED_FRAME: &str = r#"{"type":"conversation.item.input_audio_transcription.completed","transcript":"hello","item_id":"i","usage":{"type":"duration","seconds":1}}"#;
pub(super) const SESSION_CREATED: &str = r#"{"type":"session.created","session":{"id":"s1"}}"#;
pub(super) const DISABLE: &str = r#"{"type":"session.update","session":{"type":"realtime","audio":{"input":{"turn_detection":{"type":"server_vad","create_response":false}}}}}"#;

pub(super) fn completed_json() -> Json {
    json(COMPLETED_FRAME)
}

#[derive(Default)]
pub(super) struct ProviderScript {
    pub(super) responses: VecDeque<ScriptedResponse>,
    pub(super) inputs: Vec<RealtimeResponseTransformInput>,
    pub(super) requests: Vec<String>,
    pub(super) request_failures: Vec<usize>,
    pub(super) request_attempts: usize,
}

#[derive(Clone, Default)]
pub(super) struct ScriptedProvider {
    pub(super) script: Arc<Mutex<ProviderScript>>,
    pub(super) requires_session_configuration: bool,
    pub(super) unbilled_seconds: Option<u64>,
}

#[async_trait]
impl BaseRealtimeConfig for ScriptedProvider {
    type Error = ScriptedFailure;
    type Backend = MockBackend;

    fn validate_environment(
        &self,
        _environment: &Environment<'_>,
    ) -> Result<HeaderMap, ScriptedFailure> {
        Ok(HeaderMap::new())
    }

    fn transform_realtime_request(
        &self,
        message: &str,
        _model: &str,
        _session_configuration_request: Option<&str>,
    ) -> Result<Vec<RealtimeMessage>, ScriptedFailure> {
        let refused = {
            let mut script = lock(&self.script);
            let attempt = script.request_attempts;
            script.request_attempts = script.request_attempts.saturating_add(1);
            script.requests.push(message.to_owned());
            script.request_failures.contains(&attempt)
        };
        if refused {
            return Err(ScriptedFailure::Request);
        }
        Ok(vec![RealtimeMessage::Text(message.to_owned())])
    }

    fn transform_realtime_response(
        &self,
        _message: &RealtimeMessage,
        _model: &str,
        input: &RealtimeResponseTransformInput,
    ) -> Result<RealtimeResponseTypedDict, ScriptedFailure> {
        let popped = {
            let mut script = lock(&self.script);
            script.inputs.push(input.clone());
            script.responses.pop_front()
        };
        let Some(next) = popped else {
            return Err(ScriptedFailure::Transform);
        };
        if next.fail {
            return Err(ScriptedFailure::Transform);
        }
        Ok(RealtimeResponseTypedDict {
            response: next.response,
            current_output_item_id: next.current_output_item_id,
            current_response_id: next.current_response_id,
            current_delta_chunks: next.current_delta_chunks,
            current_conversation_id: next.current_conversation_id,
            current_item_chunks: next.current_item_chunks,
            current_delta_type: next.current_delta_type,
            session_configuration_request: next.session_configuration_request,
        })
    }
}

impl RealtimeProviderConfig for ScriptedProvider {
    fn requires_session_configuration(&self) -> bool {
        self.requires_session_configuration
    }

    fn unbilled_usage_on_close(&self, _model: &str) -> Option<AudioTranscriptionUsage> {
        self.unbilled_seconds.map(|seconds| {
            AudioTranscriptionUsage::Duration(DurationUsage {
                seconds: serde_json::Number::from(seconds),
            })
        })
    }
}

#[derive(Clone, Default)]
pub(super) struct RecordingLogging(Arc<Mutex<LoggingData>>);

#[derive(Default)]
pub(super) struct LoggingData {
    pub(super) details: Vec<(String, Json)>,
    pub(super) dispatches: usize,
    pub(super) pre_calls: usize,
}

impl RecordingLogging {
    pub(super) fn details(&self) -> Vec<(String, Json)> {
        lock(&self.0).details.clone()
    }

    pub(super) fn dispatches(&self) -> usize {
        lock(&self.0).dispatches
    }

    pub(super) fn detail(&self, key: &str) -> Option<Json> {
        lock(&self.0)
            .details
            .iter()
            .find(|(name, _value)| name == key)
            .map(|(_name, value)| value.clone())
    }
}

impl RealtimeLogging for RecordingLogging {
    fn pre_call(&self, _input: &SessionInput<'_>, _api_key: &str) {
        let mut data = lock(&self.0);
        data.pre_calls = data.pre_calls.saturating_add(1);
    }

    fn record_model_call_detail(&self, key: &str, value: Json) {
        lock(&self.0).details.push((key.to_owned(), value));
    }

    fn dispatch_success_handlers(&self, _messages: &[IndexMap<String, Json>]) -> SuccessWork {
        {
            let mut data = lock(&self.0);
            data.dispatches = data.dispatches.saturating_add(1);
        };
        Box::pin(async {})
    }
}

#[derive(Clone)]
pub(super) struct RecordingQueue {
    pub(super) enqueues: Arc<Mutex<usize>>,
    pub(super) accept: Arc<AtomicBool>,
}

impl Default for RecordingQueue {
    fn default() -> Self {
        Self {
            enqueues: Arc::new(Mutex::new(0)),
            accept: Arc::new(AtomicBool::new(true)),
        }
    }
}

impl RecordingQueue {
    pub(super) fn enqueues(&self) -> usize {
        *lock(&self.enqueues)
    }
}

impl RealtimeSuccessQueue for RecordingQueue {
    fn enqueue(&self, _work: SuccessWork) -> bool {
        {
            let mut count = lock(&self.enqueues);
            *count = count.saturating_add(1);
        };
        self.accept.load(Ordering::SeqCst)
    }
}

#[derive(Clone, Default)]
pub(super) struct RecordingDispatch(Arc<Mutex<DispatchData>>);

#[derive(Default)]
pub(super) struct DispatchData {
    pub(super) refusals: Vec<BackendRefusal>,
    pub(super) prefer_async: Vec<bool>,
}

impl RecordingDispatch {
    pub(super) fn refusals(&self) -> Vec<BackendRefusal> {
        lock(&self.0).refusals.clone()
    }

    pub(super) fn prefer_async(&self) -> Vec<bool> {
        lock(&self.0).prefer_async.clone()
    }
}

impl RealtimeFailureDispatch for RecordingDispatch {
    fn dispatch_failure_handlers(
        &self,
        refusal: &BackendRefusal,
        prefer_async_handlers: bool,
    ) -> SuccessWork {
        {
            let mut data = lock(&self.0);
            data.refusals.push(refusal.clone());
            data.prefer_async.push(prefer_async_handlers);
        };
        Box::pin(async {})
    }
}

#[derive(Clone, Default)]
pub(super) struct Normalizer {
    pub(super) drop_types: Vec<String>,
    pub(super) rename: Vec<(String, String)>,
}

pub(super) fn normalizer(drop_types: &[&str], rename: &[(&str, &str)]) -> Normalizer {
    let mut dropped = Vec::with_capacity(drop_types.len());
    for kind in drop_types {
        dropped.push(String::from(*kind));
    }
    let mut renamed = Vec::with_capacity(rename.len());
    for (from, to) in rename {
        renamed.push((String::from(*from), String::from(*to)));
    }
    Normalizer {
        drop_types: dropped,
        rename: renamed,
    }
}

pub(super) fn respond(fixture: &Fixture, response: Json) {
    lock(&fixture.provider.script)
        .responses
        .push_back(ScriptedResponse::new(response));
}

impl RealtimeEventNormalizer for Normalizer {
    fn should_drop(&self, event: &Json) -> bool {
        let Json::Object(fields) = event else {
            return false;
        };
        let Some(Json::Text(kind)) = fields.get("type") else {
            return false;
        };
        self.drop_types.iter().any(|dropped| dropped == kind)
    }

    fn normalize(&self, event: Json) -> Json {
        let Json::Object(mut fields) = event else {
            return event;
        };
        let renamed = match fields.get("type") {
            Some(Json::Text(kind)) => self
                .rename
                .iter()
                .find(|(from, _to)| from == kind)
                .map(|(_from, to)| to.clone()),
            Some(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
            )
            | None => None,
        };
        if let Some(replacement) = renamed {
            fields.insert("type".to_owned(), Json::Text(replacement));
        }
        Json::Object(fields)
    }

    fn patch_outgoing_session(&self, session: Json) -> Json {
        session
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum GuardrailOutcome {
    Clean,
    Block,
}

#[derive(Default)]
pub(super) struct GuardrailRecords {
    pub(super) checks: Vec<GuardrailEventHooks>,
    pub(super) applied: Vec<GuardrailInput>,
}

pub(super) struct ScriptedGuardrail {
    pub(super) outcome: GuardrailOutcome,
    pub(super) policy: RealtimeGuardrailPolicy,
    pub(super) records: Arc<Mutex<GuardrailRecords>>,
}

impl Default for ScriptedGuardrail {
    fn default() -> Self {
        Self {
            outcome: GuardrailOutcome::Clean,
            policy: RealtimeGuardrailPolicy::default(),
            records: Arc::new(Mutex::new(GuardrailRecords::default())),
        }
    }
}

impl ScriptedGuardrail {
    pub(super) fn blocking() -> Self {
        Self {
            outcome: GuardrailOutcome::Block,
            policy: RealtimeGuardrailPolicy::default(),
            records: Arc::new(Mutex::new(GuardrailRecords::default())),
        }
    }
}

impl RealtimeGuardrailCallback for ScriptedGuardrail {
    type Error = Infallible;

    fn is_custom_guardrail(&self) -> bool {
        true
    }

    fn should_run_guardrail(
        &self,
        _data: &RequestData,
        event: GuardrailEventHooks,
    ) -> Result<bool, Infallible> {
        lock(&self.records).checks.push(event);
        Ok(event == GuardrailEventHooks::RealtimeInputTranscription)
    }
}

impl RealtimeGuardrail for ScriptedGuardrail {
    fn use_native_lifecycle_hooks(&self) -> bool {
        false
    }

    fn policy(&self) -> &RealtimeGuardrailPolicy {
        &self.policy
    }

    fn apply_guardrail(
        &self,
        input: GuardrailInput,
    ) -> BoxFuture<'_, Result<GuardrailTexts, GuardrailFailure<Infallible>>> {
        lock(&self.records).applied.push(input);
        let outcome = self.outcome;
        Box::pin(async move {
            match outcome {
                GuardrailOutcome::Clean => Ok(GuardrailTexts::default()),
                GuardrailOutcome::Block => Err(GuardrailFailure::Blocked(GuardrailViolation {
                    detail: Json::Null,
                    message: String::from("denied"),
                })),
            }
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Sockets {
    Both,
    ClientOnly,
    BackendOnly,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Logging {
    Off,
    LoggerAndWorker,
}

pub(super) struct StateOptions {
    pub(super) sockets: Sockets,
    pub(super) logging: Logging,
    pub(super) provider: Flag,
    pub(super) normalizer: Option<Normalizer>,
    pub(super) client_wants_beta: Flag,
    pub(super) backend_uses_beta_protocol: Option<bool>,
    pub(super) force_transcription_model: Option<String>,
    pub(super) logged_event_types: LoggedEventTypes,
    pub(super) session_created_sent: Flag,
    pub(super) pending_messages: Vec<String>,
    pub(super) client_failures: Vec<usize>,
}

impl Default for StateOptions {
    fn default() -> Self {
        Self {
            sockets: Sockets::Both,
            logging: Logging::Off,
            provider: Flag(false),
            normalizer: None,
            client_wants_beta: Flag(false),
            backend_uses_beta_protocol: None,
            force_transcription_model: None,
            logged_event_types: LoggedEventTypes::Default,
            session_created_sent: Flag(false),
            pending_messages: Vec::new(),
            client_failures: Vec::new(),
        }
    }
}

impl StateOptions {
    #[must_use]
    pub(super) fn provider(mut self) -> Self {
        self.provider = Flag(true);
        self
    }

    #[must_use]
    pub(super) fn logging(mut self, logging: Logging) -> Self {
        self.logging = logging;
        self
    }

    #[must_use]
    pub(super) fn pending(mut self, messages: &[&str]) -> Self {
        self.pending_messages = messages.iter().map(|text| String::from(*text)).collect();
        self
    }

    #[must_use]
    pub(super) fn normalizer(mut self, normalizer: Normalizer) -> Self {
        self.normalizer = Some(normalizer);
        self
    }

    #[must_use]
    pub(super) fn client_wants_beta(mut self) -> Self {
        self.client_wants_beta = Flag(true);
        self
    }

    #[must_use]
    pub(super) fn force_transcription_model(mut self, model: &str) -> Self {
        self.force_transcription_model = Some(String::from(model));
        self
    }

    #[must_use]
    pub(super) fn logged_event_types(mut self, types: LoggedEventTypes) -> Self {
        self.logged_event_types = types;
        self
    }

    #[must_use]
    pub(super) fn client_failures(mut self, failures: Vec<usize>) -> Self {
        self.client_failures = failures;
        self
    }

    #[must_use]
    pub(super) fn sockets(mut self, sockets: Sockets) -> Self {
        self.sockets = sockets;
        self
    }
}

#[derive(Default)]
pub(super) struct Fixture {
    pub(super) client: SendLog,
    pub(super) backend: SendLog,
    pub(super) logging: RecordingLogging,
    pub(super) queue: RecordingQueue,
    pub(super) dispatch: RecordingDispatch,
    pub(super) provider: ScriptedProvider,
    pub(super) source: ScriptedSource,
}

impl Sockets {
    fn client(self, options: &StateOptions, fixture: &Fixture) -> Option<Box<dyn ClientWebSocket>> {
        let log = if options.client_failures.is_empty() {
            fixture.client.clone()
        } else {
            SendLog::failing(options.client_failures.clone())
        };
        match self {
            Self::Both | Self::ClientOnly => Some(Box::new(MockClient {
                log,
                scope: WebSocketScope::default(),
            })),
            Self::BackendOnly => None,
        }
    }

    fn backend(self, fixture: &Fixture) -> Option<Box<dyn RealtimeBackend>> {
        if self == Self::ClientOnly {
            return None;
        }
        Some(Box::new(MockBackend {
            log: fixture.backend.clone(),
        }))
    }
}

impl Logging {
    fn logger(self, fixture: &Fixture) -> Option<Box<dyn RealtimeLogging>> {
        match self {
            Self::Off => None,
            Self::LoggerAndWorker => Some(Box::new(fixture.logging.clone())),
        }
    }

    fn worker(self, fixture: &Fixture) -> Option<Box<dyn RealtimeSuccessQueue>> {
        match self {
            Self::Off => None,
            Self::LoggerAndWorker => Some(Box::new(fixture.queue.clone())),
        }
    }
}

pub(super) fn streaming(options: &StateOptions, fixture: &Fixture) -> RealtimeStreaming {
    let provider: Option<Box<dyn RealtimeProviderConfig>> = options
        .provider
        .0
        .then(|| Box::new(fixture.provider.clone()));
    let normalizer: Option<Box<dyn RealtimeEventNormalizer>> = options
        .normalizer
        .clone()
        .map(|normalizer| Box::new(normalizer));
    let mut state = RealtimeStreaming::new(ConnectionSetup {
        client: options.sockets.client(options, fixture),
        backend: options.sockets.backend(fixture),
        logging: options.logging.logger(fixture),
        logging_worker: options.logging.worker(fixture),
        provider_config: provider,
        event_normalizer: normalizer,
        logged_real_time_event_types: options.logged_event_types.clone(),
        model: String::from("gpt-realtime"),
        user_api_key_dict: None,
        request_data: Some(IndexMap::new()),
        force_transcription_model: options.force_transcription_model.clone(),
        client_wants_beta: options.client_wants_beta.0,
        backend_uses_beta_protocol: options.backend_uses_beta_protocol,
    });
    state.setup.session_created_sent_to_client.0 = options.session_created_sent.0;
    state.setup.pending_messages_until_setup = options.pending_messages.clone();
    state.setup.pending_messages_byte_total = options
        .pending_messages
        .iter()
        .map(String::len)
        .fold(0_usize, usize::saturating_add);
    state
}

pub(super) type RelayResult = Result<BackendClose, RealtimeRelayError<ScriptedFailure, Infallible>>;

// Every test drives the same relay loop; the arguments only choose the state's sockets, logging and the scripted provider
pub(super) async fn run(
    options: &StateOptions,
    fixture: &Fixture,
    source: &mut ScriptedSource,
    guardrails: &[&dyn RealtimeGuardrail<Error = Infallible>],
    token: &CancellationToken,
) -> (RelayResult, RealtimeStreaming) {
    let provider: Option<&ScriptedProvider> = options.provider.0.then_some(&fixture.provider);
    let mut state = streaming(options, fixture);
    let mut relay = RealtimeBackendRelay {
        state: &mut state,
        provider,
        guardrails,
        callbacks: relay_callbacks(guardrails),
        failure_dispatch: &fixture.dispatch,
    };
    let result = relay.backend_to_client_send_messages(source, token).await;
    (result, state)
}

#[derive(Clone, Default)]
pub(super) struct Captured(Arc<Mutex<Vec<(Level, String)>>>);
impl Captured {
    pub(super) fn messages(&self) -> Vec<String> {
        lock(&self.0)
            .iter()
            .map(|(_level, message)| message.clone())
            .collect()
    }

    pub(super) fn contains(&self, expected: &str) -> bool {
        self.messages().iter().any(|message| message == expected)
    }

    pub(super) fn starts_with(&self, prefix: &str) -> bool {
        self.messages()
            .iter()
            .any(|message| message.starts_with(prefix))
    }
}

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.0 = value.to_owned();
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Captured {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let mut message = Message::default();
        event.record(&mut message);
        if !message.0.is_empty() {
            lock(&self.0).push((*event.metadata().level(), message.0));
        }
    }
}

pub(super) fn capture() -> (Captured, DefaultGuard) {
    let logs = Captured::default();
    let subscriber = registry().with(logs.clone());
    let guard = set_default(subscriber);
    (logs, guard)
}
