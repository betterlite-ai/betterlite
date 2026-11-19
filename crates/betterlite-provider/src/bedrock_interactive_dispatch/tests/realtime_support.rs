use super::*;

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use async_trait::async_trait;
use betterlite_bedrock::agentcore_stream::IdSource;
use betterlite_bedrock::aws_settings::AuthParams;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock::realtime_events::client::{ClientError, RealtimeClientWebSocket};
use betterlite_bedrock::realtime_events::scope::EventScope;
use betterlite_bedrock::realtime_events::stream::{
    BedrockBidirectionalStream, BedrockInputStream, BedrockOutputChunk, BedrockOutputStream,
    ClosableBedrockRuntimeClient, StreamError,
};
use betterlite_bedrock::realtime_transport::error::{ProviderClass, RealtimeTransportError};
use betterlite_bedrock::realtime_transport::prepare::RealtimeCredentials;
use betterlite_bedrock::realtime_transport::runtime::{
    RealtimeClient, RealtimeRuntime, RuntimeConfig,
};
use betterlite_bedrock::realtime_transport::session::RealtimeCall;
use betterlite_config::api::realtime_session_state::LoggedEventTypes;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_core::api::realtime_session_state::logging::{
    RealtimeLogging, SessionInput, SuccessWork,
};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};

#[derive(Debug)]
struct TestError {
    class: &'static str,
    message: &'static str,
}

impl TestError {
    const fn new(class: &'static str, message: &'static str) -> Self {
        Self { class, message }
    }
}

impl std::fmt::Display for TestError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message)
    }
}

impl std::error::Error for TestError {}

impl ProviderClass for TestError {
    fn provider_class_name(&self) -> Option<&str> {
        Some(self.class)
    }

    fn provider_message(&self) -> Option<&str> {
        Some(self.message)
    }
}

#[derive(Default)]
struct Shared {
    input_sent: Mutex<Vec<Json>>,
    input_closed: AtomicBool,
    runtime_closed: AtomicBool,
}

struct TestRuntime {
    shared: Arc<Shared>,
    fail_connect: bool,
    configs: Mutex<Vec<RuntimeConfig>>,
}

impl TestRuntime {
    fn new(shared: Arc<Shared>, fail_connect: bool) -> Self {
        Self {
            shared,
            fail_connect,
            configs: Mutex::new(Vec::new()),
        }
    }
}

impl RealtimeRuntime for TestRuntime {
    type Error = TestError;

    fn connect(
        &self,
        config: RuntimeConfig,
    ) -> impl Future<
        Output = Result<Box<dyn RealtimeClient<Error = TestError>>, StreamError<TestError>>,
    > + Send {
        self.configs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(config);
        let outcome: Result<Box<dyn RealtimeClient<Error = TestError>>, StreamError<TestError>> =
            if self.fail_connect {
                Err(StreamError {
                    message: "the websocket could not open".to_owned(),
                    source: Some(TestError::new("WebSocketError", "open failed")),
                })
            } else {
                Ok(Box::new(TestClient {
                    closer: TestCloser {
                        shared: Arc::clone(&self.shared),
                    },
                    shared: Arc::clone(&self.shared),
                }))
            };
        ready(outcome)
    }
}

struct TestClient {
    closer: TestCloser,
    shared: Arc<Shared>,
}

#[async_trait]
impl RealtimeClient for TestClient {
    type Error = TestError;

    fn closable(&self) -> &dyn ClosableBedrockRuntimeClient<Error = TestError> {
        &self.closer
    }

    async fn invoke(
        &self,
        _model_id: &str,
    ) -> Result<Box<dyn BedrockBidirectionalStream<Error = TestError>>, StreamError<TestError>>
    {
        Ok(Box::new(TestStream {
            input: TestInput {
                shared: Arc::clone(&self.shared),
            },
        }))
    }
}

struct TestCloser {
    shared: Arc<Shared>,
}

#[async_trait]
impl ClosableBedrockRuntimeClient for TestCloser {
    type Error = TestError;

    async fn close(&self) -> Result<(), StreamError<TestError>> {
        self.shared.runtime_closed.store(true, Ordering::SeqCst);
        Ok(())
    }
}

struct TestInput {
    shared: Arc<Shared>,
}

#[async_trait]
impl BedrockInputStream for TestInput {
    type Error = TestError;

    async fn send(&self, event: Json) -> Result<(), StreamError<TestError>> {
        self.shared
            .input_sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
        Ok(())
    }

    async fn close(&self) -> Result<(), StreamError<TestError>> {
        self.shared.input_closed.store(true, Ordering::SeqCst);
        Ok(())
    }
}

struct TestStream {
    input: TestInput,
}

#[async_trait]
impl BedrockBidirectionalStream for TestStream {
    type Error = TestError;

    fn input_stream(&self) -> &dyn BedrockInputStream<Error = TestError> {
        &self.input
    }

    async fn await_output(
        &self,
    ) -> Result<(Json, Box<dyn BedrockOutputStream<Error = TestError>>), StreamError<TestError>>
    {
        Ok((Json::Object(IndexMap::new()), Box::new(TestOutput)))
    }
}

struct TestOutput;

#[async_trait]
impl BedrockOutputStream for TestOutput {
    type Error = TestError;

    async fn receive(&self) -> Result<Option<Box<dyn BedrockOutputChunk>>, StreamError<TestError>> {
        Ok(None)
    }
}

struct TestSocket {
    sent: Mutex<Vec<String>>,
    closed: Mutex<Option<u16>>,
}

