use std::collections::{BTreeMap, VecDeque};
use std::error::Error;
use std::future::{Future, ready};
use std::marker::PhantomData;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::tls::{TlsPolicy, Verification};
use betterlite_mcp::client::cleanup::http_client::CleanupScope;
use betterlite_mcp::client::sessions::client::{SessionClient, SessionSettings};
use betterlite_mcp::client::sessions::error::ExitFailure;
use betterlite_mcp::client::sessions::factory::{HttpClientConfig, SseRequest};
use betterlite_mcp::client::sessions::negotiate::{Callbacks, ProtocolVersion};
use betterlite_mcp::client::sessions::runtime::{
    HttpClientHandle, McpRuntime, SessionContext, SessionOptions,
};
use betterlite_mcp::client::sessions::transport::{
    StdioConfig, StdioServerParameters, TransportContext, TransportType,
};
use betterlite_mcp::credentials::{McpAuthType, McpClient as CredentialClient};
use tokio_util::task::TaskTracker;

use super::super::input::Settings;
use super::session::Session;
use super::types::{Script, Simulated};

pub(crate) struct Runtime<Source> {
    pub(crate) script: Script<Source>,
}

pub(crate) struct Transport<Source>(PhantomData<Source>);

impl<Source> TransportContext for Transport<Source>
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

pub(crate) struct HttpClient<Source>(PhantomData<Source>);

impl<Source> HttpClientHandle for HttpClient<Source>
where
    Source: Error + Send + Sync + 'static,
{
    type Error = Source;

    fn begin_cleanup(&self, _scope: CleanupScope) {}

    fn finish_cleanup(&self) -> Vec<Arc<Source>> {
        Vec::new()
    }
}

pub(crate) struct Context<Source> {
    pub(crate) script: Arc<Script<Source>>,
}

impl<Source> SessionContext for Context<Source>
where
    Source: Simulated,
{
    type Session = Session<Source>;
    type Error = Source;

    fn enter(&mut self) -> impl Future<Output = Result<Session<Source>, Source>> + Send {
        let script = Arc::clone(&self.script);
        let frames = Mutex::new(VecDeque::from(self.script.frames.clone()));
        async move { Ok(Session { script, frames }) }
    }

    fn exit(self) -> impl Future<Output = Result<(), ExitFailure<Source>>> + Send + 'static {
        ready(Ok(()))
    }
}

impl<Source> McpRuntime for Runtime<Source>
where
    Source: Simulated,
{
    type Error = Source;
    type Streams = ();
    type Transport = Transport<Source>;
    type HttpClient = HttpClient<Source>;
    type Session = Context<Source>;

    fn stdio_client(&self, _parameters: StdioServerParameters) -> Transport<Source> {
        Transport(PhantomData)
    }

    fn sse_client(&self, _request: SseRequest) -> Transport<Source> {
        Transport(PhantomData)
    }

    fn streamable_http_client(
        &self,
        _url: String,
        _client: Arc<HttpClient<Source>>,
    ) -> Transport<Source> {
        Transport(PhantomData)
    }

    fn http_client(&self, _config: HttpClientConfig) -> Result<HttpClient<Source>, Source> {
        Ok(HttpClient(PhantomData))
    }

    fn session_context(&self, _streams: (), _options: SessionOptions<Source>) -> Context<Source> {
        Context {
            script: Arc::new(self.script.clone()),
        }
    }

    fn close_http_client(
        self: Arc<Self>,
        _client: Arc<HttpClient<Source>>,
    ) -> impl Future<Output = Result<(), ExitFailure<Source>>> + Send + 'static {
        ready(Ok(()))
    }
}

fn tls_policy() -> TlsPolicy {
    TlsPolicy {
        verification: Verification::Enabled,
        ca_file: None,
        certificate: None,
        http2: false,
        ecdh_curve: None,
        user_agent: "betterlite-oracle".to_owned(),
    }
}

pub(crate) fn build_client<Source>(
    settings: &Settings,
    script: Script<Source>,
) -> SessionClient<Runtime<Source>>
where
    Source: Simulated,
{
    let timeout = Duration::from_millis(settings.timeout_ms);
    let cancelled = script.cancelled.clone();
    SessionClient {
        runtime: Arc::new(Runtime { script }),
        auth: CredentialClient::new(
            settings.server_url.clone(),
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
