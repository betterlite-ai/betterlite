use std::collections::{BTreeMap, VecDeque};
use std::error::Error;
use std::fmt::Debug;
use std::future::{Future, ready};
use std::marker::PhantomData;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::tls::{TlsPolicy, Verification};
use betterlite_prompts::arguments::json::Object;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, Subscriber, set_default};
use tracing::{Event, Level};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::{Context, SubscriberExt as _};
use tracing_subscriber::registry;

use crate::client::cleanup::http_client::CleanupScope;
use crate::client::sessions::client::{SessionClient, SessionSettings};
use crate::client::sessions::error::{ExitFailure, McpFailure, SessionError};
use crate::client::sessions::factory::{HttpClientConfig, SseRequest};
use crate::client::sessions::negotiate::{
    Callbacks, ClientSession, InitializeOutcome, InitializeRequest, ProtocolVersion,
};
use crate::client::sessions::runtime::{
    HttpClientHandle, McpRuntime, SessionContext, SessionOptions,
};
use crate::client::sessions::transport::{
    StdioConfig, StdioServerParameters, TransportContext, TransportType,
};
use crate::client::tools::progress::{ProgressForwarder, ProgressHost};
use crate::client::tools::result::{
    CallToolResult, ContentBlock, InputRequiredResult, TextContent, ToolCallResult,
};
use crate::client::tools::session::ToolCallSession;
use crate::credentials::{McpAuthType, McpClient as CredentialClient};
use crate::tools::pagination::{McpSession, Page};
use crate::tools::schemas::McpTool;
use crate::tools::tool_calls::CallRequest;

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum TestError {
    #[error("upstream failure: {0}")]
    Boom(&'static str),
    #[error("the server returned an input_required result")]
    InputRequiredRejected,
}

// The SDK's exception classes are distinct types, so a broken connection is its own source type here
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("the connection is broken")]
pub(crate) struct BrokenResource;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ErrorScript<Source> {
    Runtime(Source),
    Protocol(McpFailure),
}

