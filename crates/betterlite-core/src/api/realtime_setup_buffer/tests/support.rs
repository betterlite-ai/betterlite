#![cfg(test)]

use std::collections::VecDeque;
use std::convert::Infallible;
use std::fmt::Debug;
use std::mem::take;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use betterlite_config::api::realtime_session_state::LoggedEventTypes;
use betterlite_models::decisions::Json;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use crate::api::realtime_contracts::config::{BaseRealtimeConfig, Environment};
use crate::api::realtime_contracts::error::{HeaderMap, WebSocketError};
use crate::api::realtime_contracts::events::{
    AudioTranscriptionUsage, RealtimeResponseTransformInput, RealtimeResponseTypedDict,
};
use crate::api::realtime_contracts::transport::{
    ClientWebSocket, RealtimeBackend, RealtimeMessage, ScopedWebSocket, WebSocketScope,
};
use crate::api::realtime_guardrail_policy::RealtimeGuardrailCallback;
use crate::api::realtime_sender::RealtimeSender;
use crate::api::realtime_session_state::{
    ConnectionSetup, RealtimeProviderConfig, RealtimeStreaming,
};

pub(super) const APPEND: &str = r#"{"type":"input_audio_buffer.append"}"#;
pub(super) const COMMIT_FRAME: &str = r#"{"type":"input_audio_buffer.commit"}"#;
pub(super) const CLEAR_FRAME: &str = r#"{"type":"input_audio_buffer.clear"}"#;
pub(super) const END_FRAME: &str = r#"{"type":"input_audio_buffer.end"}"#;
pub(super) const UNRELATED_FRAME: &str = r#"{"type":"session.update"}"#;
pub(super) const MALFORMED_FRAME: &str = "{not json";
pub(super) const NON_OBJECT_FRAME: &str = "[1]";

pub(super) fn audio_frame(padding: &str) -> String {
    format!(r#"{{"type":"input_audio_buffer.append","audio":"{padding}"}}"#)
}

pub(super) fn append(id: &str) -> String {
    audio_frame(id)
}

pub(super) fn byte_sum(frames: &[String]) -> usize {
    frames
        .iter()
        .map(String::len)
        .fold(0, usize::saturating_add)
}

pub(super) fn no_guardrails() -> [&'static dyn RealtimeGuardrailCallback<Error = Infallible>; 0] {
    []
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Debug, thiserror::Error)]
pub(super) enum ProviderFailure {
    #[error("the scripted provider transformation failed")]
    Transform,
}

#[derive(Debug, Default)]
pub(super) struct ScriptData {
    pub(super) transforms: VecDeque<Vec<RealtimeMessage>>,
    pub(super) seen: Vec<String>,
    pub(super) fail_transform: bool,
}

#[derive(Clone, Default)]
pub(super) struct Script(Arc<Mutex<ScriptData>>);

#[derive(Clone, Default)]
pub(super) struct ScriptedProvider {
    pub(super) requires_session_configuration: bool,
    pub(super) script: Script,
}

#[async_trait]
impl BaseRealtimeConfig for ScriptedProvider {
    type Error = ProviderFailure;
    type Backend = MockBackend;

    fn validate_environment(
        &self,
        _environment: &Environment<'_>,
    ) -> Result<HeaderMap, ProviderFailure> {
        Ok(HeaderMap::new())
    }

    // A frame without a scripted transformation reaches the backend unchanged, like a provider that rewrites nothing
    fn transform_realtime_request(
        &self,
        message: &str,
        _model: &str,
        _session_configuration_request: Option<&str>,
    ) -> Result<Vec<RealtimeMessage>, ProviderFailure> {
        let mut data = lock(&self.script.0);
        data.seen.push(message.to_owned());
        if data.fail_transform {
            return Err(ProviderFailure::Transform);
        }
        Ok(data
            .transforms
            .pop_front()
            .unwrap_or_else(|| vec![RealtimeMessage::Text(message.to_owned())]))
    }

    fn requires_session_configuration(&self) -> bool {
        self.requires_session_configuration
    }

