use super::error::WebSocketError;
use async_trait::async_trait;
use std::future::Future;
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RealtimeMessage {
    Text(String),
    Binary(Vec<u8>),
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WebSocketScope {
    pub headers: Vec<(String, String)>,
}
pub trait ScopedWebSocket {
    fn scope(&self) -> &WebSocketScope;
}
#[async_trait]
pub trait ClientWebSocket: ScopedWebSocket + Send {
    async fn send_text(&mut self, data: &str) -> Result<(), WebSocketError>;

    async fn receive_text(&mut self) -> Result<String, WebSocketError>;

    async fn close(&mut self, code: u16, reason: Option<&str>) -> Result<(), WebSocketError>;
}

// LiteLLM's async context manager entry is the constructor and its exit is close
#[async_trait]
pub trait RealtimeBackend: Send {
    async fn send(&mut self, message: RealtimeMessage) -> Result<(), WebSocketError>;

    async fn recv(&mut self, decode: Option<bool>) -> Result<RealtimeMessage, WebSocketError>;

    async fn close(&mut self) -> Result<(), WebSocketError>;
}
pub trait LoggingWorker {
    fn enqueue<Work>(&self, work: Work)
    where
        Work: Future<Output = ()> + Send + 'static;
}
