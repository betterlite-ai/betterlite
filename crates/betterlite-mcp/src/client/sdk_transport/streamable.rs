pub mod get_stream;
pub mod post;
pub mod sse_response;

use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use bytes::Bytes;
use http::header::{ACCEPT, CONTENT_TYPE, HeaderName, HeaderValue};
use http::{HeaderMap, Method};
use rmcp::RoleClient;
use rmcp::model::ErrorCode;
use rmcp::service::{RxJsonRpcMessage, TxJsonRpcMessage};
use rmcp::transport::Transport;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::{debug, warn};
use url::Url;

use crate::client::cleanup::http_client::McpHttpClient;
use crate::client::http_transport::request::McpHttpRequest;
use crate::client::http_transport::send::McpHttpTransport;
use crate::client::sessions::error::ExitFailure;
use crate::client::sessions::stream::{MessageHandler, StreamFailureKind};
use crate::client::sessions::transport::TransportContext;

use super::error::{SdkError, report_failure};
use super::event_stream::MAX_EVENT_SIZE;
use super::origin::delete_within_origin;
pub(super) const CONNECTION_CLOSED: ErrorCode = ErrorCode(-32000);
pub(super) const MAX_RECONNECTION_ATTEMPTS: usize = 2;
pub(super) const RECONNECTION_DELAY: Duration = Duration::from_millis(1000);
pub(super) const LAST_EVENT_ID: HeaderName = HeaderName::from_static("last-event-id");
const MCP_SESSION_ID: HeaderName = HeaderName::from_static("mcp-session-id");
const MCP_PROTOCOL_VERSION: HeaderName = HeaderName::from_static("mcp-protocol-version");
const INCOMING_CAPACITY: usize = 32;
pub(super) enum Incoming {
    Message(RxJsonRpcMessage<RoleClient>),
    Closed,
}
pub(super) struct SessionState {
    pub(super) session_id: Option<String>,
    pub(super) protocol_version: Option<String>,
}
pub(super) struct StreamableCore {
    pub(super) url: Url,
    pub(super) http: Arc<McpHttpTransport>,
    pub(super) cleanup: Arc<McpHttpClient<McpHttpTransport>>,
    pub(super) handler: MessageHandler<SdkError>,
    pub(super) incoming: mpsc::Sender<Incoming>,
    pub(super) state: Mutex<SessionState>,
    pub(super) cancel: CancellationToken,
    pub(super) tasks: TaskTracker,
    pub(super) max_event_size: usize,
}
pub struct StreamableHttp {
    core: Arc<StreamableCore>,
    incoming: Option<mpsc::Receiver<Incoming>>,
}
pub struct StreamableStreams {
    core: Arc<StreamableCore>,
    incoming: mpsc::Receiver<Incoming>,
}
impl StreamableHttp {
    pub fn new(
        url: Url,
        http: Arc<McpHttpTransport>,
        cleanup: Arc<McpHttpClient<McpHttpTransport>>,
        handler: MessageHandler<SdkError>,
    ) -> Self {
        let (incoming, receiver) = mpsc::channel(INCOMING_CAPACITY);
        let state = SessionState {
            session_id: None,
            protocol_version: None,
        };
        let core = StreamableCore {
            url,
            http,
            cleanup,
            handler,
            incoming,
            state: Mutex::new(state),
            cancel: CancellationToken::new(),
            tasks: TaskTracker::new(),
            max_event_size: MAX_EVENT_SIZE,
        };
        Self {
            core: Arc::new(core),
            incoming: Some(receiver),
        }
    }
    pub fn set_protocol_version(&self, version: &str) {
        let mut state = self
            .core
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        state.protocol_version = Some(version.to_owned());
    }
}
impl StreamableCore {
    pub(super) fn headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/json, text/event-stream"),
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(session) = state.session_id.as_deref()
            && let Ok(value) = HeaderValue::from_str(session)
        {
            headers.insert(MCP_SESSION_ID, value);
        }
        if let Some(version) = state.protocol_version.as_deref()
            && let Ok(value) = HeaderValue::from_str(version)
        {
            headers.insert(MCP_PROTOCOL_VERSION, value);
        }
        headers
    }
    pub(super) fn session_id(&self) -> Option<String> {
        match self
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .session_id
            .as_deref()
        {
            Some(session) => Some(session.to_owned()),
            None => None,
        }
    }
    pub(super) fn note_session(&self, headers: &HeaderMap) {
        let Some(session) = headers
            .get(MCP_SESSION_ID)
            .and_then(|value| value.to_str().ok())
        else {
            return;
        };
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.session_id = Some(session.to_owned());
        debug!("Received session ID: {session}");
    }
    pub(super) fn clear_protocol_version(&self) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .protocol_version = None;
    }
    pub(super) const fn retry_delay(retry_ms: Option<u64>) -> Duration {
        match retry_ms {
            Some(milliseconds) => Duration::from_millis(milliseconds),
            None => RECONNECTION_DELAY,
        }
    }
    pub(super) fn failure(&self, kind: StreamFailureKind, error: SdkError) {
        report_failure(&self.handler, kind, error);
        drop(self.incoming.try_send(Incoming::Closed));
    }
    pub(super) async fn close(&self) -> Result<(), SdkError> {
        let session = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.session_id.take()
        };
        if let Some(session) = session {
            let mut headers = self.headers();
            if let Ok(value) = HeaderValue::from_str(&session) {
                headers.insert(MCP_SESSION_ID, value);
            }
            let request = McpHttpRequest {
                method: Method::DELETE,
                url: self.url.clone(),
                headers,
                body: Bytes::new(),
                read_timeout: None,
            };
            match delete_within_origin(&self.cleanup, request).await {
                Ok(405) => debug!("Server does not allow session termination"),
                Ok(200 | 204) => {}
                Ok(status) => warn!("Session termination failed: {status}"),
                Err(error) => warn!("Session termination failed: {error}"),
            }
        }
        self.cancel.cancel();
        self.tasks.close();
        self.tasks.wait().await;
        Ok(())
    }
}
impl TransportContext for StreamableHttp {
    type Streams = StreamableStreams;
    type Error = SdkError;
    fn enter(&mut self) -> impl Future<Output = Result<StreamableStreams, SdkError>> + Send {
        let incoming = self.incoming.take();
        let core = Arc::clone(&self.core);
        async move {
            let Some(incoming) = incoming else {
                return Err(SdkError::AlreadyOpen);
            };
            Ok(StreamableStreams { core, incoming })
        }
    }
    fn exit(self) -> impl Future<Output = Result<(), ExitFailure<SdkError>>> + Send + 'static {
        async move {
            self.core.close().await?;
            Ok(())
        }
    }
}
impl Transport<RoleClient> for StreamableStreams {
    type Error = SdkError;
    fn send(
        &mut self,
        item: TxJsonRpcMessage<RoleClient>,
    ) -> impl Future<Output = Result<(), SdkError>> + Send + 'static {
        let core = Arc::clone(&self.core);
        async move { core.send(item).await }
    }
    fn receive(&mut self) -> impl Future<Output = Option<RxJsonRpcMessage<RoleClient>>> + Send {
        async move {
            match self.incoming.recv().await {
                Some(Incoming::Message(message)) => Some(message),
                Some(Incoming::Closed) | None => None,
            }
        }
    }
    fn close(&mut self) -> impl Future<Output = Result<(), SdkError>> + Send {
        async move { self.core.close().await }
    }
}
