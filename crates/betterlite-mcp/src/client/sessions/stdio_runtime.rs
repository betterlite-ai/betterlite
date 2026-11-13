use std::future::{Future, ready};
use std::io;
use std::sync::Arc;

use crate::client::cleanup::http_client::CleanupScope;

use super::error::ExitFailure;
use super::factory::{HttpClientConfig, SseRequest};
use super::negotiate::ClientSession;
use super::runtime::{HttpClientHandle, McpRuntime, SessionContext, SessionOptions};
use super::stdio::{StdioServer, StdioStreams, StdioTransport};
use super::transport::StdioServerParameters;

pub trait StdioSessionFactory: Send + Sync + 'static {
    type Session: ClientSession<Error = io::Error> + Send + 'static;

    fn build(&self, streams: StdioStreams, options: SessionOptions<io::Error>) -> Self::Session;
}

pub struct StdioRuntime<Factory> {
    pub factory: Arc<Factory>,
}

pub struct UnsupportedHttpClient;

impl HttpClientHandle for UnsupportedHttpClient {
    type Error = io::Error;

    fn begin_cleanup(&self, _scope: CleanupScope) {}

    fn finish_cleanup(&self) -> Vec<Arc<io::Error>> {
        Vec::new()
    }
}

pub struct StdioSessionContext<Factory> {
    streams: Option<StdioStreams>,
    options: Option<SessionOptions<io::Error>>,
    factory: Arc<Factory>,
}

impl<Factory: StdioSessionFactory> SessionContext for StdioSessionContext<Factory> {
    type Session = Factory::Session;
    type Error = io::Error;

    fn enter(&mut self) -> impl Future<Output = Result<Factory::Session, io::Error>> + Send {
        let built = self
            .streams
            .take()
            .zip(self.options.take())
            .map(|(streams, options)| self.factory.build(streams, options));
        ready(built.ok_or_else(|| io::Error::other("the stdio session is already open")))
    }

    fn exit(self) -> impl Future<Output = Result<(), ExitFailure<io::Error>>> + Send + 'static {
        ready(Ok(()))
    }
}

impl<Factory> McpRuntime for StdioRuntime<Factory>
where
    Factory: StdioSessionFactory,
{
    type Error = io::Error;
    type Streams = StdioStreams;
    type Transport = StdioTransport;
    type HttpClient = UnsupportedHttpClient;
    type Session = StdioSessionContext<Factory>;

    fn stdio_client(&self, parameters: StdioServerParameters) -> StdioTransport {
        StdioTransport::Server(Box::new(StdioServer {
            parameters,
            child: None,
            stdin: None,
            stdout: None,
        }))
    }

    fn sse_client(&self, _request: SseRequest) -> StdioTransport {
        StdioTransport::Unsupported("SSE")
    }

    fn streamable_http_client(
        &self,
        _url: String,
        _client: Arc<UnsupportedHttpClient>,
    ) -> StdioTransport {
        StdioTransport::Unsupported("streamable HTTP")
    }

    fn http_client(&self, _config: HttpClientConfig) -> Result<UnsupportedHttpClient, io::Error> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "the stdio runtime has no HTTP client",
        ))
    }

    fn session_context(
        &self,
        streams: StdioStreams,
        options: SessionOptions<io::Error>,
    ) -> StdioSessionContext<Factory> {
        StdioSessionContext {
            streams: Some(streams),
            options: Some(options),
            factory: Arc::clone(&self.factory),
        }
    }

    fn close_http_client(
        self: Arc<Self>,
        _client: Arc<UnsupportedHttpClient>,
    ) -> impl Future<Output = Result<(), ExitFailure<io::Error>>> + Send + 'static {
        ready(Ok(()))
    }
}
