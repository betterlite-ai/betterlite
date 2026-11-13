use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::proxy::transport_error::TransportError;
use http::{HeaderMap, Method};

// LiteLLM builds the Replicate client with params={"timeout": 600.0} for the create POST and every poll
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicateRequest {
    pub method: Method,
    pub url: String,
    pub headers: HeaderMap,
    pub body: Option<Vec<u8>>,
    pub timeout: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicateResponse {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

#[async_trait]
pub trait ReplicateTransport: Send + Sync {
    async fn send(&self, request: ReplicateRequest) -> Result<ReplicateResponse, TransportError>;
}
