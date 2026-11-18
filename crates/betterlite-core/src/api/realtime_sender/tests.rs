#![cfg(test)]

use std::collections::VecDeque;
use std::convert::Infallible;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use betterlite_config::api::realtime_session_state::LoggedEventTypes;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_models::decisions::Json;

use crate::api::realtime_contracts::config::{BaseRealtimeConfig, Environment};
use crate::api::realtime_contracts::error::{HeaderMap, WebSocketError};
use crate::api::realtime_contracts::events::{
    AudioTranscriptionUsage, RealtimeResponseTransformInput, RealtimeResponseTypedDict,
};
use crate::api::realtime_contracts::transport::{
    ClientWebSocket, RealtimeBackend, RealtimeEventNormalizer, RealtimeMessage, ScopedWebSocket,
    WebSocketScope,
};
use crate::api::realtime_guardrail_policy::RealtimeGuardrailCallback;
use crate::api::realtime_session_state::{
    ConnectionSetup, RealtimeProviderConfig, RealtimeStreaming,
};
use crate::api::request_destination::RequestData;

use super::RealtimeSender;

mod backend;
mod client;
mod enforce;
mod setup;

const SETUP_FRAME: &str = r#"{"setup":{"model":"models/gemini-live","generationConfig":{"responseModalities":["AUDIO"]}}}"#;
const SECOND_SETUP: &str = r#"{"setup":{"model":"models/gemini-live","systemInstruction":{"parts":[{"text":"be brief"}]}}}"#;
const CONTENT_FRAME: &str = r#"{"realtimeInput":{"mediaChunks":[{"mimeType":"audio/pcm"}]}}"#;
const BINARY_FRAME: &[u8] = b"binary-frame";
const FLAT_UPDATE: &str = r#"{"type":"session.update","session":{"input_audio_transcription":{"model":"forbidden","language":"en"},"instructions":"keep"}}"#;
const FLAT_EXPECTED: &str = r#"{"type":"session.update","session":{"input_audio_transcription":{"model":"allowed","language":"en"},"instructions":"keep"}}"#;
const GA_UPDATE: &str = r#"{"type":"session.update","session":{"audio":{"input":{"transcription":{"model":"other","language":"fr"},"format":{"type":"audio/pcm"}},"output":{"voice":"alloy"}}}}"#;
const GA_EXPECTED: &str = r#"{"type":"session.update","session":{"audio":{"input":{"transcription":{"model":"allowed","language":"fr"},"format":{"type":"audio/pcm"}},"output":{"voice":"alloy"}}}}"#;
const BOTH_UPDATE: &str = r#"{"type":"transcription_session.update","session":{"type":"transcription","input_audio_transcription":{"model":"forbidden"},"audio":{"input":{"transcription":{"model":"other"}}}}}"#;
const BOTH_EXPECTED: &str = r#"{"type":"transcription_session.update","session":{"type":"transcription","input_audio_transcription":{"model":"allowed"},"audio":{"input":{"transcription":{"model":"allowed"}}}}}"#;
const GA_DISABLE: &str = r#"{"type":"session.update","session":{"type":"realtime","audio":{"input":{"turn_detection":{"type":"server_vad","create_response":false}}}}}"#;
const BETA_DISABLE: &str = r#"{"type":"session.update","session":{"turn_detection":{"type":"server_vad","create_response":false}}}"#;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn parsed(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn no_guardrails() -> [&'static dyn RealtimeGuardrailCallback<Error = Infallible>; 0] {
    []
}

fn frame_label(message: &RealtimeMessage) -> String {
    match message {
        RealtimeMessage::Text(text) => text.clone(),
        RealtimeMessage::Binary(bytes) => String::from_utf8(bytes.clone()).unwrap(),
    }
}

fn cached(state: &RealtimeStreaming) -> Option<&str> {
    state.response.session_configuration_request.as_deref()
}

fn make<'sender>(
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

