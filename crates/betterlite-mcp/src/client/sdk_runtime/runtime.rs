use std::fmt::Display;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use rmcp::RoleClient;
use rmcp::service::{RxJsonRpcMessage, TxJsonRpcMessage};
use rmcp::transport::Transport;
use tokio_util::task::TaskTracker;
use tracing::warn;
use url::Url;

use crate::client::cleanup::http_client::{CleanupScope, McpHttpClient};
use crate::client::http_transport::capture::ErrorCapture;
use crate::client::http_transport::send::{HttpBase, McpHttpTransport};
use crate::client::sdk_transport::context::{SdkStreams, SdkTransport, StdioContext};
use crate::client::sdk_transport::error::SdkError;
use crate::client::sdk_transport::legacy_sse::LegacySse;
use crate::client::sdk_transport::streamable::StreamableHttp;
use crate::client::sessions::error::ExitFailure;
use crate::client::sessions::factory::{HttpClientConfig, SseRequest};
use crate::client::sessions::runtime::{
    HttpClientHandle, McpRuntime, SessionOptions, stdio_transport,
};
use crate::client::sessions::stdio::{StdioShutdown, StdioTransport};
use crate::client::sessions::stream::MessageHandler;
use crate::client::sessions::transport::{StdioServerParameters, TransportContext};

use super::session::SdkSessionContext;

pub struct SdkHttpClient {
    pub(super) transport: Arc<McpHttpTransport>,
    pub(super) client: Arc<McpHttpClient<McpHttpTransport>>,
}

impl HttpClientHandle for SdkHttpClient {
    type Error = SdkError;

    fn begin_cleanup(&self, scope: CleanupScope) {
        let _replaced = self.client.enter_cleanup(scope);
    }

    fn finish_cleanup(&self) -> Vec<Arc<SdkError>> {
        self.client
            .leave_cleanup()
            .into_iter()
            .map(|error| Arc::new(SdkError::Http(error)))
            .collect()
    }
}

// The transport a session runs on stays with the context for its exit, so the session gets this handle to stamp the negotiated version on every later streamable request
#[derive(Clone)]
pub struct ProtocolVersionHandle {
    transport: Arc<Mutex<Option<SdkTransport>>>,
}

impl ProtocolVersionHandle {
    pub fn set(&self, version: &str) {
        let transport = self
            .transport
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(SdkTransport::Streamable(http)) = transport.as_ref() {
            http.set_protocol_version(version);
        }
    }
}

// The streams carry this session's own stream handler, so enter and a replayed failure never read another session's slot
pub struct SdkSessionStreams {
    pub(super) streams: SdkStreams,
    pub(super) handler: MessageHandler<SdkError>,
    pub(super) protocol_version: ProtocolVersionHandle,
}

impl Transport<RoleClient> for SdkSessionStreams {
    type Error = SdkError;

    fn send(
        &mut self,
        item: TxJsonRpcMessage<RoleClient>,
    ) -> impl Future<Output = Result<(), SdkError>> + Send + 'static {
        self.streams.send(item)
    }

    async fn receive(&mut self) -> Option<RxJsonRpcMessage<RoleClient>> {
        self.streams.receive().await
    }

    async fn close(&mut self) -> Result<(), SdkError> {
        self.streams.close().await
    }
}

pub struct SdkTransportContext {
    transport: Arc<Mutex<Option<SdkTransport>>>,
    handler: MessageHandler<SdkError>,
}

impl TransportContext for SdkTransportContext {
    type Streams = SdkSessionStreams;
    type Error = SdkError;

    async fn enter(&mut self) -> Result<SdkSessionStreams, SdkError> {
        let Some(mut transport) = self
            .transport
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        else {
            return Err(SdkError::AlreadyOpen);
        };
        let opened = transport.enter().await;
        *self
            .transport
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(transport);
        Ok(SdkSessionStreams {
            streams: opened?,
            handler: self.handler.clone(),
            protocol_version: ProtocolVersionHandle {
                transport: Arc::clone(&self.transport),
            },
        })
    }

    async fn exit(self) -> Result<(), ExitFailure<SdkError>> {
        let transport = self
            .transport
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        match transport {
            Some(opened) => opened.exit().await,
            None => Ok(()),
        }
    }
}

