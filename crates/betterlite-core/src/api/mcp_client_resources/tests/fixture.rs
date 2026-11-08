#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::error::Error;
use std::future::{Future, ready};
use std::io;
use std::marker::PhantomData;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use betterlite_config::tls::{TlsPolicy, Verification};
use betterlite_mcp::client::cleanup::http_client::CleanupScope;
use betterlite_mcp::client::sessions::client::{SessionClient, SessionSettings};
use betterlite_mcp::client::sessions::error::{ExitFailure, McpFailure, SessionError};
use betterlite_mcp::client::sessions::factory::{HttpClientConfig, SseRequest};
use betterlite_mcp::client::sessions::negotiate::{
    Callbacks, ClientSession, InitializeOutcome, InitializeRequest, ProtocolVersion,
};
use betterlite_mcp::client::sessions::runtime::{
    HttpClientHandle, McpRuntime, SessionContext, SessionOptions,
};
use betterlite_mcp::client::sessions::transport::{
    StdioServerParameters, TransportContext, TransportType,
};
use betterlite_mcp::credentials::{McpAuthType, McpClient as CredentialClient};
use thiserror::Error;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::subscriber::{DefaultGuard, set_default};
use url::Url;

use crate::api::mcp_client_resources::session::ResourceSession;
use crate::api::mcp_client_resources::types::{
    GetPromptRequestParams, GetPromptResult, PromptPage, ReadResourceResult, ResourcePage,
    ResourceTemplatePage, ServerCapabilities,
};

pub(crate) trait FakeError: Error + Send + Sync + 'static {
    fn protocol(failure: McpFailure) -> Self;
    fn plain(message: &'static str) -> Self;
    fn protocol_failure(&self) -> Option<McpFailure>;
}

#[derive(Debug, Error)]
pub(crate) enum ScriptedFailure {
    #[error("MCP protocol error {0:?}")]
    Protocol(McpFailure),
    #[error("{0}")]
    Plain(&'static str),
}

impl FakeError for ScriptedFailure {
    fn protocol(failure: McpFailure) -> Self {
        Self::Protocol(failure)
    }

    fn plain(message: &'static str) -> Self {
        Self::Plain(message)
    }

    fn protocol_failure(&self) -> Option<McpFailure> {
        match self {
            Self::Protocol(failure) => Some(failure.clone()),
            Self::Plain(_) => None,
        }
    }
}

#[derive(Debug, Error)]
#[error("broken resource")]
pub(crate) struct BrokenResourceError;

impl FakeError for BrokenResourceError {
    fn protocol(_failure: McpFailure) -> Self {
        Self
    }

    fn plain(_message: &'static str) -> Self {
        Self
    }

    fn protocol_failure(&self) -> Option<McpFailure> {
        None
    }
}

pub(crate) enum Step<Value> {
    Ready(Value),
    Delayed { value: Value, delay: Duration },
    Protocol(McpFailure),
    Plain(&'static str),
}

#[derive(Default)]
pub(crate) struct Script {
    pub(crate) capabilities: Option<ServerCapabilities>,
    pub(crate) prompts: VecDeque<Step<PromptPage>>,
    pub(crate) resources: VecDeque<Step<ResourcePage>>,
    pub(crate) templates: VecDeque<Step<ResourceTemplatePage>>,
    pub(crate) prompt_result: Option<Step<GetPromptResult>>,
    pub(crate) resource_result: Option<Step<ReadResourceResult>>,
    pub(crate) cursors: Vec<Option<String>>,
    pub(crate) prompt_params: Vec<GetPromptRequestParams>,
    pub(crate) resource_uris: Vec<Url>,
    pub(crate) transport_fails: bool,
}

impl Script {
    pub(crate) fn with_capabilities(capabilities: Option<ServerCapabilities>) -> Self {
        Self {
            capabilities,
            prompts: VecDeque::new(),
            resources: VecDeque::new(),
            templates: VecDeque::new(),
            prompt_result: None,
            resource_result: None,
            cursors: Vec::new(),
            prompt_params: Vec::new(),
            resource_uris: Vec::new(),
            transport_fails: false,
        }
    }
}

pub(crate) struct FakeRuntime<Source> {
    pub(crate) script: Arc<Mutex<Script>>,
    marker: PhantomData<Source>,
}

impl<Source> FakeRuntime<Source> {
    pub(crate) fn new(script: Script) -> Self {
        Self {
            script: Arc::new(Mutex::new(script)),
            marker: PhantomData,
        }
    }
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) struct FakeTransport<Source> {
    script: Arc<Mutex<Script>>,
    marker: PhantomData<Source>,
}

impl<Source> TransportContext for FakeTransport<Source>
where
    Source: FakeError,
{
    type Streams = ();
    type Error = Source;

    fn enter(&mut self) -> impl Future<Output = Result<(), Source>> + Send {
        let fails = lock(&self.script).transport_fails;
        async move {
            if fails {
                Err(Source::plain("the scripted transport failed"))
            } else {
                Ok(())
            }
        }
    }

    fn exit(self) -> impl Future<Output = Result<(), ExitFailure<Source>>> + Send + 'static {
        ready(Ok(()))
    }
}

#[derive(Default)]
pub(crate) struct FakeHttpClient<Source> {
    marker: PhantomData<Source>,
}

impl<Source> HttpClientHandle for FakeHttpClient<Source>
where
    Source: FakeError,
{
    type Error = Source;

    fn begin_cleanup(&self, _scope: CleanupScope) {}

    fn finish_cleanup(&self) -> Vec<Arc<Source>> {
        Vec::new()
    }
}

pub(crate) struct FakeInit {
    version: String,
}

impl InitializeOutcome for FakeInit {
    fn protocol_version(&self) -> &str {
        &self.version
    }