#[derive(Clone, Default)]
struct Timeline(Arc<Mutex<Vec<String>>>);

impl Timeline {
    fn record(&self, entry: String) {
        lock(&self.0).push(entry);
    }

    fn entries(&self) -> Vec<String> {
        lock(&self.0).clone()
    }
}

#[derive(Debug, thiserror::Error)]
enum ProviderFailure {
    #[error("the scripted provider transformation failed")]
    Transform,
}

#[derive(Debug, Default)]
struct ScriptData {
    transforms: VecDeque<Vec<RealtimeMessage>>,
    setup: Vec<Json>,
    content: Vec<Json>,
    paced: Vec<Vec<u8>>,
    seen: Vec<(String, String, Option<String>)>,
    fail_transform: bool,
}

#[derive(Clone, Default)]
struct Script(Arc<Mutex<ScriptData>>);

#[derive(Clone, Default)]
struct ScriptedProvider {
    script: Script,
    timeline: Timeline,
    requires_session_configuration: bool,
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

    fn transform_realtime_request(
        &self,
        message: &str,
        model: &str,
        session_configuration_request: Option<&str>,
    ) -> Result<Vec<RealtimeMessage>, ProviderFailure> {
        let mut data = lock(&self.script.0);
        data.seen.push((
            message.to_owned(),
            model.to_owned(),
            session_configuration_request.map(str::to_owned),
        ));
        if data.fail_transform {
            return Err(ProviderFailure::Transform);
        }
        Ok(data.transforms.pop_front().unwrap_or_default())
    }

    async fn pace_backend_send(&self, message: &[u8]) {
        self.timeline.record(format!(
            "pace:{}",
            String::from_utf8(message.to_vec()).unwrap()
        ));
        lock(&self.script.0).paced.push(message.to_vec());
    }

    fn is_setup_message(&self, message: &Json) -> bool {
        lock(&self.script.0).setup.contains(message)
    }

    fn is_content_message(&self, message: &Json) -> bool {
        lock(&self.script.0).content.contains(message)
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

struct SendData<Item> {
    sent: Vec<Item>,
    failures: Vec<usize>,
    attempts: usize,
}

impl<Item> Default for SendData<Item> {
    fn default() -> Self {
        Self {
            sent: Vec::new(),
            failures: Vec::new(),
            attempts: 0,
        }
    }
}

#[derive(Clone)]
struct SendLog<Item>(Arc<Mutex<SendData<Item>>>);

impl<Item> Default for SendLog<Item> {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(SendData::default())))
    }
}

impl<Item> SendLog<Item> {
    fn fail_on(&self, index: usize) {
        lock(&self.0).failures.push(index);
    }

    fn attempts(&self) -> usize {
        lock(&self.0).attempts
    }

    // An injected attempt index fails before the frame reaches the socket, so a failed attempt is never recorded
    fn begin(&self, target: &str) -> Result<(), WebSocketError> {
        let mut data = lock(&self.0);
        let index = data.attempts;
        data.attempts = data.attempts.saturating_add(1);
        if data.failures.contains(&index) {
            return Err(WebSocketError {
                message: format!("the scripted {target} send {index} failed"),
            });
        }
        drop(data);
        Ok(())
    }

    fn push(&self, item: Item) {
        lock(&self.0).sent.push(item);
    }
}

impl<Item> SendLog<Item>
where
    Item: Clone,
{
    fn sent(&self) -> Vec<Item> {
        lock(&self.0).sent.clone()
    }
}

type BackendLog = SendLog<RealtimeMessage>;
type ClientLog = SendLog<String>;

struct MockBackend {
    log: BackendLog,
    timeline: Timeline,
}

#[async_trait]
impl RealtimeBackend for MockBackend {
    async fn send(&mut self, message: RealtimeMessage) -> Result<(), WebSocketError> {
        self.log.begin("backend")?;
        self.timeline
            .record(format!("backend:{}", frame_label(&message)));
        self.log.push(message);
        Ok(())
    }

