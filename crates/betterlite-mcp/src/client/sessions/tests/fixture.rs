use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, pending};
use std::mem::take;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::tls::{TlsPolicy, Verification};
use thiserror::Error;
use tokio::time::{Instant, sleep};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::client::cleanup::http_client::CleanupScope;
use crate::client::sessions::client::{SessionClient, SessionSettings};
use crate::client::sessions::error::{ExitFailure, McpFailure, SessionError};
use crate::client::sessions::factory::{HttpClientConfig, SseRequest};
use crate::client::sessions::negotiate::{
    Callback, Callbacks, ClientSession, InitializeOutcome, InitializeRequest, LegacyVersion,
    ProtocolVersion,
};
use crate::client::sessions::runtime::{
    HttpClientHandle, McpRuntime, SessionContext, SessionOptions,
};
use crate::client::sessions::stream::MessageHandler;
use crate::client::sessions::transport::{
    StdioConfig, StdioServerParameters, TransportContext, TransportContexts, TransportType,
};
use crate::credentials::{
    AuthValue, CredentialError, McpAuthType, McpClient as CredentialClient, Request, ResolvedAuth,
};

pub(crate) const CLIENT_VERSION: &str = "1.105.1";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Step {
    #[default]
    Ok,
    Fail,
    Cancel,
    Timeout,
    RecordedFailure,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum InitStep {
    #[default]
    Ok,
    Fail,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum TestError {
    #[error("boom: {0}")]
    Boom(&'static str),
    #[error("status {0}")]
    Status(u16),
}

#[derive(Clone, Default)]
pub(crate) struct Script {
    pub(crate) calls: Arc<Mutex<Vec<String>>>,
    pub(crate) requests: Arc<Mutex<Vec<InitializeRequest>>>,
}

impl Script {
    pub(crate) fn record(&self, name: &str) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(name.to_owned());
    }

    pub(crate) fn calls(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn requests(&self) -> Vec<InitializeRequest> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Plan {
    pub(crate) transport_enter: Step,
    pub(crate) transport_exit: Step,
    pub(crate) session_enter: Step,
    pub(crate) session_exit: Step,
    pub(crate) initialize: InitStep,
    pub(crate) server_version: String,
    pub(crate) instructions: Option<String>,
    pub(crate) http_close: Step,
}

impl Default for Plan {
    fn default() -> Self {
        Self {
            transport_enter: Step::Ok,
            transport_exit: Step::Ok,
            session_enter: Step::Ok,
            session_exit: Step::Ok,
            initialize: InitStep::Ok,
            server_version: "2025-06-18".to_owned(),
            instructions: None,
            http_close: Step::Ok,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RecordedScope {
    pub(crate) shield: bool,
    pub(crate) deadline: Instant,
}

#[derive(Default)]
struct HttpState {
    scopes: Mutex<Vec<RecordedScope>>,
    errors: Mutex<Vec<Arc<TestError>>>,
    closed: Mutex<bool>,
    finished: Mutex<usize>,
}

#[derive(Clone, Default)]
pub(crate) struct FakeHttpClient {
    state: Arc<HttpState>,
}

impl FakeHttpClient {
    pub(crate) fn scopes(&self) -> Vec<RecordedScope> {
        self.state
            .scopes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn closed(&self) -> bool {
        *self
            .state
            .closed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn finished(&self) -> usize {
        *self
            .state
            .finished
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn record_error(&self, error: Arc<TestError>) {
        self.state
            .errors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(error);
    }
}

impl HttpClientHandle for FakeHttpClient {
    type Error = TestError;

    fn begin_cleanup(&self, scope: CleanupScope) {
        self.state
            .scopes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedScope {
                shield: scope.shield,
                deadline: scope.deadline,
            });
    }

    fn finish_cleanup(&self) -> Vec<Arc<TestError>> {
        let mut finished = self
            .state
            .finished
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        *finished = finished.saturating_add(1);
        drop(finished);
        take(
            &mut *self
                .state
                .errors
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        )
    }
}

fn record_failure(http_client: Option<&FakeHttpClient>, error: &Arc<TestError>) {
    if let Some(client) = http_client {
        client.record_error(Arc::clone(error));
    }
}

pub(crate) struct FakeTransport {
    script: Script,
    enter: Step,
    exit: Step,
    http_client: Option<FakeHttpClient>,
}

impl FakeTransport {
    pub(crate) fn new(script: Script, plan: &Plan, http_client: Option<FakeHttpClient>) -> Self {
        Self {
            script,
            enter: plan.transport_enter,
            exit: plan.transport_exit,
            http_client,
        }
    }
}

impl TransportContext for FakeTransport {
    type Streams = ();
    type Error = TestError;

    fn enter(&mut self) -> impl Future<Output = Result<Self::Streams, Self::Error>> + Send {
        let script = self.script.clone();
        let step = self.enter;
        async move {
            script.record("transport.enter");
            match step {
                Step::Fail => Err(TestError::Boom("transport enter")),
                Step::Cancel => pending::<Result<(), TestError>>().await,
                Step::Ok | Step::Timeout | Step::RecordedFailure => Ok(()),
            }
        }
    }

    fn exit(self) -> impl Future<Output = Result<(), ExitFailure<TestError>>> + Send + 'static {
        let script = self.script.clone();
        let step = self.exit;
        let http_client = self.http_client;
        async move {
            script.record("transport.exit");
            match step {
                Step::Ok => Ok(()),
                Step::Cancel => Err(ExitFailure::Cancelled),
                Step::Timeout => {
                    sleep(Duration::from_secs(60)).await;
                    Ok(())
                }
                Step::Fail => Err(ExitFailure::Failure(Arc::new(TestError::Boom(
                    "transport exit",
                )))),
                Step::RecordedFailure => {
                    let error = Arc::new(TestError::Boom("recorded cleanup error"));
                    record_failure(http_client.as_ref(), &error);
                    Err(ExitFailure::Group(vec![ExitFailure::Failure(error)]))
                }
            }
        }
    }
}

pub(crate) struct FakeInit {
    pub(crate) version: String,
    pub(crate) instructions: Option<String>,
}

impl InitializeOutcome for FakeInit {
    fn protocol_version(&self) -> &str {
        &self.version
    }

    fn instructions(&self) -> Option<&str> {
        self.instructions.as_deref()
    }
}

pub(crate) struct FakeSession {
    pub(crate) script: Script,
    pub(crate) plan: Plan,
    pub(crate) handler: MessageHandler,
}

impl ClientSession for FakeSession {
    type Error = TestError;
    type InitializeResult = FakeInit;

    fn initialize(&mut self) -> impl Future<Output = Result<FakeInit, TestError>> + Send {
        let script = self.script.clone();
        let plan = self.plan.clone();
        async move {
            script.record("initialize");
            match plan.initialize {
                InitStep::Ok => Ok(FakeInit {
                    version: plan.server_version,
                    instructions: plan.instructions,
                }),
                InitStep::Fail => Err(TestError::Boom("initialize")),
            }
        }
    }

    fn send_initialize(
        &mut self,
        request: InitializeRequest,
    ) -> impl Future<Output = Result<FakeInit, TestError>> + Send {
        let script = self.script.clone();
        let plan = self.plan.clone();
        async move {
            script.record("send_initialize");
            script
                .requests
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(request);
            Ok(FakeInit {
                version: plan.server_version,
                instructions: plan.instructions,
            })
        }
    }

    fn adopt(&mut self, _result: &FakeInit) {
        self.script.record("adopt");
    }

    fn send_initialized(&mut self) -> impl Future<Output = Result<(), TestError>> + Send {
        let script = self.script.clone();
        async move {
            script.record("send_initialized");
            Ok(())
        }
    }
}

pub(crate) struct FakeSessionContext {
    script: Script,
    plan: Plan,
    handler: MessageHandler,
}

impl SessionContext for FakeSessionContext {
    type Session = FakeSession;
    type Error = TestError;

    fn enter(&mut self) -> impl Future<Output = Result<FakeSession, TestError>> + Send {
        let script = self.script.clone();
        let plan = self.plan.clone();
        let handler = self.handler.clone();
        async move {
            script.record("session.enter");
            match plan.session_enter {
                Step::Fail => Err(TestError::Boom("session enter")),
                Step::Ok | Step::Cancel | Step::Timeout | Step::RecordedFailure => {
                    Ok(FakeSession {
                        script,
                        plan,
                        handler,
                    })
                }
            }
        }
    }

    fn exit(self) -> impl Future<Output = Result<(), ExitFailure<TestError>>> + Send + 'static {
        let script = self.script.clone();
        let step = self.plan.session_exit;
        async move {
            script.record("session.exit");
            match step {
                Step::Ok => Ok(()),
                Step::Cancel => Err(ExitFailure::Cancelled),
                Step::Timeout => {
                    sleep(Duration::from_secs(60)).await;
                    Ok(())
                }
                Step::Fail => Err(ExitFailure::Failure(Arc::new(TestError::Boom(
                    "session exit",
                )))),
                Step::RecordedFailure => Err(ExitFailure::Group(vec![ExitFailure::Cancelled])),
            }
        }
    }
}

#[derive(Default)]
pub(crate) struct RuntimeState {
    stdio: Mutex<Option<StdioServerParameters>>,
    sse: Mutex<Option<SseRequest>>,
    http_configs: Mutex<Vec<HttpClientConfig>>,
    urls: Mutex<Vec<String>>,
    read_timeouts: Mutex<Vec<Duration>>,
}

pub(crate) struct FakeRuntime {
    pub(crate) script: Script,
    pub(crate) plan: Plan,
    pub(crate) http_client: FakeHttpClient,
    pub(crate) state: RuntimeState,
}

impl FakeRuntime {
    pub(crate) fn new(plan: Plan) -> Self {
        Self {
            script: Script::default(),
            plan,
            http_client: FakeHttpClient::default(),
            state: RuntimeState::default(),
        }
    }

    pub(crate) fn stdio(&self) -> Option<StdioServerParameters> {
        self.state
            .stdio
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn sse(&self) -> Option<SseRequest> {
        self.state
            .sse
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn http_configs(&self) -> Vec<HttpClientConfig> {
        self.state
            .http_configs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn urls(&self) -> Vec<String> {
        self.state
            .urls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl McpRuntime for FakeRuntime {
    type Error = TestError;
    type Streams = ();
    type Transport = FakeTransport;
    type HttpClient = FakeHttpClient;
    type Session = FakeSessionContext;

    fn stdio_client(&self, parameters: StdioServerParameters) -> FakeTransport {
        self.script.record("stdio_client");
        *self
            .state
            .stdio
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(parameters);
        FakeTransport::new(self.script.clone(), &self.plan, None)
    }

    fn sse_client(&self, request: SseRequest) -> FakeTransport {
        self.script.record("sse_client");
        *self
            .state
            .sse
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(request);
        FakeTransport::new(self.script.clone(), &self.plan, None)
    }

    fn streamable_http_client(&self, url: String, client: Arc<FakeHttpClient>) -> FakeTransport {
        self.script.record("streamable_http_client");
        self.state
            .urls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(url);
        FakeTransport::new(
            self.script.clone(),
            &self.plan,
            Some(FakeHttpClient::clone(&client)),
        )
    }

    fn http_client(&self, config: HttpClientConfig) -> Result<FakeHttpClient, TestError> {
        self.script.record("http_client");
        self.state
            .http_configs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(config);
        Ok(self.http_client.clone())
    }

    fn session_context(&self, _streams: (), options: SessionOptions) -> FakeSessionContext {
        self.script.record("session_context");
        self.state
            .read_timeouts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(options.read_timeout);
        FakeSessionContext {
            script: self.script.clone(),
            plan: self.plan.clone(),
            handler: options.message_handler,
        }
    }

    fn close_http_client(
        self: Arc<Self>,
        client: Arc<FakeHttpClient>,
    ) -> impl Future<Output = Result<(), ExitFailure<TestError>>> + Send + 'static {
        self.script.record("http.close");
        let step = self.plan.http_close;
        async move {
            *client
                .state
                .closed
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = true;
            match step {
                Step::Ok => Ok(()),
                Step::Cancel => Err(ExitFailure::Cancelled),
                Step::Timeout => {
                    sleep(Duration::from_secs(60)).await;
                    Ok(())
                }
                Step::Fail | Step::RecordedFailure => Err(ExitFailure::Failure(Arc::new(
                    TestError::Boom("http close"),
                ))),
            }
        }
    }
}

pub(crate) struct PresenceAuth {
    pub(crate) header_name: Option<String>,
}

impl ResolvedAuth for PresenceAuth {
    fn authenticate(&self, request: Request) -> Result<Request, CredentialError> {
        Ok(request)
    }

    fn close(&self) {}

    fn header_name(&self) -> Option<&str> {
        self.header_name.as_deref()
    }
}

pub(crate) fn tls_policy() -> TlsPolicy {
    TlsPolicy {
        verification: Verification::Enabled,
        ca_file: None,
        certificate: None,
        http2: false,
        ecdh_curve: None,
        user_agent: "betterlite-test".to_owned(),
    }
}

pub(crate) struct Config {
    pub(crate) server_url: String,
    pub(crate) transport_type: TransportType,
    pub(crate) stdio: Option<StdioConfig>,
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) npm_cache_dir: String,
    pub(crate) protocol_version: ProtocolVersion,
    pub(crate) callbacks: BTreeSet<Callback>,
    pub(crate) timeout: Duration,
    pub(crate) auth_type: McpAuthType,
    pub(crate) auth_value: Option<AuthValue>,
    pub(crate) auth_header_name: Option<String>,
    pub(crate) extra_headers: BTreeMap<String, String>,
    pub(crate) resolved_auth: Option<PresenceAuth>,
    pub(crate) aws_auth: Option<PresenceAuth>,
    pub(crate) tls: TlsPolicy,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server_url: "https://mcp.example.com/mcp".to_owned(),
            transport_type: TransportType::Http,
            stdio: None,
            environment: BTreeMap::new(),
            npm_cache_dir: "/tmp/npm-cache".to_owned(),
            protocol_version: ProtocolVersion::Auto,
            callbacks: BTreeSet::new(),
            timeout: Duration::from_secs(60),
            auth_type: McpAuthType::None,
            auth_value: None,
            auth_header_name: None,
            extra_headers: BTreeMap::new(),
            resolved_auth: None,
            aws_auth: None,
            tls: tls_policy(),
        }
    }
}

pub(crate) fn settings(config: &Config) -> SessionSettings {
    SessionSettings {
        transport_type: config.transport_type,
        stdio: config.stdio.clone(),
        environment: config.environment.clone(),
        npm_cache_dir: config.npm_cache_dir.clone(),
        tls: config.tls.clone(),
        protocol_version: config.protocol_version,
        client_version: CLIENT_VERSION.to_owned(),
        callbacks: Callbacks {
            kinds: config.callbacks.clone(),
        },
    }
}

pub(crate) fn make_plan<Setter>(set: Setter) -> Plan
where
    Setter: FnOnce(&mut Plan),
{
    let mut plan = Plan::default();
    set(&mut plan);
    plan
}

pub(crate) fn make_config<Setter>(set: Setter) -> Config
where
    Setter: FnOnce(&mut Config),
{
    let mut config = Config::default();
    set(&mut config);
    config
}

pub(crate) fn session(script: &Script, plan: Plan) -> FakeSession {
    FakeSession {
        script: script.clone(),
        plan,
        handler: MessageHandler::default(),
    }
}

pub(crate) fn manual_contexts(
    runtime: &FakeRuntime,
    with_http: bool,
) -> TransportContexts<FakeRuntime> {
    let http = with_http.then(|| runtime.http_client.clone());
    TransportContexts {
        transport: FakeTransport::new(runtime.script.clone(), &runtime.plan, http.clone()),
        http_client: http.map(Arc::new),
    }
}

pub(crate) fn build_client(
    runtime: FakeRuntime,
    config: Config,
    tracker: TaskTracker,
    caller_cancelled: CancellationToken,
) -> SessionClient<FakeRuntime> {
    let session_settings = settings(&config);
    let credentials = CredentialClient::new(
        config.server_url.clone(),
        config.auth_type,
        config.auth_value,
        config.auth_header_name,
        config.extra_headers,
        config.timeout,
        config
            .resolved_auth
            .map(|auth| -> Box<dyn ResolvedAuth> { Box::new(auth) }),
        config
            .aws_auth
            .map(|auth| -> Box<dyn ResolvedAuth> { Box::new(auth) }),
    );
    SessionClient {
        runtime: Arc::new(runtime),
        auth: credentials,
        settings: session_settings,
        tracker,
        caller_cancelled,
        last_initialize_instructions: None,
    }
}

pub(crate) fn legacy(value: &str) -> ProtocolVersion {
    ProtocolVersion::Legacy(match value {
        "2024-11-05" => LegacyVersion::Nov2024,
        "2025-03-26" => LegacyVersion::Mar2025,
        "2025-06-18" => LegacyVersion::Jun2025,
        _ => LegacyVersion::Nov2025,
    })
}

pub(crate) fn protocol_error(code: i32) -> SessionError<TestError> {
    SessionError::Protocol(McpFailure {
        code,
        message: "upstream error".to_owned(),
        context: None,
    })
}