    fn instructions(&self) -> Option<&str> {
        None
    }
}

pub(crate) struct FakeSession<Source> {
    script: Arc<Mutex<Script>>,
    marker: PhantomData<Source>,
}

pub(crate) fn fake_session<Source>(script: Script) -> FakeSession<Source> {
    FakeSession {
        script: Arc::new(Mutex::new(script)),
        marker: PhantomData,
    }
}

pub(crate) struct FakeSessionContext<Source> {
    script: Arc<Mutex<Script>>,
    marker: PhantomData<Source>,
}

impl<Source> SessionContext for FakeSessionContext<Source>
where
    Source: FakeError,
{
    type Session = FakeSession<Source>;
    type Error = Source;

    fn enter(&mut self) -> impl Future<Output = Result<FakeSession<Source>, Source>> + Send {
        let session = FakeSession {
            script: Arc::clone(&self.script),
            marker: PhantomData,
        };
        async move { Ok(session) }
    }

    fn exit(self) -> impl Future<Output = Result<(), ExitFailure<Source>>> + Send + 'static {
        ready(Ok(()))
    }
}

impl<Source> ClientSession for FakeSession<Source>
where
    Source: FakeError,
{
    type Error = Source;
    type InitializeResult = FakeInit;

    fn initialize(&mut self) -> impl Future<Output = Result<FakeInit, Source>> + Send {
        ready(Ok(FakeInit {
            version: "2025-06-18".to_owned(),
        }))
    }

    fn send_initialize(
        &mut self,
        _request: InitializeRequest,
    ) -> impl Future<Output = Result<FakeInit, Source>> + Send {
        ready(Ok(FakeInit {
            version: "2025-06-18".to_owned(),
        }))
    }

    fn adopt(&mut self, _result: &FakeInit) {}

    fn send_initialized(&mut self) -> impl Future<Output = Result<(), Source>> + Send {
        ready(Ok(()))
    }

    fn protocol_failure(error: &Source) -> Option<McpFailure> {
        error.protocol_failure()
    }
}

pub(crate) async fn resolve_session_step<Source, Value>(
    step: Step<Value>,
) -> Result<Value, SessionError<Source>>
where
    Source: FakeError,
{
    match step {
        Step::Ready(value) => Ok(value),
        Step::Delayed { value, delay } => {
            sleep(delay).await;
            Ok(value)
        }
        Step::Protocol(failure) => Err(SessionError::Protocol(failure)),
        Step::Plain(message) => Err(SessionError::Runtime(Arc::new(Source::plain(message)))),
    }
}

macro_rules! page_method {
    ($method:ident, $source:ident, $queue:ident, $page:ident) => {
        fn $method(
            &mut self,
            cursor: Option<String>,
        ) -> impl Future<Output = Result<$page, $source>> + Send {
            let next_step = {
                let mut script = lock(&self.script);
                script.cursors.push(cursor);
                script.$queue.pop_front()
            };
            async move {
                match next_step {
                    Some(Step::Ready(value)) => Ok(value),
                    Some(Step::Delayed { value, delay }) => {
                        sleep(delay).await;
                        Ok(value)
                    }
                    Some(Step::Protocol(failure)) => Err($source::protocol(failure)),
                    Some(Step::Plain(message)) => Err($source::plain(message)),
                    None => Err($source::plain("no scripted page")),
                }
            }
        }
    };
}