    async fn recv(&mut self, _decode: Option<bool>) -> Result<RealtimeMessage, WebSocketError> {
        Err(WebSocketError {
            message: "the scripted backend does not read".to_owned(),
        })
    }

    async fn close(&mut self) -> Result<(), WebSocketError> {
        Ok(())
    }
}

struct MockClient {
    log: ClientLog,
    timeline: Timeline,
    scope: WebSocketScope,
}

#[async_trait]
impl ClientWebSocket for MockClient {
    async fn send_text(&mut self, data: &str) -> Result<(), WebSocketError> {
        self.log.begin("client")?;
        self.timeline.record(format!("client:{data}"));
        self.log.push(data.to_owned());
        Ok(())
    }

    async fn receive_text(&mut self) -> Result<String, WebSocketError> {
        Err(WebSocketError {
            message: "the scripted client does not read".to_owned(),
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

struct Fixture {
    provider: ScriptedProvider,
    backend_log: BackendLog,
    client_log: ClientLog,
    timeline: Timeline,
}

impl Fixture {
    fn new() -> Self {
        let timeline = Timeline::default();
        Self {
            provider: ScriptedProvider {
                script: Script::default(),
                timeline: timeline.clone(),
                requires_session_configuration: false,
            },
            backend_log: BackendLog::default(),
            client_log: ClientLog::default(),
            timeline,
        }
    }

    fn script(&self) -> MutexGuard<'_, ScriptData> {
        lock(&self.provider.script.0)
    }

    fn state(&self) -> RealtimeStreaming {
        RealtimeStreaming::new(ConnectionSetup {
            client: Some(Box::new(MockClient {
                log: self.client_log.clone(),
                timeline: self.timeline.clone(),
                scope: WebSocketScope::default(),
            })),
            backend: Some(Box::new(MockBackend {
                log: self.backend_log.clone(),
                timeline: self.timeline.clone(),
            })),
            logging: None,
            logging_worker: None,
            provider_config: Some(Box::new(self.provider.clone())),
            event_normalizer: None,
            logged_real_time_event_types: LoggedEventTypes::Default,
            model: "gpt-realtime".to_owned(),
            user_api_key_dict: None,
            request_data: None,
            force_transcription_model: None,
            client_wants_beta: false,
            backend_uses_beta_protocol: None,
        })
    }
}

struct AudioGuardrail;

impl RealtimeGuardrailCallback for AudioGuardrail {
    type Error = Infallible;

    fn is_custom_guardrail(&self) -> bool {
        true
    }

    fn should_run_guardrail(
        &self,
        _data: &RequestData,
        event: GuardrailEventHooks,
    ) -> Result<bool, Infallible> {
        Ok(event == GuardrailEventHooks::RealtimeInputTranscription)
    }
}

struct PreCallGuardrail;

impl RealtimeGuardrailCallback for PreCallGuardrail {
    type Error = Infallible;

    fn is_custom_guardrail(&self) -> bool {
        true
    }

    fn should_run_guardrail(
        &self,
        _data: &RequestData,
        event: GuardrailEventHooks,
    ) -> Result<bool, Infallible> {
        Ok(event == GuardrailEventHooks::PreCall)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the scripted guardrail predicate failed")]
struct GuardFailure;

struct FailingGuardrail;

impl RealtimeGuardrailCallback for FailingGuardrail {
    type Error = GuardFailure;

    fn is_custom_guardrail(&self) -> bool {
        true
    }

    fn should_run_guardrail(
        &self,
        _data: &RequestData,
        _event: GuardrailEventHooks,
    ) -> Result<bool, GuardFailure> {
        Err(GuardFailure)
    }
}

struct Normalizer {
    drop_types: Vec<String>,
    rename: Vec<(String, String)>,
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
                .find(|(from, _)| from == kind)
                .map(|(_, to)| to.clone()),
            _ => None,
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
