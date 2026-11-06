use std::error::Error;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use crate::client::cleanup::http_client::{
    CleanupScope, McpHttpClient, Transport as HttpTransport,
};

use super::error::ExitFailure;
use super::factory::{HttpClientConfig, SseRequest};
use super::negotiate::{Callbacks, ClientSession};
use super::stream::MessageHandler;
use super::transport::{StdioServerParameters, TransportContext};

pub struct SessionOptions {
    pub read_timeout: Duration,
    pub callbacks: Callbacks,
    pub message_handler: MessageHandler,
}

pub trait SessionContext: Sized + Send + 'static {
    type Session: ClientSession<Error = Self::Error> + Send + 'static;
    type Error: Error + Send + Sync + 'static;

    fn enter(&mut self) -> impl Future<Output = Result<Self::Session, Self::Error>> + Send;

    fn exit(self) -> impl Future<Output = Result<(), ExitFailure<Self::Error>>> + Send + 'static;
}

pub trait HttpClientHandle {
    type Error: Error + Send + Sync + 'static;

    fn begin_cleanup(&self, scope: CleanupScope);

    fn finish_cleanup(&self) -> Vec<Arc<Self::Error>>;
}

impl<Client> HttpClientHandle for McpHttpClient<Client>
where
    Client: HttpTransport,
{
    type Error = Client::Error;

    fn begin_cleanup(&self, scope: CleanupScope) {
        let _replaced = Self::enter_cleanup(self, scope);
    }

    fn finish_cleanup(&self) -> Vec<Arc<Client::Error>> {
        Self::leave_cleanup(self)
    }
}

pub trait McpRuntime: Send + Sync + 'static {
    type Error: Error + Send + Sync + 'static;
    type Streams: Send + 'static;
    type Transport: TransportContext<Streams = Self::Streams, Error = Self::Error>;
    type HttpClient: HttpClientHandle<Error = Self::Error> + Send + Sync + 'static;
    type Session: SessionContext<Error = Self::Error>;

    fn stdio_client(&self, parameters: StdioServerParameters) -> Self::Transport;

    fn sse_client(&self, request: SseRequest) -> Self::Transport;

    fn streamable_http_client(&self, url: String, client: Arc<Self::HttpClient>)
    -> Self::Transport;

    fn http_client(&self, config: HttpClientConfig) -> Result<Self::HttpClient, Self::Error>;

    fn session_context(&self, streams: Self::Streams, options: SessionOptions) -> Self::Session;

    fn close_http_client(
        self: Arc<Self>,
        client: Arc<Self::HttpClient>,
    ) -> impl Future<Output = Result<(), ExitFailure<Self::Error>>> + Send + 'static;
}