macro_rules! result_method {
    ($method:ident, $source:ident, $queue:ident, $record:ident, $request:ty, $output:ty) => {
        fn $method(
            &mut self,
            request: $request,
        ) -> impl Future<Output = Result<$output, $source>> + Send {
            let next_step = {
                let mut script = lock(&self.script);
                script.$record.push(request);
                script.$queue.take()
            };
            async move {
                match next_step {
                    Some(Step::Ready(value)) => Ok(value),
                    Some(Step::Delayed { value, delay }) => {
                        sleep(delay).await;
                        Ok(value)
                    }
                    Some(Step::Protocol(failure)) => Err($source::protocol(failure)),
                    Some(Step::Plain(message)) => Err($source::plain(message)),
                    None => Err($source::plain("no scripted result")),
                }
            }
        }
    };
}

impl<Source> ResourceSession for FakeSession<Source>
where
    Source: FakeError,
{
    type Error = Source;

    fn server_capabilities(&self) -> Option<ServerCapabilities> {
        lock(&self.script).capabilities.clone()
    }

    page_method!(list_prompts, Source, prompts, PromptPage);
    page_method!(list_resources, Source, resources, ResourcePage);
    page_method!(
        list_resource_templates,
        Source,
        templates,
        ResourceTemplatePage
    );
    result_method!(
        get_prompt,
        Source,
        prompt_result,
        prompt_params,
        GetPromptRequestParams,
        GetPromptResult
    );
    result_method!(
        read_resource,
        Source,
        resource_result,
        resource_uris,
        Url,
        ReadResourceResult
    );

    fn protocol_failure(error: &Source) -> Option<McpFailure> {
        error.protocol_failure()
    }
}

impl<Source> McpRuntime for FakeRuntime<Source>
where
    Source: FakeError,
{
    type Error = Source;
    type Streams = ();
    type Transport = FakeTransport<Source>;
    type HttpClient = FakeHttpClient<Source>;
    type Session = FakeSessionContext<Source>;

    fn stdio_client(&self, _parameters: StdioServerParameters) -> FakeTransport<Source> {
        FakeTransport {
            script: Arc::clone(&self.script),
            marker: PhantomData,
        }
    }

    fn sse_client(&self, _request: SseRequest) -> FakeTransport<Source> {
        FakeTransport {
            script: Arc::clone(&self.script),
            marker: PhantomData,
        }
    }

    fn streamable_http_client(
        &self,
        _url: String,
        _client: Arc<FakeHttpClient<Source>>,
    ) -> FakeTransport<Source> {
        FakeTransport {
            script: Arc::clone(&self.script),
            marker: PhantomData,
        }
    }

    fn http_client(&self, _config: HttpClientConfig) -> Result<FakeHttpClient<Source>, Source> {
        Ok(FakeHttpClient {
            marker: PhantomData,
        })
    }

    fn session_context(
        &self,
        _streams: (),
        _options: SessionOptions<Source>,
    ) -> FakeSessionContext<Source> {
        FakeSessionContext {
            script: Arc::clone(&self.script),
            marker: PhantomData,
        }
    }

    fn close_http_client(
        self: Arc<Self>,
        _client: Arc<FakeHttpClient<Source>>,
    ) -> impl Future<Output = Result<(), ExitFailure<Source>>> + Send + 'static {
        ready(Ok(()))
    }
}

pub(crate) fn session_client<Source>(
    runtime: FakeRuntime<Source>,
    timeout: Duration,
) -> SessionClient<FakeRuntime<Source>>
where
    Source: FakeError,
{
    SessionClient {
        runtime: Arc::new(runtime),
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
            transport_type: TransportType::Sse,
            stdio: None,
            environment: BTreeMap::new(),
            npm_cache_dir: String::new(),
            tls: TlsPolicy {
                verification: Verification::Enabled,
                ca_file: None,
                certificate: None,
                http2: false,
                ecdh_curve: None,
                user_agent: "betterlite".to_owned(),
            },
            protocol_version: ProtocolVersion::Auto,
            client_version: "1.105.1".to_owned(),
            callbacks: Callbacks::default(),
        },
        tracker: TaskTracker::new(),
        caller_cancelled: CancellationToken::new(),
        last_initialize_instructions: None,
    }
}

#[derive(Clone, Default)]
pub(crate) struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    pub(crate) fn text(&self) -> String {
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

pub(crate) fn capture() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .finish();
    (captured, set_default(subscriber))
}

pub(crate) fn protocol(code: i32, message: &str) -> McpFailure {
    McpFailure {
        code,
        message: message.to_owned(),
        context: None,
    }
}
