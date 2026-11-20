use std::error::Error as StdError;
use std::fmt::{Debug, Formatter, Result as FmtResult};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use http::HeaderMap;
use thiserror::Error;

#[derive(Debug, Error)]
#[error("the Gemini transport failed: {source}")]
pub struct TransportError {
    #[source]
    pub source: Box<dyn StdError + Send + Sync + 'static>,
}

// The reply a transport hands back: any status is a reply, only failing to get one is an error
#[derive(Debug, Clone)]
pub struct GeminiReply {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, Copy)]
pub enum TransportMethod {
    Post,
    Put,
}

impl TransportMethod {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Post => "POST",
            Self::Put => "PUT",
        }
    }
}

// A JSON body is serialized by the transport that sends it, so a body it cannot write is its error
#[derive(Debug)]
pub enum TransportBody {
    Json(Json),
    Bytes(Vec<u8>),
}

impl TransportBody {
    pub fn into_bytes(self) -> Result<Vec<u8>, serde_json::Error> {
        match self {
            Self::Json(value) => serde_json::to_vec(&value),
            Self::Bytes(bytes) => Ok(bytes),
        }
    }
}

pub struct TransportRequest {
    pub method: TransportMethod,
    pub url: String,
    pub headers: HeaderMap,
    pub body: TransportBody,
    pub timeout: Duration,
}

// The headers carry the API key, so a request prints their names and a body size, never a value
impl Debug for TransportRequest {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> FmtResult {
        let body_bytes = match &self.body {
            TransportBody::Json(value) => serde_json::to_vec(value).ok().map(|bytes| bytes.len()),
            TransportBody::Bytes(bytes) => Some(bytes.len()),
        };
        formatter
            .debug_struct("TransportRequest")
            .field("method", &self.method)
            .field("url", &self.url)
            .field("header_names", &self.headers.keys().collect::<Vec<_>>())
            .field("body_bytes", &body_bytes)
            .field("timeout", &self.timeout)
            .finish()
    }
}

// The one call a Gemini store or upload makes; B3460 builds this on betterlite-http
#[async_trait]
pub trait GeminiTransport: Send + Sync {
    async fn send(&self, request: TransportRequest) -> Result<GeminiReply, TransportError>;
}
