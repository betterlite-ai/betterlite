use std::collections::VecDeque;
use std::convert::Infallible;
use std::future::pending;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::api::realtime_session_state::LoggedEventTypes;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_config::tls::Verification;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use tokio::sync::Mutex as TokioMutex;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::api::realtime_backend_relay::source::{RealtimeBackendSource, RelayReceiveError};
use crate::api::realtime_backend_transport::dial::{
    ConnectedBackend, DialRequest, RealtimeSsl, RealtimeWebSocketDialer,
};
use crate::api::realtime_backend_transport::error::{
    ClientCloseError, DialError, RealtimeConnectionError,
};
use crate::api::realtime_backend_transport::protocol::{
    ClientHandle, RealtimeClientSocket, RealtimeSessionRelay, SessionRelay,
};
use crate::api::realtime_backend_transport::session::RealtimeCall;
use crate::api::realtime_contracts::close::{CloseFrame, ConnectionClosed};
use crate::api::realtime_contracts::config::{BaseRealtimeConfig, Environment};
use crate::api::realtime_contracts::error::{HeaderMap, WebSocketError};
use crate::api::realtime_contracts::events::{
    AudioTranscriptionUsage, RealtimeResponseTransformInput, RealtimeResponseTypedDict,
};
use crate::api::realtime_contracts::transport::{RealtimeBackend, RealtimeMessage, WebSocketScope};
use crate::api::realtime_guardrail_policy::RealtimeGuardrailCallback;
use crate::api::realtime_session_state::state::SessionFlag;
use crate::api::request_destination::RequestData;

fn websocket_error(message: &str) -> WebSocketError {
    WebSocketError {
        message: message.to_owned(),
    }
}

#[derive(Default)]
struct BackendLog {
    sent: Vec<RealtimeMessage>,
    closed: bool,
}

struct FakeBackend {
    log: Arc<Mutex<BackendLog>>,
    send_error: Option<String>,
    close_error: Option<String>,
}

#[async_trait]
impl RealtimeBackend for FakeBackend {
    async fn send(&mut self, message: RealtimeMessage) -> Result<(), WebSocketError> {
        if let Some(error) = &self.send_error {
            return Err(websocket_error(error));
        }
        self.log.lock().unwrap().sent.push(message);
        Ok(())
    }

    async fn recv(&mut self, _decode: Option<bool>) -> Result<RealtimeMessage, WebSocketError> {
        Err(WebSocketError {
            message: "the fake backend holds no frames".to_owned(),
        })
    }

    async fn close(&mut self) -> Result<(), WebSocketError> {
        self.log.lock().unwrap().closed = true;
        self.close_error
            .as_ref()
            .map_or(Ok(()), |error| Err(websocket_error(error)))
    }
}

struct ScriptedSource {
    frames: VecDeque<RealtimeMessage>,
}

#[async_trait]
impl RealtimeBackendSource for ScriptedSource {
    async fn recv(&mut self, _decode: Option<bool>) -> Result<RealtimeMessage, RelayReceiveError> {
        let Some(frame) = self.frames.pop_front() else {
            return Err(RelayReceiveError::Closed(ConnectionClosed {
                received: Some(CloseFrame {
                    code: 1000,
                    reason: String::new(),
                }),
                message: "upstream closed".to_owned(),
            }));
        };
        Ok(frame)
    }
}

pub(super) enum DialOutcome {
    Fail(DialError),
    Hang,
    Connect,
}

pub(super) fn os(message: &str) -> DialOutcome {
    DialOutcome::Fail(DialError::Os {
        message: message.to_owned(),
    })
}

pub(super) fn websocket(message: &str) -> DialOutcome {
    DialOutcome::Fail(DialError::WebSocket {
        message: message.to_owned(),
    })
}

pub(super) const fn invalid_status(status_code: u16) -> DialOutcome {
    DialOutcome::Fail(DialError::InvalidStatus {
        status_code,
        message: String::new(),
    })
}

pub(super) fn invalid_status_code(status_code: u16, message: &str) -> DialOutcome {
    DialOutcome::Fail(DialError::InvalidStatusCode {
        status_code,
        message: message.to_owned(),
    })
}

struct DialRecord {
    url: String,
    headers: HeaderMap,
    ssl: RealtimeSsl,
    max_size: usize,
    open_timeout: Duration,
    at: Instant,
}

