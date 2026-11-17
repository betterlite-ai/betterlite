use std::fmt::Display;
use std::future::ready;
use std::sync::Arc;

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
use crate::client::sessions::stdio::StdioTransport;
use crate::client::sessions::stream::MessageHandler;
use crate::client::sessions::transport::StdioServerParameters;

use super::session::SdkSessionContext;

pub struct SdkHttpClient {
    transport: Arc<McpHttpTransport>,
    client: Arc<McpHttpClient<McpHttpTransport>>,
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

pub struct SdkRuntime {
    base: HttpBase,
    capture: Arc<dyn ErrorCapture>,
    tracker: TaskTracker,
    // One handler serves every transport this runtime builds, and a session mirrors its failure into B4744's handler
    handler: MessageHandler<SdkError>,
}

impl SdkRuntime {
    // The runtime reads no environment and holds no global; every dependency arrives through this constructor
    #[must_use]
    pub fn new(base: HttpBase, capture: Arc<dyn ErrorCapture>, tracker: TaskTracker) -> Self {
        Self {
            base,
            capture,
            tracker,
            handler: MessageHandler::default(),
        }
    }

    fn legacy_sse(&self, request: SseRequest) -> Result<LegacySse, SdkError> {
        let url = Url::parse(&request.url).map_err(SdkError::EndpointUrl)?;
        let http = McpHttpTransport::new(&request.client, &self.base, Arc::clone(&self.capture))?;
        Ok(LegacySse {
            url,
            http: Arc::new(http),
            handler: self.handler.clone(),
        })
    }

    fn streamable(&self, url: &str, client: &SdkHttpClient) -> Result<StreamableHttp, SdkError> {
        let url = Url::parse(url).map_err(SdkError::EndpointUrl)?;
        Ok(StreamableHttp::new(
            url,
            Arc::clone(&client.transport),
            Arc::clone(&client.client),
            self.handler.clone(),
        ))
    }
}

// McpRuntime::sse_client has no Result, so a transport that can't be built stands in as an unavailable one
fn unsupported(name: &'static str, error: &dyn Display) -> SdkTransport {
    warn!("the MCP {name} client is unavailable: {error}");
    SdkTransport::Stdio(StdioContext(StdioTransport::Unsupported(name)))
}

impl McpRuntime for SdkRuntime {
    type Error = SdkError;
    type Streams = SdkStreams;
    type Transport = SdkTransport;
    type HttpClient = SdkHttpClient;
    type Session = SdkSessionContext<SdkStreams>;

    fn stdio_client(&self, parameters: StdioServerParameters) -> SdkTransport {
        SdkTransport::Stdio(StdioContext(stdio_transport(parameters)))
    }

    fn sse_client(&self, request: SseRequest) -> SdkTransport {
        match self.legacy_sse(request) {
            Ok(sse) => SdkTransport::LegacySse(sse),
            Err(error) => unsupported("legacy SSE", &error),
        }
    }

    fn streamable_http_client(&self, url: String, client: Arc<SdkHttpClient>) -> SdkTransport {
        match self.streamable(&url, &client) {
            Ok(transport) => SdkTransport::Streamable(transport),
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
        streams: SdkStreams,
        options: SessionOptions<SdkError>,
    ) -> SdkSessionContext<SdkStreams> {
        SdkSessionContext::new(streams, options, self.handler.clone())
    }

    fn close_http_client(
        self: Arc<Self>,
        _client: Arc<SdkHttpClient>,
    ) -> impl std::future::Future<Output = Result<(), ExitFailure<SdkError>>> + Send + 'static {
        ready(Ok(()))
    }
}