impl<Source> ErrorScript<Source>
where
    Source: Error + Send + Sync + 'static,
{
    fn into_session_error(self) -> SessionError<Source> {
        match self {
            Self::Runtime(source) => SessionError::Runtime(Arc::new(source)),
            Self::Protocol(failure) => SessionError::Protocol(failure),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Frame {
    pub(crate) delay: Duration,
    pub(crate) tools: Vec<McpTool>,
    pub(crate) next_cursor: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct ProgressEvent {
    pub(crate) progress: f64,
    pub(crate) total: Option<f64>,
    pub(crate) message: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) enum CallOutcome<Source> {
    Echo,
    Complete(CallToolResult),
    InputRequired(InputRequiredResult),
    Fail(ErrorScript<Source>),
}

#[derive(Clone, Debug)]
pub(crate) struct Script<Source> {
    pub(crate) pages: Vec<Frame>,
    pub(crate) list_error: Option<ErrorScript<Source>>,
    pub(crate) call: CallOutcome<Source>,
    pub(crate) progress: Vec<ProgressEvent>,
    pub(crate) hang_on_exit: bool,
}

impl<Source> Default for Script<Source> {
    fn default() -> Self {
        Self {
            pages: Vec::new(),
            list_error: None,
            call: CallOutcome::Echo,
            progress: Vec::new(),
            hang_on_exit: false,
        }
    }
}

#[must_use]
pub(crate) fn failing<Source>(error: Source) -> Script<Source> {
    Script {
        pages: Vec::new(),
        list_error: Some(ErrorScript::Runtime(error)),
        call: CallOutcome::Echo,
        progress: Vec::new(),
        hang_on_exit: false,
    }
}

#[must_use]
pub(crate) fn protocol_failure() -> ErrorScript<TestError> {
    ErrorScript::Protocol(McpFailure {
        code: -32_000,
        message: "Broken pipe while reading the response".to_owned(),
        context: None,
    })
}

#[must_use]
pub(crate) fn calling<Source>(
    outcome: CallOutcome<Source>,
    progress: Vec<ProgressEvent>,
) -> Script<Source> {
    Script {
        pages: Vec::new(),
        list_error: None,
        call: outcome,
        progress,
        hang_on_exit: false,
    }
}

#[must_use]
pub(crate) fn hanging<Source>(pages: Vec<Frame>, outcome: CallOutcome<Source>) -> Script<Source> {
    let mut script = calling(outcome, Vec::new());
    script.pages = pages;
    script.hang_on_exit = true;
    script
}

#[must_use]
pub(crate) fn text_result(text: &str) -> CallToolResult {
    CallToolResult {
        content: vec![ContentBlock::Text(TextContent {
            text: text.to_owned(),
            extra: Object::new(),
        })],
        is_error: false,
        structured_content: None,
        meta: None,
    }
}

#[must_use]
pub(crate) fn tool(name: &str) -> McpTool {
    McpTool {
        name: name.to_owned(),
        description: None,
        input_schema: Some(Object::new()),
        extra: Object::new(),
    }
}

#[must_use]
pub(crate) fn page(names: &[&str], next_cursor: Option<&str>) -> Frame {
    Frame {
        delay: Duration::ZERO,
        tools: names.iter().map(|name| tool(name)).collect(),
        next_cursor: next_cursor.map(str::to_owned),
    }
}

pub(crate) struct FakeRuntime<Source> {
    script: Script<Source>,
}

pub(crate) struct FakeTransport<Source>(PhantomData<Source>);

impl<Source> TransportContext for FakeTransport<Source>
where
    Source: Error + Send + Sync + 'static,
{
    type Streams = ();
    type Error = Source;

    fn enter(&mut self) -> impl Future<Output = Result<(), Source>> + Send {
        ready(Ok(()))
    }

    fn exit(self) -> impl Future<Output = Result<(), ExitFailure<Source>>> + Send + 'static {
        ready(Ok(()))
    }
}

pub(crate) struct FakeHttpClient<Source>(PhantomData<Source>);

impl<Source> HttpClientHandle for FakeHttpClient<Source>
where
    Source: Error + Send + Sync + 'static,
{
    type Error = Source;

    fn begin_cleanup(&self, _scope: CleanupScope) {}

    fn finish_cleanup(&self) -> Vec<Arc<Source>> {
        Vec::new()
    }
}

pub(crate) struct FakeInit;

impl InitializeOutcome for FakeInit {
    fn protocol_version(&self) -> &'static str {
        "2025-06-18"
    }

    fn instructions(&self) -> Option<&str> {
        None
    }
}

pub(crate) struct FakeSession<Source> {
    script: Arc<Script<Source>>,
    pages: Mutex<VecDeque<Frame>>,
}

impl<Source> ClientSession for FakeSession<Source>
where
    Source: Error + Send + Sync + 'static,
{
    type Error = Source;
    type InitializeResult = FakeInit;

    fn initialize(&mut self) -> impl Future<Output = Result<FakeInit, Source>> + Send {
        ready(Ok(FakeInit))
    }

    fn send_initialize(
        &mut self,
        _request: InitializeRequest,
    ) -> impl Future<Output = Result<FakeInit, Source>> + Send {
        ready(Ok(FakeInit))
    }

    fn adopt(&mut self, _result: &FakeInit) {}

    fn send_initialized(&mut self) -> impl Future<Output = Result<(), Source>> + Send {
        ready(Ok(()))
    }
}

impl<Source> McpSession for FakeSession<Source>
where
    Source: Error + Send + Sync + Clone + 'static,
{
    type Error = SessionError<Source>;
    type Output = ToolCallResult;

    async fn list_tools(&mut self, _cursor: Option<&str>) -> Result<Page, Self::Error> {
        if let Some(error) = self.script.list_error.clone() {
            return Err(error.into_session_error());
        }
        let popped = self
            .pages
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        let Some(frame) = popped else {
            return Ok(Page::default());
        };
        sleep(frame.delay).await;
        Ok(Page {
            tools: frame.tools,
            next_cursor: frame.next_cursor,
        })
    }

    fn call_tool(
        &mut self,
        _request: CallRequest,
    ) -> impl Future<Output = Result<ToolCallResult, SessionError<Source>>> + Send {
        ready(Ok(ToolCallResult::Complete(text_result("unused"))))
    }
}

impl<Source> ToolCallSession for FakeSession<Source>
where
    Source: Error + Send + Sync + Clone + FromTestError + 'static,
{
    type Error = SessionError<Source>;

    async fn call_tool<Host>(
        &mut self,
        request: &CallRequest,
        progress: &ProgressForwarder<'_, Host>,
        allow_input_required: bool,
    ) -> Result<ToolCallResult, SessionError<Source>>
    where
        Host: ProgressHost + Sync,
    {
        for event in &self.script.progress {
            progress
                .report(event.progress, event.total, event.message.as_deref())
                .await;
        }
        match &self.script.call {
            CallOutcome::Echo => Ok(ToolCallResult::Complete(text_result(&format!(
                "{}:{:?}",
                request.name, request.arguments
            )))),
            CallOutcome::Complete(result) => Ok(ToolCallResult::Complete(result.clone())),
            CallOutcome::InputRequired(result) if allow_input_required => {
                Ok(ToolCallResult::InputRequired(result.clone()))
            }
            CallOutcome::InputRequired(_) => Err(SessionError::Runtime(Arc::new(
                Source::from_test_error(TestError::InputRequiredRejected),
            ))),
            CallOutcome::Fail(error) => Err(error.clone().into_session_error()),
        }
    }
}

pub(crate) struct FakeContext<Source> {
    script: Arc<Script<Source>>,
}

impl<Source> SessionContext for FakeContext<Source>
where
    Source: Error + Send + Sync + 'static,
{
    type Session = FakeSession<Source>;
    type Error = Source;

    fn enter(&mut self) -> impl Future<Output = Result<FakeSession<Source>, Source>> + Send {
        let script = Arc::clone(&self.script);
        let pages = Mutex::new(VecDeque::from(self.script.pages.clone()));
        async move { Ok(FakeSession { script, pages }) }
    }

    fn exit(self) -> impl Future<Output = Result<(), ExitFailure<Source>>> + Send + 'static {
        let hang = self.script.hang_on_exit;
        async move {
            if hang {
                sleep(Duration::from_secs(60)).await;
            }
            Ok(())
        }
    }
}

impl<Source> McpRuntime for FakeRuntime<Source>
where
    Source: Error + Send + Sync + Clone + 'static,
{
    type Error = Source;
    type Streams = ();
    type Transport = FakeTransport<Source>;
    type HttpClient = FakeHttpClient<Source>;
    type Session = FakeContext<Source>;

    fn stdio_client(&self, _parameters: StdioServerParameters) -> FakeTransport<Source> {
        FakeTransport(PhantomData)
    }

    fn sse_client(&self, _request: SseRequest) -> FakeTransport<Source> {
        FakeTransport(PhantomData)
    }

    fn streamable_http_client(
        &self,
        _url: String,
        _client: Arc<FakeHttpClient<Source>>,
    ) -> FakeTransport<Source> {
        FakeTransport(PhantomData)
    }

    fn http_client(&self, _config: HttpClientConfig) -> Result<FakeHttpClient<Source>, Source> {
        Ok(FakeHttpClient(PhantomData))
    }

    fn session_context(
        &self,
        _streams: (),
        _options: SessionOptions<Source>,
    ) -> FakeContext<Source> {
        FakeContext {
            script: Arc::new(self.script.clone()),
        }
    }

    fn close_http_client(
        self: Arc<Self>,
        _client: Arc<FakeHttpClient<Source>>,
    ) -> impl Future<Output = Result<(), ExitFailure<Source>>> + Send + 'static {
        ready(Ok(()))
    }
}