pub struct SdkRuntime {
    base: HttpBase,
    capture: Arc<dyn ErrorCapture>,
    tracker: TaskTracker,
    shutdown: StdioShutdown,
}

impl SdkRuntime {
    // The runtime reads no environment and holds no global; every dependency arrives through this constructor
    #[must_use]
    pub fn new(
        base: HttpBase,
        capture: Arc<dyn ErrorCapture>,
        tracker: TaskTracker,
        shutdown: StdioShutdown,
    ) -> Self {
        Self {
            base,
            capture,
            tracker,
            shutdown,
        }
    }

    fn legacy_sse(&self, request: &SseRequest) -> Result<SdkTransportContext, SdkError> {
        let url = Url::parse(&request.url).map_err(SdkError::EndpointUrl)?;
        let http = McpHttpTransport::new(&request.client, &self.base, Arc::clone(&self.capture))?;
        let handler = MessageHandler::default();
        let sdk_transport = SdkTransport::LegacySse(LegacySse {
            url,
            http: Arc::new(http),
            handler: handler.clone(),
        });
        let transport = Arc::new(Mutex::new(Some(sdk_transport)));
        Ok(SdkTransportContext { transport, handler })
    }
}

fn streamable(url: &str, client: &SdkHttpClient) -> Result<SdkTransportContext, SdkError> {
    let endpoint = Url::parse(url).map_err(SdkError::EndpointUrl)?;
    let handler = MessageHandler::default();
    let sdk_transport = SdkTransport::Streamable(StreamableHttp::new(
        endpoint,
        Arc::clone(&client.transport),
        Arc::clone(&client.client),
        handler.clone(),
    ));
    let transport = Arc::new(Mutex::new(Some(sdk_transport)));
    Ok(SdkTransportContext { transport, handler })
}

// McpRuntime::sse_client has no Result, so a transport that can't be built stands in as an unavailable one
fn unsupported(name: &'static str, error: &dyn Display) -> SdkTransportContext {
    warn!("the MCP {name} client is unavailable: {error}");
    SdkTransportContext {
        transport: Arc::new(Mutex::new(Some(SdkTransport::Stdio(StdioContext(
            StdioTransport::Unsupported(name),
        ))))),
        handler: MessageHandler::default(),
    }
}

impl McpRuntime for SdkRuntime {
    type Error = SdkError;
    type Streams = SdkSessionStreams;
    type Transport = SdkTransportContext;
    type HttpClient = SdkHttpClient;
    type Session = SdkSessionContext<SdkSessionStreams>;

    fn stdio_client(&self, parameters: StdioServerParameters) -> SdkTransportContext {
        SdkTransportContext {
            transport: Arc::new(Mutex::new(Some(SdkTransport::Stdio(StdioContext(
                stdio_transport(parameters, self.shutdown),
            ))))),
            handler: MessageHandler::default(),
        }
    }

    fn sse_client(&self, request: SseRequest) -> SdkTransportContext {
        match self.legacy_sse(&request) {
            Ok(context) => context,
            Err(error) => unsupported("legacy SSE", &error),
        }
    }

    fn streamable_http_client(
        &self,
        url: String,
        client: Arc<SdkHttpClient>,
    ) -> SdkTransportContext {
        match streamable(&url, &client) {
            Ok(context) => context,
            Err(error) => unsupported("streamable HTTP", &error),
        }
    }

    fn http_client(&self, config: HttpClientConfig) -> Result<SdkHttpClient, SdkError> {
        let transport = Arc::new(McpHttpTransport::new(
            &config,
            &self.base,
            Arc::clone(&self.capture),
        )?);
        let client = Arc::new(McpHttpClient::new(
            Arc::clone(&transport),
            self.tracker.clone(),
        ));
        Ok(SdkHttpClient { transport, client })
    }

    fn session_context(
        &self,
        streams: SdkSessionStreams,
        options: SessionOptions<SdkError>,
    ) -> SdkSessionContext<SdkSessionStreams> {
        let handler = streams.handler.clone();
        let protocol_version = streams.protocol_version.clone();
        SdkSessionContext::new(streams, options, handler, Some(protocol_version))
    }

    fn close_http_client(
        self: Arc<Self>,
        _client: Arc<SdkHttpClient>,
    ) -> impl Future<Output = Result<(), ExitFailure<SdkError>>> + Send + 'static {
        ready(Ok(()))
    }
}