struct ScriptedDialer {
    outcomes: Mutex<VecDeque<DialOutcome>>,
    records: Arc<Mutex<Vec<DialRecord>>>,
    backend_log: Arc<Mutex<BackendLog>>,
    source_frames: Vec<RealtimeMessage>,
    send_error: Option<String>,
    close_error: Option<String>,
}

#[async_trait]
impl RealtimeWebSocketDialer for ScriptedDialer {
    async fn connect(&self, request: &DialRequest<'_>) -> Result<ConnectedBackend, DialError> {
        self.records.lock().unwrap().push(DialRecord {
            url: request.url.to_owned(),
            headers: request.headers.clone(),
            ssl: request.ssl.clone(),
            max_size: request.max_size,
            open_timeout: request.open_timeout,
            at: Instant::now(),
        });
        let outcome = self
            .outcomes
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| os("the dial script ran out"));
        match outcome {
            DialOutcome::Fail(error) => Err(error),
            DialOutcome::Hang => {
                pending::<()>().await;
                Err(DialError::Other {
                    message: String::new(),
                })
            }
            DialOutcome::Connect => Ok(ConnectedBackend {
                sender: Box::new(FakeBackend {
                    log: Arc::clone(&self.backend_log),
                    send_error: self.send_error.clone(),
                    close_error: self.close_error.clone(),
                }),
                source: Some(Box::new(ScriptedSource {
                    frames: VecDeque::from(self.source_frames.clone()),
                })),
            }),
        }
    }
}

#[derive(Default)]
struct ClientLog {
    texts: Vec<String>,
    close: Option<(u16, Option<String>)>,
}

struct FakeClient {
    log: Arc<Mutex<ClientLog>>,
    send_error: Option<String>,
    close_error: Option<ClientCloseError>,
}

impl FakeClient {
    fn new(options: &HarnessOptions, logs: &SharedLogs) -> Self {
        Self {
            log: Arc::clone(&logs.client),
            send_error: options.send_error.clone(),
            close_error: options.close_error.clone(),
        }
    }
}

#[async_trait]
impl RealtimeClientSocket for FakeClient {
    async fn send_text(&mut self, data: &str) -> Result<(), WebSocketError> {
        if let Some(message) = &self.send_error {
            return Err(websocket_error(message));
        }
        self.log.lock().unwrap().texts.push(data.to_owned());
        Ok(())
    }

    async fn receive_text(&mut self) -> Result<String, WebSocketError> {
        Err(WebSocketError {
            message: "the fake client holds no frames".to_owned(),
        })
    }

    async fn close(&mut self, code: u16, reason: Option<&str>) -> Result<(), ClientCloseError> {
        self.log.lock().unwrap().close = Some((code, reason.map(str::to_owned)));
        self.close_error
            .as_ref()
            .map_or(Ok(()), |error| Err(error.clone()))
    }
}

struct TestProvider {
    requires_setup: bool,
    setup: Option<String>,
    synthetic: Option<Json>,
    own_backend: bool,
    validate_error: Option<String>,
    open_backend_error: Option<String>,
    open_headers: Arc<Mutex<Vec<HeaderMap>>>,
    backend_log: Arc<Mutex<BackendLog>>,
    send_error: Option<String>,
    close_error: Option<String>,
    close_usage: Option<AudioTranscriptionUsage>,
}

#[async_trait]
impl BaseRealtimeConfig for TestProvider {
    type Error = io::Error;
    type Backend = FakeBackend;

    fn validate_environment(&self, environment: &Environment<'_>) -> Result<HeaderMap, io::Error> {
        if let Some(message) = &self.validate_error {
            return Err(io::Error::other(message.clone()));
        }
        let mut headers = environment.headers.clone();
        headers.insert(
            VALIDATED_HEADER.to_owned(),
            Json::Text(VALIDATED_HEADER_VALUE.to_owned()),
        );
        Ok(headers)
    }

    fn get_complete_url(
        &self,
        api_base: Option<&str>,
        _model: &str,
        _api_key: Option<&str>,
    ) -> String {
        api_base
            .unwrap_or("wss://provider.test/realtime")
            .to_owned()
    }

    fn transform_realtime_request(
        &self,
        _message: &str,
        _model: &str,
        _session_configuration_request: Option<&str>,
    ) -> Result<Vec<RealtimeMessage>, io::Error> {
        Ok(Vec::new())
    }

    fn requires_session_configuration(&self) -> bool {
        self.requires_setup
    }

