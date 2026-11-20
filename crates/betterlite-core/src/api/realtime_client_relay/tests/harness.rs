use std::collections::VecDeque;
use std::convert::Infallible;
use std::fmt::Debug;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use betterlite_config::api::realtime_session_state::LoggedEventTypes;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_models::decisions::Json;
use betterlite_openai::realtime::normalizer::RealtimeEventNormalizer;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use futures_util::future::BoxFuture;
use indexmap::IndexMap;
use tokio_util::sync::CancellationToken;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, Subscriber, set_default};
use tracing::{Event, Level};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use crate::api::realtime_backend_relay::relay_callbacks;
use crate::api::realtime_client_relay::RealtimeClientRelay;
use crate::api::realtime_contracts::close::{BackendClose, ClientLoopExit};
use crate::api::realtime_contracts::config::{BaseRealtimeConfig, Environment};
use crate::api::realtime_contracts::error::{HeaderMap, WebSocketError};
use crate::api::realtime_contracts::events::{
    AudioTranscriptionUsage, RealtimeResponseTransformInput, RealtimeResponseTypedDict,
};
use crate::api::realtime_contracts::transport::{
    ClientWebSocket, RealtimeBackend, RealtimeMessage, ScopedWebSocket, WebSocketScope,
};
use crate::api::realtime_guardrail_policy::RealtimeGuardrailCallback;
use crate::api::realtime_guardrails::callback::{
    GuardrailApplication, GuardrailFailure, GuardrailInput, GuardrailTexts, GuardrailViolation,
    RealtimeGuardrail, RealtimeGuardrailPolicy,
};
use crate::api::realtime_session_state::logging::{
    RealtimeLogging, RealtimeSuccessQueue, SessionInput, SuccessWork,
};
use crate::api::realtime_session_state::{
    ConnectionSetup, RealtimeProviderConfig, RealtimeStreaming,
};
use crate::api::request_destination::RequestData;

pub(super) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("json value")
}

fn begin_attempt(
    failures: &[usize],
    attempts: &mut usize,
    prefix: &str,
) -> Result<(), WebSocketError> {
    let attempt = *attempts;
    *attempts = (*attempts).saturating_add(1);
    if failures.contains(&attempt) {
        return Err(WebSocketError {
            message: format!("{prefix} {attempt} failed"),
        });
    }
    Ok(())
}

#[derive(Clone, Default)]
pub(super) struct SendLog(Arc<Mutex<SendData>>);

#[derive(Default)]
pub(super) struct SendData {
    pub(super) frames: Vec<String>,
    pub(super) attempts: usize,
    pub(super) failures: Vec<usize>,
}

impl SendLog {
    fn begin(&self) -> Result<(), WebSocketError> {
        let mut data = lock(&self.0);
        begin_attempt(&data.failures, &mut data.attempts, "the scripted send")
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

    pub(super) fn attempts(&self) -> usize {
        lock(&self.0).attempts
    }
}

#[derive(Default)]
pub(super) struct ClientData {
    pub(super) incoming: VecDeque<Result<String, WebSocketError>>,
    pub(super) sent: Vec<String>,
    pub(super) attempts: usize,
    pub(super) failures: Vec<usize>,
    pub(super) closes: Vec<(u16, Option<String>)>,
    pub(super) close_attempts: usize,
    pub(super) close_failures: Vec<usize>,
}

pub(super) struct ScriptedClient {
    pub(super) data: Arc<Mutex<ClientData>>,
    pub(super) scope: WebSocketScope,
}

impl ScopedWebSocket for ScriptedClient {
    fn scope(&self) -> &WebSocketScope {
        &self.scope
    }
}

#[async_trait]
impl ClientWebSocket for ScriptedClient {
    async fn send_text(&mut self, data: &str) -> Result<(), WebSocketError> {
        let mut state = lock(&self.data);
        begin_attempt(
            &state.failures,
            &mut state.attempts,
            "the scripted client send",
        )?;
        state.sent.push(data.to_owned());
        Ok(())
    }

    async fn receive_text(&mut self) -> Result<String, WebSocketError> {
        let next = lock(&self.data).incoming.pop_front();
        next.unwrap_or_else(|| {
            Err(WebSocketError {
                message: String::from("the scripted client is exhausted"),
            })
        })
    }

