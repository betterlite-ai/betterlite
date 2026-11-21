use std::error::Error as StdError;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::api::realtime_backend_relay::source::RealtimeBackendSource;
use crate::api::realtime_beta_protocol::headers::{
    HeaderField, HeaderPair, HeaderScope, openai_beta_realtime_header,
};
use crate::api::realtime_contracts::error::WebSocketError;
use crate::api::realtime_contracts::transport::{ClientWebSocket, ScopedWebSocket, WebSocketScope};
use crate::api::realtime_session_state::RealtimeStreaming;

use super::error::ClientCloseError;

// The websockets client behind async_realtime; unlike B1004's client socket its close keeps the RuntimeError class that decides whether a failed close is ignorable
#[async_trait]
pub trait RealtimeClientSocket: Send {
    async fn send_text(&mut self, data: &str) -> Result<(), WebSocketError>;

    async fn receive_text(&mut self) -> Result<String, WebSocketError>;

    async fn close(&mut self, code: u16, reason: Option<&str>) -> Result<(), ClientCloseError>;
}

// The connection's client socket and ASGI scope, shared so the session can write to it while the relay holds the same socket in the state
#[derive(Clone)]
pub struct ClientHandle {
    pub socket: Arc<Mutex<Box<dyn RealtimeClientSocket>>>,
    pub scope: WebSocketScope,
}

// B1004's client socket over the shared handle, so the relay writes to the same connection the session opened
pub struct ClientSlot {
    pub handle: ClientHandle,
}

impl ScopedWebSocket for ClientSlot {
    fn scope(&self) -> &WebSocketScope {
        &self.handle.scope
    }
}

#[async_trait]
impl ClientWebSocket for ClientSlot {
    async fn send_text(&mut self, data: &str) -> Result<(), WebSocketError> {
        self.handle.socket.lock().await.send_text(data).await
    }

    async fn receive_text(&mut self) -> Result<String, WebSocketError> {
        self.handle.socket.lock().await.receive_text().await
    }

    async fn close(&mut self, code: u16, reason: Option<&str>) -> Result<(), WebSocketError> {
        self.handle
            .socket
            .lock()
            .await
            .close(code, reason)
            .await
            .map_err(|error| WebSocketError {
                message: error.to_string(),
            })
    }
}

// What the injected relay gets for one session: B1006's connection state, the receiving half of a dialed backend, and the caller's cancellation
pub struct SessionRelay<'session> {
    pub state: &'session mut RealtimeStreaming,
    pub source: Option<&'session mut (dyn RealtimeBackendSource + 'session)>,
    pub cancellation: &'session CancellationToken,
}

// B1009's bidirectional relay behind async_realtime; it owns the composition the generic connection cannot express with one mutable state
#[async_trait]
pub trait RealtimeSessionRelay: Send {
    type Error: StdError + Send + 'static;

    async fn relay(&mut self, session: SessionRelay<'_>) -> Result<(), Self::Error>;
}

// LiteLLM reads the client's raw scope headers and treats a scope it cannot read as no beta opt-in
#[must_use]
pub fn client_wants_beta(scope: &WebSocketScope) -> bool {
    let pairs: Vec<HeaderPair> = scope
        .headers
        .iter()
        .map(|(name, value)| {
            (
                HeaderField::Text(name.clone()),
                HeaderField::Text(value.clone()),
            )
        })
        .collect();
    let header_scope = HeaderScope { headers: &pairs };
    openai_beta_realtime_header(Some(&header_scope))
}