    fn session_configuration_request(&self, _model: &str) -> Option<String> {
        self.setup.clone()
    }

    async fn open_backend(
        &self,
        _url: &str,
        headers: &HeaderMap,
    ) -> Result<Option<Self::Backend>, io::Error> {
        self.open_headers.lock().unwrap().push(headers.clone());
        if let Some(message) = &self.open_backend_error {
            return Err(io::Error::other(message.clone()));
        }
        if self.own_backend {
            return Ok(Some(FakeBackend {
                log: Arc::clone(&self.backend_log),
                send_error: self.send_error.clone(),
                close_error: self.close_error.clone(),
            }));
        }
        Ok(None)
    }

    fn session_created_event(
        &self,
        _model: &str,
        _logging_session_id: &str,
        _session_configuration_request: Option<&str>,
    ) -> Option<Json> {
        self.synthetic.clone()
    }

    fn session_close_usage(&self, _model: &str) -> Option<AudioTranscriptionUsage> {
        self.close_usage.clone()
    }

    fn transform_realtime_response(
        &self,
        _message: &RealtimeMessage,
        _model: &str,
        _input: &RealtimeResponseTransformInput,
    ) -> Result<RealtimeResponseTypedDict, io::Error> {
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

pub(super) struct TestCallback {
    pub(super) seen: Arc<Mutex<Option<RequestData>>>,
}

impl RealtimeGuardrailCallback for TestCallback {
    type Error = Infallible;

    fn is_custom_guardrail(&self) -> bool {
        true
    }

    fn should_run_guardrail(
        &self,
        data: &RequestData,
        _event: GuardrailEventHooks,
    ) -> Result<bool, Infallible> {
        *self.seen.lock().unwrap_or_else(PoisonError::into_inner) = Some(data.clone());
        Ok(true)
    }
}

#[derive(Clone, Default)]
pub(super) enum SourceOutcome {
    #[default]
    Unread,
    Text(String),
    Failed,
    Closed {
        code: Option<u16>,
        reason: Option<String>,
        message: String,
    },
}

#[derive(Default)]
pub(super) struct RelayObservation {
    pub(super) source_present: SessionFlag,
    pub(super) source_outcome: SourceOutcome,
    pub(super) session_configuration_request: Option<String>,
    pub(super) request_data: RequestData,
    pub(super) force_transcription_model: Option<String>,
    pub(super) session_created_sent: SessionFlag,
    pub(super) backend_setup_complete: SessionFlag,
    pub(super) turn_detection_update_sent: SessionFlag,
    pub(super) stored_messages: usize,
    pub(super) client_wants_beta: SessionFlag,
    pub(super) cancelled: SessionFlag,
}

enum RelayOutcome {
    Ok,
    Fail,
}

struct RecordingRelay {
    observation: Arc<Mutex<RelayObservation>>,
    outcome: RelayOutcome,
}

#[async_trait]
impl RealtimeSessionRelay for RecordingRelay {
    type Error = io::Error;

    async fn relay(&mut self, session: SessionRelay<'_>) -> Result<(), io::Error> {
        record_observation(&self.observation, &session);
        let state = session.state;
        if let Some(mut source) = session.source {
            let outcome = receive_outcome(source.as_mut()).await;
            self.observation.lock().unwrap().source_outcome = outcome;
        }
        state
            .flush_unbilled_usage()
            .map_err(|error| io::Error::other(error.to_string()))?;
        self.observation.lock().unwrap().stored_messages = state.messages.len();
        match self.outcome {
            RelayOutcome::Ok => Ok(()),
            RelayOutcome::Fail => Err(io::Error::other("the relay failed")),
        }
    }
}

fn record_observation(shared: &Mutex<RelayObservation>, session: &SessionRelay<'_>) {
    let mut observation = shared.lock().unwrap();
    observation.source_present = SessionFlag(session.source.is_some());
    observation.session_configuration_request =
        session.state.response.session_configuration_request.clone();
    observation.request_data = session.state.request_data.clone();
    observation.force_transcription_model = session.state.force_transcription_model.clone();
    observation.session_created_sent = session.state.setup.session_created_sent_to_client;
    observation.backend_setup_complete = session.state.setup.backend_setup_complete;
    observation.turn_detection_update_sent = session.state.setup.turn_detection_update_sent;
    observation.stored_messages = session.state.messages.len();
    observation.client_wants_beta = session.state.client_wants_beta;
    observation.cancelled = SessionFlag(session.cancellation.is_cancelled());
}

async fn receive_outcome(source: &mut (dyn RealtimeBackendSource + '_)) -> SourceOutcome {
    match source.recv(None).await {
        Ok(RealtimeMessage::Text(text)) => SourceOutcome::Text(text),
        Ok(RealtimeMessage::Binary(bytes)) => {
            SourceOutcome::Text(String::from_utf8(bytes).expect("the frame is UTF-8"))
        }
        Err(RelayReceiveError::UnsupportedDecode | RelayReceiveError::Failed(_)) => {
            SourceOutcome::Failed
        }
        Err(RelayReceiveError::Closed(closed)) => {
            let (code, reason) = match closed.received {
                Some(frame) => (Some(frame.code), Some(frame.reason)),
                None => (None, None),
            };
            SourceOutcome::Closed {
                code,
                reason,
                message: closed.message,
            }
        }
    }
}

pub(super) struct HarnessOptions {
    pub(super) requires_setup: SessionFlag,
    pub(super) setup: Option<String>,
    pub(super) synthetic: Option<Json>,
    pub(super) own_backend: SessionFlag,
    pub(super) validate_error: Option<String>,
    pub(super) open_backend_error: Option<String>,
    pub(super) client_scope_headers: Vec<(String, String)>,
    pub(super) metadata: Option<IndexMap<String, Json>>,
    pub(super) query_params: Option<IndexMap<String, String>>,
    pub(super) dials: Vec<DialOutcome>,
    pub(super) send_error: Option<String>,
    pub(super) close_error: Option<ClientCloseError>,
    pub(super) cancellation: SessionFlag,
    pub(super) relay_fails: SessionFlag,
    pub(super) verification: Verification,
    pub(super) max_message_size_bytes: usize,
    pub(super) source_frames: Vec<RealtimeMessage>,
    pub(super) backend_send_error: Option<String>,
    pub(super) backend_close_error: Option<String>,
    pub(super) close_usage: Option<AudioTranscriptionUsage>,
}

impl Default for HarnessOptions {
    fn default() -> Self {
        Self {
            requires_setup: SessionFlag(false),
            setup: None,
            synthetic: None,
            own_backend: SessionFlag(false),
            validate_error: None,
            open_backend_error: None,
            client_scope_headers: Vec::new(),
            metadata: None,
            query_params: None,
            dials: vec![DialOutcome::Connect],
            send_error: None,
            close_error: None,
            cancellation: SessionFlag(false),
            relay_fails: SessionFlag(false),
            verification: Verification::Enabled,
            max_message_size_bytes: 1024,
            source_frames: Vec::new(),
            backend_send_error: None,
            backend_close_error: None,
            close_usage: None,
        }
    }
}

impl HarnessOptions {
    pub(super) const fn with_requires_setup(mut self, requires_setup: SessionFlag) -> Self {
        self.requires_setup = requires_setup;
        self
    }

    pub(super) fn with_setup(mut self, setup: Option<String>) -> Self {
        self.setup = setup;
        self
    }

    pub(super) fn with_synthetic(mut self, synthetic: Option<Json>) -> Self {
        self.synthetic = synthetic;
        self
    }

    pub(super) const fn with_own_backend(mut self, own_backend: SessionFlag) -> Self {
        self.own_backend = own_backend;
        self
    }

    pub(super) fn with_validate_error(mut self, validate_error: Option<String>) -> Self {
        self.validate_error = validate_error;
        self
    }

    pub(super) fn with_open_backend_error(mut self, open_backend_error: Option<String>) -> Self {
        self.open_backend_error = open_backend_error;
        self
    }

    pub(super) fn with_client_scope_headers(
        mut self,
        client_scope_headers: Vec<(String, String)>,
    ) -> Self {
        self.client_scope_headers = client_scope_headers;
        self
    }

    pub(super) fn with_metadata(mut self, metadata: Option<IndexMap<String, Json>>) -> Self {
        self.metadata = metadata;
        self
    }

    pub(super) fn with_query_params(
        mut self,
        query_params: Option<IndexMap<String, String>>,
    ) -> Self {
        self.query_params = query_params;
        self
    }

    pub(super) fn with_dials(mut self, dials: Vec<DialOutcome>) -> Self {
        self.dials = dials;
        self
    }

    pub(super) fn with_send_error(mut self, send_error: Option<String>) -> Self {
        self.send_error = send_error;
        self
    }

    pub(super) fn with_close_error(mut self, close_error: Option<ClientCloseError>) -> Self {
        self.close_error = close_error;
        self
    }

    pub(super) const fn with_cancellation(mut self, cancellation: SessionFlag) -> Self {
        self.cancellation = cancellation;
        self
    }

    pub(super) const fn with_relay_fails(mut self, relay_fails: SessionFlag) -> Self {
        self.relay_fails = relay_fails;
        self
    }

    pub(super) fn with_verification(mut self, verification: Verification) -> Self {
        self.verification = verification;
        self
    }

    pub(super) const fn with_max_message_size_bytes(
        mut self,
        max_message_size_bytes: usize,
    ) -> Self {
        self.max_message_size_bytes = max_message_size_bytes;
        self
    }

    pub(super) fn with_source_frames(mut self, source_frames: Vec<RealtimeMessage>) -> Self {
        self.source_frames = source_frames;
        self
    }

    pub(super) fn with_backend_send_error(mut self, backend_send_error: Option<String>) -> Self {
        self.backend_send_error = backend_send_error;
        self
    }

    pub(super) fn with_backend_close_error(mut self, backend_close_error: Option<String>) -> Self {
        self.backend_close_error = backend_close_error;
        self
    }

    pub(super) fn with_close_usage(mut self, close_usage: Option<AudioTranscriptionUsage>) -> Self {
        self.close_usage = close_usage;
        self
    }
}

pub(super) struct Harness<'callback> {
    provider: Arc<TestProvider>,
    dialer: ScriptedDialer,
    callback_refs: Vec<&'callback TestCallback>,
    client: ClientHandle,
    relay: RecordingRelay,
    redactor: Redactor,
    cancellation: CancellationToken,
    headers: HeaderMap,
    metadata: Option<IndexMap<String, Json>>,
    query_params: Option<IndexMap<String, String>>,
    verification: Verification,
    max_message_size_bytes: usize,
    records: Arc<Mutex<Vec<DialRecord>>>,
    open_headers: Arc<Mutex<Vec<HeaderMap>>>,
    client_log: Arc<Mutex<ClientLog>>,
    backend_log: Arc<Mutex<BackendLog>>,
    pub(super) observation: Arc<Mutex<RelayObservation>>,
}

#[derive(Default)]
struct SharedLogs {
    backend: Arc<Mutex<BackendLog>>,
    client: Arc<Mutex<ClientLog>>,
    records: Arc<Mutex<Vec<DialRecord>>>,
    open_headers: Arc<Mutex<Vec<HeaderMap>>>,
    observation: Arc<Mutex<RelayObservation>>,
}

fn provider_of(options: &HarnessOptions, logs: &SharedLogs) -> Arc<TestProvider> {
    Arc::new(TestProvider {
        requires_setup: options.requires_setup.0,
        setup: options.setup.clone(),
        synthetic: options.synthetic.clone(),
        own_backend: options.own_backend.0,
        validate_error: options.validate_error.clone(),
        open_backend_error: options.open_backend_error.clone(),
        open_headers: Arc::clone(&logs.open_headers),
        backend_log: Arc::clone(&logs.backend),
        send_error: options.backend_send_error.clone(),
        close_error: options.backend_close_error.clone(),
        close_usage: options.close_usage.clone(),
    })
}

fn client_of(options: &HarnessOptions, logs: &SharedLogs) -> ClientHandle {
    let socket: Box<dyn RealtimeClientSocket> = Box::new(FakeClient::new(options, logs));
    ClientHandle {
        socket: Arc::new(TokioMutex::new(socket)),
        scope: WebSocketScope {
            headers: options.client_scope_headers.clone(),
        },
    }
}

pub(super) fn client_handle(options: &HarnessOptions) -> ClientHandle {
    client_of(options, &SharedLogs::default())
}

fn dialer_of(
    dials: Vec<DialOutcome>,
    source_frames: Vec<RealtimeMessage>,
    send_error: Option<String>,
    close_error: Option<String>,
    logs: &SharedLogs,
) -> ScriptedDialer {
    ScriptedDialer {
        outcomes: Mutex::new(VecDeque::from(dials)),
        records: Arc::clone(&logs.records),
        backend_log: Arc::clone(&logs.backend),
        source_frames,
        send_error,
        close_error,
    }
}

fn relay_of(fails: bool, logs: &SharedLogs) -> RecordingRelay {
    RecordingRelay {
        observation: Arc::clone(&logs.observation),
        outcome: if fails {
            RelayOutcome::Fail
        } else {
            RelayOutcome::Ok
        },
    }
}

const VALIDATED_HEADER: &str = "x-provider-validated";
const VALIDATED_HEADER_VALUE: &str = "yes";

fn auth_headers() -> HeaderMap {
    HeaderMap::from([(
        "Authorization".to_owned(),
        Json::Text("Bearer test".to_owned()),
    )])
}

pub(super) fn harness(options: HarnessOptions, callback_refs: Vec<&TestCallback>) -> Harness<'_> {
    let logs = SharedLogs::default();
    let cancellation = CancellationToken::new();
    if options.cancellation.0 {
        cancellation.cancel();
    }
    let provider = provider_of(&options, &logs);
    let client = client_of(&options, &logs);
    let dialer = dialer_of(
        options.dials,
        options.source_frames,
        options.backend_send_error,
        options.backend_close_error,
        &logs,
    );
    Harness {
        provider,
        dialer,
        callback_refs,
        client,
        relay: relay_of(options.relay_fails.0, &logs),
        redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap(),
        cancellation,
        headers: auth_headers(),
        metadata: options.metadata,
        query_params: options.query_params,
        verification: options.verification,
        max_message_size_bytes: options.max_message_size_bytes,
        records: Arc::clone(&logs.records),
        open_headers: Arc::clone(&logs.open_headers),
        client_log: Arc::clone(&logs.client),
        backend_log: Arc::clone(&logs.backend),
        observation: Arc::clone(&logs.observation),
    }
}

pub(super) struct DialObservation {
    pub(super) url: String,
    pub(super) headers: HeaderMap,
    pub(super) ssl: RealtimeSsl,
    pub(super) max_size: usize,
    pub(super) open_timeout: Duration,
    pub(super) at: Instant,
}

impl Harness<'_> {
    fn configuration(&mut self) -> RealtimeCall<'_, TestProvider, TestCallback, RecordingRelay> {
        RealtimeCall {
            model: "gemini-test",
            api_base: Some("wss://upstream.test/realtime"),
            api_key: Some("key-1"),
            headers: &self.headers,
            provider: Arc::clone(&self.provider),
            dialer: &self.dialer,
            client: self.client.clone(),
            relay: &mut self.relay,
            callbacks: &self.callback_refs,
            logging: None,
            logging_worker: None,
            logging_session_id: "trace-1",
            user_api_key_dict: None,
            logged_real_time_event_types: LoggedEventTypes::Default,
            litellm_metadata: self.metadata.as_ref(),
            query_params: self.query_params.as_ref(),
            verification: self.verification.clone(),
            max_message_size_bytes: self.max_message_size_bytes,
            redactor: &self.redactor,
            cancellation: self.cancellation.clone(),
        }
    }

    pub(super) async fn run(
        &mut self,
    ) -> Result<(), RealtimeConnectionError<io::Error, Infallible, io::Error>> {
        self.configuration().async_realtime().await
    }

    pub(super) fn texts(&self) -> Vec<String> {
        self.client_log.lock().unwrap().texts.clone()
    }

    pub(super) fn close(&self) -> Option<(u16, Option<String>)> {
        self.client_log.lock().unwrap().close.clone()
    }

    pub(super) fn sent(&self) -> Vec<RealtimeMessage> {
        self.backend_log.lock().unwrap().sent.clone()
    }

    pub(super) fn closed(&self) -> bool {
        self.backend_log.lock().unwrap().closed
    }

    pub(super) fn dials(&self) -> Vec<DialObservation> {
        let records = self.records.lock().unwrap();
        records
            .iter()
            .map(|entry| DialObservation {
                url: entry.url.clone(),
                headers: entry.headers.clone(),
                ssl: entry.ssl.clone(),
                max_size: entry.max_size,
                open_timeout: entry.open_timeout,
                at: entry.at,
            })
            .collect()
    }

    pub(super) fn source_outcome(&self) -> SourceOutcome {
        self.observation.lock().unwrap().source_outcome.clone()
    }

    pub(super) fn open_backend_headers(&self) -> Vec<HeaderMap> {
        self.open_headers.lock().unwrap().clone()
    }
}

pub(super) fn text_of(message: &RealtimeMessage) -> &str {
    match message {
        RealtimeMessage::Text(text) => text,
        RealtimeMessage::Binary(_) => "",
    }
}

pub(super) fn parse(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}