// The rejection is the runtime's own error type, so a source that differs from TestError maps it to its closest value
trait FromTestError: Sized {
    fn from_test_error(error: TestError) -> Self;
}

impl FromTestError for TestError {
    fn from_test_error(error: Self) -> Self {
        error
    }
}

impl FromTestError for BrokenResource {
    fn from_test_error(_error: TestError) -> Self {
        Self
    }
}

fn tls_policy() -> TlsPolicy {
    TlsPolicy {
        verification: Verification::Enabled,
        ca_file: None,
        certificate: None,
        http2: false,
        ecdh_curve: None,
        user_agent: "betterlite-test".to_owned(),
    }
}

pub(crate) fn build_client<Source>(
    script: Script<Source>,
    timeout: Duration,
    cancelled: CancellationToken,
) -> SessionClient<FakeRuntime<Source>>
where
    Source: Error + Send + Sync + Clone + 'static,
{
    SessionClient {
        runtime: Arc::new(FakeRuntime { script }),
        auth: CredentialClient::new(
            "https://mcp.example.com/mcp".to_owned(),
            McpAuthType::None,
            None,
            None,
            BTreeMap::new(),
            timeout,
            None,
            None,
        ),
        settings: SessionSettings {
            transport_type: TransportType::Stdio,
            stdio: Some(StdioConfig {
                command: "server".to_owned(),
                args: Vec::new(),
                env: None,
            }),
            environment: BTreeMap::new(),
            npm_cache_dir: "/tmp/npm".to_owned(),
            tls: tls_policy(),
            protocol_version: ProtocolVersion::Auto,
            client_version: "1.105.1".to_owned(),
            callbacks: Callbacks::default(),
        },
        tracker: TaskTracker::new(),
        caller_cancelled: cancelled,
        last_initialize_instructions: None,
    }
}