impl TestSocket {
    fn new() -> Self {
        Self {
            sent: Mutex::new(Vec::new()),
            closed: Mutex::new(None),
        }
    }

    fn sent(&self) -> Vec<String> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn close_code(&self) -> Option<u16> {
        *self.closed.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[async_trait]
impl RealtimeClientWebSocket for TestSocket {
    type Error = io::Error;

    async fn receive_text(&self) -> Result<String, ClientError<io::Error>> {
        Err(ClientError {
            message: "the client left".to_owned(),
            source: None,
        })
    }

    async fn send_text(&self, data: String) -> Result<(), ClientError<io::Error>> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(data);
        Ok(())
    }

    async fn close(&self, code: u16, _reason: Option<String>) {
        *self.closed.lock().unwrap_or_else(PoisonError::into_inner) = Some(code);
    }
}

struct TestLoggingRealtime;

impl RealtimeLogging for TestLoggingRealtime {
    fn pre_call(&self, _input: &SessionInput<'_>, _api_key: &str) {}

    fn record_model_call_detail(&self, _key: &str, _value: Json) {}

    fn dispatch_success_handlers(&self, _messages: &[IndexMap<String, Json>]) -> SuccessWork {
        Box::pin(async {})
    }
}

struct TestResolver;

impl RealtimeCredentials for TestResolver {
    fn resolve(
        &self,
        _settings: &AwsSettings,
        _region: &str,
    ) -> impl Future<Output = Result<Option<Credentials>, CredentialError>> + Send {
        ready(Ok(Some(credentials())))
    }
}

struct CountingIds {
    next: AtomicU64,
}

impl CountingIds {
    fn new() -> Self {
        Self {
            next: AtomicU64::new(1),
        }
    }
}

impl IdSource for CountingIds {
    fn next_id(&self) -> uuid::Uuid {
        let next = self.next.fetch_add(1, Ordering::SeqCst);
        uuid::Uuid::from_u128(u128::from(next))
    }
}

struct SessionFixture {
    socket: TestSocket,
    runtime: TestRuntime,
    resolver: TestResolver,
    ids: CountingIds,
    redactor: Redactor,
    logging: TestLoggingRealtime,
    logged_types: LoggedEventTypes,
    shared: Arc<Shared>,
}

impl SessionFixture {
    fn new(fail_connect: bool) -> Self {
        let shared = Arc::new(Shared::default());
        Self {
            socket: TestSocket::new(),
            runtime: TestRuntime::new(Arc::clone(&shared), fail_connect),
            resolver: TestResolver,
            ids: CountingIds::new(),
            redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap(),
            logging: TestLoggingRealtime,
            logged_types: LoggedEventTypes::Default,
            shared,
        }
    }

    fn call<'call>(&'call self, scope: &'call mut EventScope) -> RealtimeCall<'call, io::Error> {
        RealtimeCall {
            model: "amazon.nova-sonic-v1:0",
            client: &self.socket,
            logging: &self.logging,
            logging_worker: None,
            api_base: None,
            aws_bedrock_runtime_endpoint: None,
            aws_region_name: Some("us-east-1"),
            auth: AuthParams::default(),
            region_inputs: RegionInputs::default(),
            scope,
            ids: &self.ids,
            redactor: &self.redactor,
            logged_types: &self.logged_types,
            trace_id: "trace-interactive",
        }
    }
}

fn pending_scope() -> EventScope {
    EventScope {
        pending_session_update: Json::Text(SESSION_UPDATE.to_owned()),
        ..EventScope::default()
    }
}

fn realtime_pending_preface() {
    let fixture = SessionFixture::new(false);
    let mut scope = pending_scope();
    let outcome = block_on(realtime_bridge::session(
        &fixture.runtime,
        &fixture.resolver,
        fixture.call(&mut scope),
    ));
    assert!(
        outcome.is_ok(),
        "{:?}",
        outcome.err().map(|error| error.to_string())
    );
    let sent = fixture.socket.sent();
    assert!(
        sent.iter()
            .any(|frame| frame.contains("\"session.updated\""))
    );
    assert!(
        !sent
            .iter()
            .any(|frame| frame.contains("\"session.created\""))
    );
    assert_eq!(scope.session_committed, Json::Bool(true));
    let frames = fixture
        .shared
        .input_sent
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .len();
    assert!(frames > 0);
    assert!(fixture.shared.input_closed.load(Ordering::SeqCst));
    assert!(fixture.shared.runtime_closed.load(Ordering::SeqCst));
    assert_eq!(fixture.socket.close_code(), Some(1000));
}

fn realtime_connect_failure() {
    let fixture = SessionFixture::new(true);
    let mut scope = pending_scope();
    let error = block_on(realtime_bridge::session(
        &fixture.runtime,
        &fixture.resolver,
        fixture.call(&mut scope),
    ))
    .unwrap_err();
    let realtime_bridge::Error::Session { source } = error else {
        panic!("expected a session failure");
    };
    let transport = source
        .downcast_ref::<RealtimeTransportError<TestError, io::Error>>()
        .expect("the transport error is preserved");
    assert!(matches!(transport, RealtimeTransportError::Stream(_)));
    assert_eq!(transport.status_code(), None);
    assert!(fixture.socket.sent().is_empty());
    assert!(!fixture.shared.runtime_closed.load(Ordering::SeqCst));
    assert_eq!(
        scope.pending_session_update,
        Json::Text(SESSION_UPDATE.to_owned())
    );
}

pub(super) fn realtime_case() {
    realtime_pending_preface();
    realtime_connect_failure();
}
