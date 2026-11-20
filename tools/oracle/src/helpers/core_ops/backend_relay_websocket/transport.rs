use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use betterlite_core::api::realtime_backend_relay::source::{
    RealtimeBackendSource, RelayReceiveError,
};
use betterlite_core::api::realtime_contracts::close::{CloseFrame, ConnectionClosed};
use betterlite_core::api::realtime_contracts::error::WebSocketError;
use betterlite_core::api::realtime_contracts::transport::{
    ClientWebSocket, RealtimeBackend, RealtimeMessage, ScopedWebSocket, WebSocketScope,
};
use betterlite_models::decisions::Json;
use serde_json::{Value, json};

use super::input::CloseDocument;

pub(crate) type Frames = Arc<Mutex<Vec<String>>>;

// A poisoned lock is recovered instead of panicking, since no code here can leave the guarded data inconsistent
pub(crate) trait Locked<T> {
    fn locked(&self) -> MutexGuard<'_, T>;
}

impl<T> Locked<T> for Mutex<T> {
    fn locked(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

// A frame is compared as the JSON it encodes, because each side re-encodes a rewritten frame its own way
pub(crate) fn recorded(data: &str) -> Value {
    serde_json::from_str::<Json>(data).map_or_else(
        |_error| json!({"kind": "text", "text": data}),
        |value| json!({"kind": "json", "value": value}),
    )
}

pub(crate) fn frames(log: &Frames) -> Vec<Value> {
    log.locked()
        .iter()
        .map(|text| recorded(text.as_str()))
        .collect()
}

pub(crate) struct ClientSink {
    pub(crate) scope: WebSocketScope,
    pub(crate) log: Frames,
}

impl ScopedWebSocket for ClientSink {
    fn scope(&self) -> &WebSocketScope {
        &self.scope
    }
}

#[async_trait]
impl ClientWebSocket for ClientSink {
    async fn send_text(&mut self, data: &str) -> Result<(), WebSocketError> {
        self.log.locked().push(data.to_owned());
        Ok(())
    }

    async fn receive_text(&mut self) -> Result<String, WebSocketError> {
        Err(WebSocketError {
            message: String::from("the scripted client never receives"),
        })
    }

    async fn close(&mut self, _code: u16, _reason: Option<&str>) -> Result<(), WebSocketError> {
        Ok(())
    }
}

pub(crate) struct RecordingBackend {
    pub(crate) log: Frames,
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
        self.log.locked().push(text);
        Ok(())
    }

    async fn recv(&mut self, _decode: Option<bool>) -> Result<RealtimeMessage, WebSocketError> {
        Err(WebSocketError {
            message: String::from("the scripted backend never receives"),
        })
    }

    async fn close(&mut self) -> Result<(), WebSocketError> {
        Ok(())
    }
}

pub(crate) struct ScriptedSource {
    pub(crate) frames: VecDeque<String>,
    pub(crate) close: Option<CloseDocument>,
    pub(crate) decode_calls: Vec<Option<bool>>,
}

#[async_trait]
impl RealtimeBackendSource for ScriptedSource {
    async fn recv(&mut self, decode: Option<bool>) -> Result<RealtimeMessage, RelayReceiveError> {
        self.decode_calls.push(decode);
        let Some(text) = self.frames.pop_front() else {
            let received = self.close.as_ref().map(|document| CloseFrame {
                code: document.code,
                reason: document.reason.clone(),
            });
            let message = match &received {
                Some(frame) => frame.reason.clone(),
                None => String::from("no close frame received or sent"),
            };
            return Err(RelayReceiveError::Closed(ConnectionClosed {
                message,
                received,
            }));
        };
        Ok(RealtimeMessage::Text(text))
    }
}