#[derive(Default)]
pub(crate) struct RecordingHost {
    pub(crate) seen: Mutex<Vec<(f64, Option<f64>)>>,
    pub(crate) fail_on: Option<usize>,
}

impl RecordingHost {
    #[must_use]
    pub(crate) fn failing_on(call: usize) -> Self {
        Self {
            seen: Mutex::new(Vec::new()),
            fail_on: Some(call),
        }
    }

    pub(crate) fn seen(&self) -> Vec<(f64, Option<f64>)> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ProgressHost for RecordingHost {
    type Error = TestError;

    fn report(
        &self,
        progress: f64,
        total: Option<f64>,
    ) -> impl Future<Output = Result<(), TestError>> + Send {
        let failed = {
            let mut seen = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
            seen.push((progress, total));
            self.fail_on == Some(seen.len())
        };
        ready(if failed {
            Err(TestError::Boom("host"))
        } else {
            Ok(())
        })
    }
}

#[derive(Clone, Default)]
pub(crate) struct Logs {
    events: Arc<Mutex<Vec<(Level, String)>>>,
}

impl Logs {
    pub(crate) fn events(&self) -> Vec<(Level, String)> {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn messages(&self) -> Vec<String> {
        self.events()
            .into_iter()
            .map(|(_, message)| message)
            .collect()
    }

    pub(crate) fn count(&self, level: Level) -> usize {
        self.events()
            .into_iter()
            .filter(|(seen, _)| *seen == level)
            .count()
    }

    pub(crate) fn matching(&self, needle: &str) -> Vec<String> {
        self.messages()
            .into_iter()
            .filter(|message| message.contains(needle))
            .collect()
    }
}

struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Current: Subscriber> Layer<Current> for Logs {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Current>) {
        let mut message = Message(String::new());
        event.record(&mut message);
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((*event.metadata().level(), message.0));
    }
}

pub(crate) fn capture() -> (Logs, DefaultGuard) {
    let logs = Logs::default();
    let subscriber = registry().with(logs.clone());
    let guard = set_default(subscriber);
    (logs, guard)
}
