use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_core::api::realtime_backend_relay::source::{
    RealtimeBackendSource, RelayReceiveError,
};
use betterlite_core::api::realtime_backend_transport::dial::{
    ConnectedBackend, DialRequest, RealtimeSsl, RealtimeWebSocketDialer,
};
use betterlite_core::api::realtime_backend_transport::error::{ClientCloseError, DialError};
use betterlite_core::api::realtime_backend_transport::protocol::RealtimeClientSocket;
use betterlite_core::api::realtime_contracts::close::{CloseFrame, ConnectionClosed};
use betterlite_core::api::realtime_contracts::error::{HeaderMap, WebSocketError};
use betterlite_core::api::realtime_contracts::transport::{RealtimeBackend, RealtimeMessage};
use betterlite_models::decisions::Json;

use super::input::{CloseErrorDocument, DialDocument};

pub(crate) type Frames = Arc<Mutex<Vec<String>>>;

pub(crate) type CloseRecord = (u16, Option<String>);

// A poisoned lock is recovered instead of panicking, since no code here can leave the guarded data inconsistent
pub(crate) trait Locked<T> {
    fn locked(&self) -> MutexGuard<'_, T>;
}

impl<T> Locked<T> for Mutex<T> {
    fn locked(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

// The same three kinds the Python helper records, so an SSL argument mismatch shows up as a pointer difference
pub(crate) const fn ssl_name_of(ssl: &RealtimeSsl) -> &'static str {
    match ssl {
        RealtimeSsl::Disabled => "disabled",
        RealtimeSsl::Insecure => "insecure",
        RealtimeSsl::Shared(_) => "shared",
    }
}

pub(crate) struct DialRecord {
    pub(crate) url: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) ssl: RealtimeSsl,
    pub(crate) max_size: usize,
    pub(crate) open_timeout: Duration,
}

pub(crate) struct RecordingBackend {
    pub(crate) sent: Frames,
    pub(crate) closed: Arc<Mutex<bool>>,
}

#[async_trait]
impl RealtimeBackend for RecordingBackend {
    async fn send(&mut self, message: RealtimeMessage) -> Result<(), WebSocketError> {
        let text = match message {
            RealtimeMessage::Text(text) => text,
            RealtimeMessage::Binary(bytes) => {
                String::from_utf8(bytes).map_err(|error| WebSocketError {
                    message: error.to_string(),
                })?
            }
        };
        self.sent.locked().push(text);
        Ok(())
    }

    async fn recv(&mut self, _decode: Option<bool>) -> Result<RealtimeMessage, WebSocketError> {
        Err(WebSocketError {
            message: String::from("the scripted backend never receives"),
        })
    }

    async fn close(&mut self) -> Result<(), WebSocketError> {
        *self.closed.locked() = true;
        Ok(())
    }
}

pub(crate) struct ClosedSource;

#[async_trait]
impl RealtimeBackendSource for ClosedSource {
    async fn recv(&mut self, _decode: Option<bool>) -> Result<RealtimeMessage, RelayReceiveError> {
        Err(RelayReceiveError::Closed(ConnectionClosed {
            received: Some(CloseFrame {
                code: 1000,
                reason: String::new(),
            }),
            message: String::from("upstream closed"),
        }))
    }
}

pub(crate) struct ScriptedDialer {
    pub(crate) attempts: Mutex<VecDeque<DialDocument>>,
    pub(crate) records: Arc<Mutex<Vec<DialRecord>>>,
    pub(crate) sent: Frames,
    pub(crate) closed: Arc<Mutex<bool>>,
}

fn header_pairs(headers: &HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| {
            let text = match value {
                Json::Text(text) => text.clone(),
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                    serde_json::to_string(value).unwrap_or_default()
                }
            };
            (name.clone(), text)
        })
        .collect()
}

fn record_dial(records: &Mutex<Vec<DialRecord>>, request: &DialRequest<'_>) {
    records.locked().push(DialRecord {
        url: request.url.to_owned(),
        headers: header_pairs(request.headers),
        ssl: request.ssl.clone(),
        max_size: request.max_size,
        open_timeout: request.open_timeout,
    });
}

impl ScriptedDialer {
    fn attempt_result(
        &self,
        request: &DialRequest<'_>,
        attempt: DialDocument,
    ) -> Result<ConnectedBackend, DialError> {
        match attempt {
            DialDocument::Connect if request.url.starts_with("ws://") => Err(DialError::Other {
                message: String::from("ssl argument is incompatible with a ws:// URI"),
            }),
            DialDocument::Connect => Ok(ConnectedBackend {
                sender: Box::new(RecordingBackend {
                    sent: Arc::clone(&self.sent),
                    closed: Arc::clone(&self.closed),
                }),
                source: Some(Box::new(ClosedSource)),
            }),
            DialDocument::Timeout { message } => Err(DialError::Timeout { message }),
            DialDocument::Os { message } => Err(DialError::Os { message }),
            DialDocument::WebSocket { message } => Err(DialError::WebSocket { message }),
            DialDocument::InvalidStatus { status_code } => Err(DialError::InvalidStatus {
                status_code,
                message: String::new(),
            }),
            DialDocument::InvalidStatusCode {
                status_code,
                message,
            } => Err(DialError::InvalidStatusCode {
                status_code,
                message,
            }),
        }
    }
}

#[async_trait]
impl RealtimeWebSocketDialer for ScriptedDialer {
    async fn connect(&self, request: &DialRequest<'_>) -> Result<ConnectedBackend, DialError> {
        record_dial(&self.records, request);
        let attempt = self
            .attempts
            .locked()
            .pop_front()
            .unwrap_or_else(|| DialDocument::Os {
                message: String::from("the dial script ran out"),
            });
        self.attempt_result(request, attempt)
    }
}

pub(crate) struct ScriptedClient {
    pub(crate) sent: Frames,
    pub(crate) closed: Arc<Mutex<Option<CloseRecord>>>,
    pub(crate) send_error: Option<String>,
    pub(crate) close_error: Option<CloseErrorDocument>,
}

#[async_trait]
impl RealtimeClientSocket for ScriptedClient {
    async fn send_text(&mut self, data: &str) -> Result<(), WebSocketError> {
        if let Some(message) = &self.send_error {
            return Err(WebSocketError {
                message: message.clone(),
            });
        }
        self.sent.locked().push(data.to_owned());
        Ok(())
    }

    async fn receive_text(&mut self) -> Result<String, WebSocketError> {
        Err(WebSocketError {
            message: String::from("the scripted client never receives"),
        })
    }

    async fn close(&mut self, code: u16, reason: Option<&str>) -> Result<(), ClientCloseError> {
        *self.closed.locked() = Some((code, reason.map(str::to_owned)));
        match &self.close_error {
            None => Ok(()),
            Some(error) if error.error_class == "RuntimeError" => Err(ClientCloseError::Runtime {
                message: error.message.clone(),
            }),
            Some(error) => Err(ClientCloseError::Other {
                message: error.message.clone(),
            }),
        }
    }
}
