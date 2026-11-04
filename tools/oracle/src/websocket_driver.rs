use serde::{Deserialize, Serialize};
use service::ServiceTag;
use std::future::Future;
use std::io;
use std::time::Duration;
use tokio::{task::JoinError, time::error::Elapsed, time::timeout};
use tokio_tungstenite::tungstenite::{
    Error,
    http::header::{InvalidHeaderName, InvalidHeaderValue, ToStrError},
};

use crate::websocket_oracle::decode::DecodeError;
use crate::websocket_oracle::{Close, Frame, WebSocketCase, close_matches, direction_matches};

mod close;
mod encode;
pub mod gateway;
mod payload;
pub(crate) mod provider;
pub mod service;
mod socket;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct HandshakeObservation {
    pub status: u16,
    pub negotiated_subprotocol: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct TranscriptObservation {
    pub handshake: HandshakeObservation,
    pub frames: Vec<Frame>,
    pub close: Option<Close>,
}

pub trait Clock: Send + Sync {
    fn timeout<F>(
        &self,
        duration: Duration,
        future: F,
    ) -> impl Future<Output = Result<F::Output, Elapsed>> + Send
    where
        F: Future + Send;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct TokioClock;

impl Clock for TokioClock {
    fn timeout<F>(
        &self,
        duration: Duration,
        future: F,
    ) -> impl Future<Output = Result<F::Output, Elapsed>> + Send
    where
        F: Future + Send,
    {
        timeout(duration, future)
    }
}

pub fn compare_transcript(
    expected: &WebSocketCase,
    observed: &TranscriptObservation,
) -> Result<(), String> {
    if expected.expected_http_status != observed.handshake.status {
        return Err("/handshake/status".to_owned());
    }
    if expected.negotiated_subprotocol != observed.handshake.negotiated_subprotocol {
        return Err("/handshake/negotiated_subprotocol".to_owned());
    }
    direction_matches(&expected.frames, &observed.frames)?;
    close_matches(expected.close.as_ref(), observed.close.as_ref())
}

#[derive(Debug, thiserror::Error)]
pub enum DriverError {
    #[error("WebSocket fixture: {0}")]
    Decode(#[from] DecodeError),
    #[error("invalid header text: {0}")]
    HeaderText(#[from] ToStrError),
    #[error("WebSocket transport: {0}")]
    Socket(#[source] Box<Error>),
    #[error("WebSocket I/O: {0}")]
    Io(#[from] io::Error),
    #[error("WebSocket deadline: {0}")]
    Timeout(#[from] Elapsed),
    #[error("WebSocket task: {0}")]
    Task(#[from] JoinError),
    #[error("invalid header name: {0}")]
    HeaderName(#[from] InvalidHeaderName),
    #[error("invalid header value: {0}")]
    HeaderValue(#[from] InvalidHeaderValue),
    #[error("WebSocket transcript: {0}")]
    Transport(String),
    #[error("service adapter {0:?} is not registered")]
    AdapterMissing(ServiceTag),
    #[error("service adapter {0:?} is already registered")]
    DuplicateAdapter(ServiceTag),
}

impl From<Error> for DriverError {
    fn from(source: Error) -> Self {
        Self::Socket(Box::new(source))
    }
}