    fn transform_realtime_response(
        &self,
        _message: &RealtimeMessage,
        _model: &str,
        _input: &RealtimeResponseTransformInput,
    ) -> Result<RealtimeResponseTypedDict, ProviderFailure> {
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

#[derive(Default)]
struct BackendData {
    sent: Vec<RealtimeMessage>,
    failures: Vec<usize>,
    attempts: usize,
}

#[derive(Clone, Default)]
pub(super) struct BackendLog(Arc<Mutex<BackendData>>);

impl BackendLog {
    pub(super) fn fail_on(&self, index: usize) {
        lock(&self.0).failures.push(index);
    }

    pub(super) fn attempts(&self) -> usize {
        lock(&self.0).attempts
    }

    // An injected attempt index fails before the frame reaches the socket, so a failed attempt is never recorded
    fn begin(&self) -> Result<(), WebSocketError> {
        let mut data = lock(&self.0);
        let index = data.attempts;
        data.attempts = data.attempts.saturating_add(1);
        if data.failures.contains(&index) {
            return Err(WebSocketError {
                message: format!("the scripted backend send {index} failed"),
            });
        }
        drop(data);
        Ok(())
    }

    fn push(&self, message: RealtimeMessage) {
        lock(&self.0).sent.push(message);
    }

    pub(super) fn sent_text(&self) -> Vec<String> {
        let data = lock(&self.0);
        data.sent.iter().map(text_of).collect()
    }
}

fn text_of(message: &RealtimeMessage) -> String {
    match message {
        RealtimeMessage::Text(text) => text.clone(),
        RealtimeMessage::Binary(bytes) => String::from_utf8(bytes.clone()).unwrap(),
    }
}

pub(super) struct MockBackend {
    log: BackendLog,
}

#[async_trait]
impl RealtimeBackend for MockBackend {
    async fn send(&mut self, message: RealtimeMessage) -> Result<(), WebSocketError> {
        self.log.begin()?;
        self.log.push(message);
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

#[derive(Clone, Default)]
pub(super) struct ClientLog(Arc<Mutex<Vec<String>>>);

impl ClientLog {
    pub(super) fn sent(&self) -> Vec<String> {
        lock(&self.0).clone()
    }
}

pub(super) struct MockClient {
    log: ClientLog,
    scope: WebSocketScope,
}

#[async_trait]
impl ClientWebSocket for MockClient {
    async fn send_text(&mut self, data: &str) -> Result<(), WebSocketError> {
        lock(&self.log.0).push(data.to_owned());
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

pub(super) struct Fixture {
    pub(super) provider: ScriptedProvider,
    pub(super) backend_log: BackendLog,
    pub(super) client_log: ClientLog,
}

impl Fixture {
    pub(super) fn new(requires_session_configuration: bool) -> Self {
        Self {
            provider: ScriptedProvider {
                requires_session_configuration,
                script: Script::default(),
            },
            backend_log: BackendLog::default(),
            client_log: ClientLog::default(),
        }
    }

    pub(super) fn script(&self) -> MutexGuard<'_, ScriptData> {
        lock(&self.provider.script.0)
    }

    pub(super) fn state(&self) -> RealtimeStreaming {
        RealtimeStreaming::new(ConnectionSetup {
            client: Some(Box::new(MockClient {
                log: self.client_log.clone(),
                scope: WebSocketScope::default(),
            })),
            backend: Some(Box::new(MockBackend {
                log: self.backend_log.clone(),
            })),
            logging: None,
            logging_worker: None,
            provider_config: Some(Box::new(self.provider.clone())),
            event_normalizer: None,
            logged_real_time_event_types: LoggedEventTypes::Default,
            model: String::from("gpt-realtime"),
            user_api_key_dict: None,
            request_data: None,
            force_transcription_model: None,
            client_wants_beta: false,
            backend_uses_beta_protocol: None,
        })
    }
}

pub(super) fn make<'sender>(
    state: &'sender mut RealtimeStreaming,
    fixture: &'sender Fixture,
    callbacks: &'sender [&'sender dyn RealtimeGuardrailCallback<Error = Infallible>],
) -> RealtimeSender<'sender, ScriptedProvider, Infallible> {
    RealtimeSender {
        state,
        provider: Some(&fixture.provider),
        guardrails: callbacks,
    }
}

// A retry step takes the sender a failed flush already built, so the concrete provider and callback types are named once here
pub(super) type Sender<'sender> = RealtimeSender<'sender, ScriptedProvider, Infallible>;

#[derive(Clone, Default)]
pub(super) struct CapturedLogs(Arc<Mutex<Vec<String>>>);

impl CapturedLogs {
    pub(super) fn take(&self) -> Vec<String> {
        take(&mut *lock(&self.0))
    }
}

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            value.clone_into(&mut self.0);
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for CapturedLogs {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let level = *event.metadata().level();
        if level != Level::DEBUG && level != Level::WARN {
            return;
        }
        let label = if level == Level::DEBUG {
            "DEBUG"
        } else {
            "WARN"
        };
        let mut message = Message::default();
        event.record(&mut message);
        lock(&self.0).push(format!("{label} {}", message.0));
    }
}

pub(super) fn capturing() -> (CapturedLogs, DefaultGuard) {
    let logs = CapturedLogs::default();
    let subscriber = tracing_subscriber::registry().with(logs.clone());
    let guard = set_default(subscriber);
    (logs, guard)
}
