use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use betterlite_core::api::realtime_contracts::error::WebSocketError;
use betterlite_core::api::realtime_contracts::transport::{RealtimeBackend, RealtimeMessage};
use betterlite_models::decisions::Json;
use futures_util::{SinkExt as _, StreamExt as _};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::Sender;
use tokio_tungstenite::tungstenite::{Bytes, Message, Utf8Bytes};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, accept_async};

// A poisoned lock is recovered instead of panicking, since no code here can leave the guarded data inconsistent
pub(crate) trait Locked<T> {
    fn locked(&self) -> MutexGuard<'_, T>;
}

impl<T> Locked<T> for Mutex<T> {
    fn locked(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct Recorded {
    pub(crate) wire: &'static str,
    pub(crate) payload: Value,
}

// A text frame is compared as the JSON it encodes, because each side re-encodes a rewritten frame its own way
pub(crate) fn recorded(wire: &'static str, data: &str) -> Recorded {
    let payload = serde_json::from_str::<Json>(data).map_or_else(
        |_error| json!({"kind": "text", "text": data}),
        |value| json!({"kind": "json", "value": value}),
    );
    Recorded { wire, payload }
}

#[derive(Debug, Default)]
pub(crate) struct WireLog {
    pub(crate) attempts: usize,
    pub(crate) delivered: usize,
    pub(crate) failures: Vec<usize>,
}

impl WireLog {
    // An injected attempt index fails before the frame reaches the socket, so a failed attempt is never recorded
    pub(crate) fn begin(&mut self) -> Result<(), WebSocketError> {
        let attempt = self.attempts;
        self.attempts = self.attempts.saturating_add(1);
        if self.failures.contains(&attempt) {
            return Err(WebSocketError {
                message: format!("the scripted backend send {attempt} failed"),
            });
        }
        Ok(())
    }
}

// Every accepted frame crosses a real loopback WebSocket to the mock upstream, which records it in arrival order
pub(crate) struct WireBackend {
    pub(crate) socket: WebSocketStream<MaybeTlsStream<TcpStream>>,
    pub(crate) log: Arc<Mutex<WireLog>>,
}

#[async_trait]
impl RealtimeBackend for WireBackend {
    async fn send(&mut self, message: RealtimeMessage) -> Result<(), WebSocketError> {
        self.log.locked().begin()?;
        let frame = match message {
            RealtimeMessage::Text(text) => Message::Text(Utf8Bytes::from(text)),
            RealtimeMessage::Binary(data) => Message::Binary(Bytes::from(data)),
        };
        self.socket
            .send(frame)
            .await
            .map_err(|error| WebSocketError {
                message: error.to_string(),
            })?;
        let mut log = self.log.locked();
        log.delivered = log.delivered.saturating_add(1);
        drop(log);
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

pub(crate) async fn relay(listener: TcpListener, sender: Sender<Recorded>) {
    let Ok((stream, _)) = listener.accept().await else {
        return;
    };
    let Ok(mut socket) = accept_async(stream).await else {
        return;
    };
    while let Some(Ok(message)) = socket.next().await {
        let frame = match message {
            Message::Text(text) => recorded("text", text.as_str()),
            Message::Binary(data) => Recorded {
                wire: "binary",
                payload: json!({"kind": "bytes", "bytes": data.as_ref()}),
            },
            Message::Ping(_) | Message::Pong(_) | Message::Close(_) | Message::Frame(_) => {
                continue;
            }
        };
        if sender.send(frame).await.is_err() {
            return;
        }
    }
}