    async fn close(&mut self, code: u16, reason: Option<&str>) -> Result<(), WebSocketError> {
        let (attempt, refused) = {
            let mut state = lock(&self.data);
            let attempt = state.close_attempts;
            state.close_attempts = state.close_attempts.saturating_add(1);
            (attempt, state.close_failures.contains(&attempt))
        };
        lock(&self.data)
            .closes
            .push((code, reason.map(str::to_owned)));
        if refused {
            return Err(WebSocketError {
                message: format!("the scripted client close {attempt} failed"),
            });
        }
        Ok(())
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
        self.log.begin()?;
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

// The provider echoes every frame and reports whether it defers session setup; a non-delivering one drops every frame
#[derive(Debug, Clone, Copy)]
pub(super) struct ScriptedProvider {
    pub(super) requires_session_configuration: bool,
    pub(super) delivers: bool,
}

impl BaseRealtimeConfig for ScriptedProvider {
    type Error = Infallible;
    type Backend = MockBackend;

    fn validate_environment(
        &self,
        _environment: &Environment<'_>,
    ) -> Result<HeaderMap, Infallible> {
        Ok(HeaderMap::new())
    }

    fn transform_realtime_request(
        &self,
        message: &str,
        _model: &str,
        _session_configuration_request: Option<&str>,
    ) -> Result<Vec<RealtimeMessage>, Infallible> {
        if self.delivers {
            return Ok(vec![RealtimeMessage::Text(message.to_owned())]);
        }
        Ok(Vec::new())
    }

    fn transform_realtime_response(
        &self,
        _message: &RealtimeMessage,
        _model: &str,
        _input: &RealtimeResponseTransformInput,
    ) -> Result<RealtimeResponseTypedDict, Infallible> {
        Ok(RealtimeResponseTypedDict {
            response: Json::Null,
            current_output_item_id: None,
            current_response_id: None,
            current_delta_chunks: None,
            current_conversation_id: None,
            current_item_chunks: None,
            current_delta_type: None,
            session_configuration_request: None,
        })
    }
}

impl RealtimeProviderConfig for ScriptedProvider {
    fn requires_session_configuration(&self) -> bool {
        self.requires_session_configuration
    }

    fn unbilled_usage_on_close(&self, _model: &str) -> Option<AudioTranscriptionUsage> {
        None
    }
}

#[derive(Clone, Default)]
pub(super) struct ScriptedNormalizer {
    pub(super) sessions: Arc<Mutex<Vec<Json>>>,
}

impl RealtimeEventNormalizer for ScriptedNormalizer {
    fn should_drop(&self, _event: &Json) -> bool {
        false
    }

    fn normalize(&self, event: Json) -> Json {
        event
    }

    fn patch_outgoing_session(&self, session: Json) -> Json {
        lock(&self.sessions).push(session.clone());
        let Json::Object(mut fields) = session else {
            return session;
        };
        fields.insert("xai_patch".to_owned(), Json::Bool(true));
        Json::Object(fields)
    }
}

#[derive(Clone)]
pub(super) struct ScriptedGuardrail {
    pub(super) hooks: Vec<GuardrailEventHooks>,
    pub(super) block_on: Option<String>,
    pub(super) custom: bool,
    pub(super) policy: RealtimeGuardrailPolicy,
    pub(super) checks: Arc<Mutex<Vec<(GuardrailEventHooks, String)>>>,
    pub(super) applied: Arc<Mutex<Vec<Vec<String>>>>,
}

impl ScriptedGuardrail {
    pub(super) fn allowing(hooks: Vec<GuardrailEventHooks>) -> Self {
        Self {
            hooks,
            block_on: None,
            custom: true,
            policy: RealtimeGuardrailPolicy::default(),
            checks: Arc::new(Mutex::new(Vec::new())),
            applied: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(super) fn blocking(hooks: Vec<GuardrailEventHooks>, block_on: &str) -> Self {
        let mut guardrail = Self::allowing(hooks);
        guardrail.block_on = Some(block_on.to_owned());
        guardrail
    }

    pub(super) fn checked(&self) -> Vec<(GuardrailEventHooks, String)> {
        lock(&self.checks).clone()
    }

    pub(super) fn applied(&self) -> Vec<Vec<String>> {
        lock(&self.applied).clone()
    }
}

impl RealtimeGuardrailCallback for ScriptedGuardrail {
    type Error = Infallible;

    fn is_custom_guardrail(&self) -> bool {
        self.custom
    }

    fn should_run_guardrail(
        &self,
        data: &RequestData,
        event: GuardrailEventHooks,
    ) -> Result<bool, Infallible> {
        let transcript = match data.get("transcript") {
            Some(Json::Text(text)) => text.clone(),
            Some(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
            )
            | None => String::new(),
        };
        lock(&self.checks).push((event, transcript));
        Ok(self.hooks.contains(&event))
    }
}

impl RealtimeGuardrail for ScriptedGuardrail {
    fn use_native_lifecycle_hooks(&self) -> bool {
        false
    }

    fn policy(&self) -> &RealtimeGuardrailPolicy {
        &self.policy
    }

    fn apply_guardrail(&self, input: GuardrailInput) -> GuardrailApplication<'_, Infallible> {
        lock(&self.applied).push(input.texts.clone());
        let blocked = input
            .texts
            .first()
            .is_some_and(|text| self.block_on.as_deref() == Some(text.as_str()));
        Box::pin(async move {
            if blocked {
                return Err(GuardrailFailure::Blocked(GuardrailViolation {
                    detail: Json::Null,
                    message: String::from("blocked"),
                }));
            }
            Ok(GuardrailTexts::default())
        })
    }
}

#[derive(Clone, Default)]
pub(super) struct RecordingLogging {
    pub(super) pre_calls: Arc<Mutex<Vec<String>>>,
    pub(super) details: Arc<Mutex<Vec<(String, Json)>>>,
}

impl RealtimeLogging for RecordingLogging {
    fn pre_call(&self, input: &SessionInput<'_>, _api_key: &str) {
        match input {
            SessionInput::Text(text) => lock(&self.pre_calls).push((*text).to_owned()),
            SessionInput::Record(record) => {
                let encoded =
                    serde_json::to_string(&Json::Object((*record).clone())).unwrap_or_default();
                lock(&self.pre_calls).push(encoded);
            }
        }
    }

    fn record_model_call_detail(&self, key: &str, value: Json) {
        lock(&self.details).push((key.to_owned(), value));
    }

    fn dispatch_success_handlers(&self, _messages: &[IndexMap<String, Json>]) -> SuccessWork {
        Box::pin(async {})
    }
}

impl RealtimeSuccessQueue for RecordingLogging {
    fn enqueue(&self, _work: SuccessWork) -> bool {
        true
    }
}

pub(super) struct Fixture {
    pub(super) state: RealtimeStreaming,
    pub(super) provider: Option<ScriptedProvider>,
    pub(super) guardrails: Vec<ScriptedGuardrail>,
    pub(super) normalizer: Option<ScriptedNormalizer>,
    pub(super) redactor: Option<Redactor>,
    pub(super) client: Arc<Mutex<ClientData>>,
    pub(super) backend: SendLog,
    pub(super) logging: RecordingLogging,
}

impl Default for Fixture {
    fn default() -> Self {
        let client = Arc::new(Mutex::new(ClientData::default()));
        let backend = SendLog::default();
        let logging = RecordingLogging::default();
        let state = RealtimeStreaming::new(ConnectionSetup {
            client: Some(Box::new(ScriptedClient {
                data: Arc::clone(&client),
                scope: WebSocketScope::default(),
            })),
            backend: Some(Box::new(MockBackend {
                log: backend.clone(),
            })),
            logging: Some(Box::new(logging.clone())),
            logging_worker: Some(Box::new(logging.clone())),
            provider_config: None,
            event_normalizer: None,
            logged_real_time_event_types: LoggedEventTypes::All,
            model: String::from("scripted"),
            user_api_key_dict: None,
            request_data: Some(IndexMap::new()),
            force_transcription_model: None,
            client_wants_beta: false,
            backend_uses_beta_protocol: None,
        });
        Self {
            state,
            provider: None,
            guardrails: Vec::new(),
            normalizer: None,
            redactor: None,
            client,
            backend,
            logging,
        }
    }
}

impl Fixture {
    pub(super) fn with_provider(
        mut self,
        requires_session_configuration: bool,
        delivers: bool,
    ) -> Self {
        let provider = ScriptedProvider {
            requires_session_configuration,
            delivers,
        };
        self.state.provider_config = Some(Box::new(provider));
        self.state.setup.backend_setup_complete.0 = requires_session_configuration;
        self.provider = Some(provider);
        self
    }

    pub(super) fn with_guardrail(mut self, guardrail: ScriptedGuardrail) -> Self {
        self.guardrails.push(guardrail);
        self
    }

    pub(super) fn with_normalizer(mut self) -> Self {
        let normalizer = ScriptedNormalizer::default();
        self.state.event_normalizer = Some(Box::new(normalizer.clone()));
        self.normalizer = Some(normalizer);
        self
    }

    pub(super) fn with_redactor(mut self) -> Self {
        self.redactor =
            Some(Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the scripted redactor builds"));
        self
    }

    pub(super) fn with_beta_backend(mut self) -> Self {
        self.state.backend_uses_beta_protocol.0 = true;
        self
    }

    pub(super) fn with_pending_messages(mut self, messages: &[&str]) -> Self {
        self.state.setup.pending_messages_until_setup = messages
            .iter()
            .map(|message| (*message).to_owned())
            .collect();
        self.state.setup.pending_messages_byte_total =
            messages.iter().map(|message| message.len()).sum();
        self
    }

    pub(super) fn with_setup_complete(mut self, complete: bool) -> Self {
        self.state.setup.backend_setup_complete.0 = complete;
        self
    }

    pub(super) fn with_flushing(mut self, flushing: bool) -> Self {
        self.state.setup.flushing_pending_messages.0 = flushing;
        self
    }

    pub(super) fn with_turn_detection_sent(mut self, sent: bool) -> Self {
        self.state.setup.turn_detection_update_sent.0 = sent;
        self
    }

    pub(super) fn with_pending_guardrail(mut self, message: &str) -> Self {
        self.state.setup.pending_guardrail_message = Some(message.to_owned());
        self
    }

    pub(super) fn with_client_message(mut self, message: &str) -> Self {
        lock(&self.client)
            .incoming
            .push_back(Ok(message.to_owned()));
        self
    }

    pub(super) fn with_client_failure(mut self, message: &str) -> Self {
        lock(&self.client).incoming.push_back(Err(WebSocketError {
            message: message.to_owned(),
        }));
        self
    }

    pub(super) fn with_backend_failures(mut self, indices: Vec<usize>) -> Self {
        lock(&self.backend.0).failures = indices;
        self
    }

    pub(super) fn with_client_send_failures(mut self, indices: Vec<usize>) -> Self {
        lock(&self.client).failures = indices;
        self
    }

    pub(super) fn with_client_close_failures(mut self, indices: Vec<usize>) -> Self {
        lock(&self.client).close_failures = indices;
        self
    }
}

pub(super) struct Outcome {
    pub(super) exit: ClientLoopExit,
    pub(super) state: RealtimeStreaming,
    pub(super) client: Arc<Mutex<ClientData>>,
    pub(super) backend: SendLog,
    pub(super) logging: RecordingLogging,
    pub(super) normalizer: Option<ScriptedNormalizer>,
}

impl Outcome {
    pub(super) fn client_frames(&self) -> Vec<Json> {
        lock(&self.client)
            .sent
            .iter()
            .map(|text| json(text))
            .collect()
    }

    pub(super) fn client_texts(&self) -> Vec<String> {
        lock(&self.client).sent.clone()
    }

    pub(super) fn closes(&self) -> Vec<(u16, Option<String>)> {
        lock(&self.client).closes.clone()
    }

    pub(super) fn pre_calls(&self) -> Vec<String> {
        lock(&self.logging.pre_calls).clone()
    }

    pub(super) fn parsed_pre_calls(&self) -> Vec<Json> {
        self.pre_calls().iter().map(|text| json(text)).collect()
    }

    pub(super) fn patched_sessions(&self) -> Vec<Json> {
        match &self.normalizer {
            Some(normalizer) => lock(&normalizer.sessions).clone(),
            None => Vec::new(),
        }
    }
}

pub(super) fn guardrail_refs(
    guardrails: &[ScriptedGuardrail],
) -> Vec<&dyn RealtimeGuardrail<Error = Infallible>> {
    guardrails
        .iter()
        .map(|guardrail| guardrail as &dyn RealtimeGuardrail<Error = Infallible>)
        .collect()
}

enum Step {
    Ack,
    Close(BackendClose),
}

pub(super) async fn ack(fixture: Fixture) -> Outcome {
    run_fixture(fixture, Step::Ack).await
}

pub(super) async fn close(fixture: Fixture, close: BackendClose) -> Outcome {
    run_fixture(fixture, Step::Close(close)).await
}

async fn run_fixture(fixture: Fixture, step: Step) -> Outcome {
    let Fixture {
        mut state,
        provider,
        guardrails,
        normalizer,
        redactor,
        client,
        backend,
        logging,
    } = fixture;
    let refs = guardrail_refs(&guardrails);
    let callbacks = relay_callbacks(&refs);
    let mut relay = RealtimeClientRelay {
        state: &mut state,
        provider: provider.as_ref(),
        guardrails: &refs,
        callbacks,
        redactor: redactor.as_ref(),
    };
    let exit = match step {
        Step::Ack => relay.client_ack_messages().await,
        Step::Close(close) => {
            relay.close_client(&close).await;
            ClientLoopExit::ClientDisconnected
        }
    };
    Outcome {
        exit,
        state,
        client,
        backend,
        logging,
        normalizer,
    }
}

#[derive(Clone)]
pub(super) struct Captured(Arc<Mutex<Vec<(Level, String)>>>);

impl Default for Captured {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(Vec::new())))
    }
}

impl Captured {
    pub(super) fn messages(&self) -> Vec<String> {
        lock(&self.0)
            .iter()
            .map(|(_level, message)| message.clone())
            .collect()
    }

    pub(super) fn starts_with(&self, prefix: &str) -> bool {
        self.messages()
            .iter()
            .any(|message| message.starts_with(prefix))
    }

    pub(super) fn is_empty(&self) -> bool {
        lock(&self.0).is_empty()
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
