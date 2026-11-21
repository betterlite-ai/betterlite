use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use betterlite_core::api::realtime_contracts::error::WebSocketError;
use betterlite_core::api::realtime_contracts::transport::{
    ClientWebSocket, RealtimeBackend, RealtimeMessage, ScopedWebSocket, WebSocketScope,
};
use serde_json::{Value, json};

pub(crate) type Frames = Arc<Mutex<Vec<String>>>;
pub(crate) type Closes = Arc<Mutex<Vec<(u16, Option<String>)>>>;

pub(crate) trait Locked<T> {
    fn locked(&self) -> MutexGuard<'_, T>;
}

impl<T> Locked<T> for Mutex<T> {
    fn locked(&self) -> MutexGuard<'_, T> {
        let guard = self.lock();
        guard.unwrap_or_else(PoisonError::into_inner)
    }
}

pub(crate) fn recorded(data: &str) -> Value {
    serde_json::from_str::<Value>(data).map_or_else(
        |_error| json!({"kind": "text", "text": data}),
        |value| json!({"kind": "json", "value": value}),
    )
}

pub(crate) fn recorded_frames(log: &Frames) -> Vec<Value> {
    log.locked()
        .iter()
        .map(|text| recorded(text.as_str()))
        .collect()
}

pub(crate) fn recorded_closes(closes: &Closes) -> Vec<Value> {
    closes
        .locked()
        .iter()
        .map(|(code, reason)| json!([code, reason]))
        .collect()
}

#[derive(Default)]
pub(crate) struct ClientData {
    pub(crate) incoming: VecDeque<String>,
    pub(crate) log: Frames,
    pub(crate) closes: Closes,
    pub(crate) attempts: usize,
    pub(crate) failures: Vec<usize>,
    pub(crate) close_attempts: usize,
    pub(crate) close_failures: Vec<usize>,
}

pub(crate) struct ScriptedClient {
    pub(crate) data: Arc<Mutex<ClientData>>,
    pub(crate) scope: WebSocketScope,
}

impl ScopedWebSocket for ScriptedClient {
    fn scope(&self) -> &WebSocketScope {
        &self.scope
    }
}

#[async_trait]
impl ClientWebSocket for ScriptedClient {
    async fn send_text(&mut self, data: &str) -> Result<(), WebSocketError> {
        let (attempt, refused) = {
            let mut state = self.data.locked();
            let attempt = state.attempts;
            state.attempts = state.attempts.saturating_add(1);
            (attempt, state.failures.contains(&attempt))
        };
        if refused {
            return Err(WebSocketError {
                message: format!("the scripted client send {attempt} failed"),
            });
        }
        self.data.locked().log.locked().push(data.to_owned());
        Ok(())
    }

    async fn receive_text(&mut self) -> Result<String, WebSocketError> {
        let next = self.data.locked().incoming.pop_front();
        next.ok_or_else(|| WebSocketError {
            message: String::from("the scripted client is exhausted"),
        })
    }

    async fn close(&mut self, code: u16, reason: Option<&str>) -> Result<(), WebSocketError> {
        let (attempt, refused) = {
            let mut state = self.data.locked();
            let attempt = state.close_attempts;
            state.close_attempts = state.close_attempts.saturating_add(1);
            (attempt, state.close_failures.contains(&attempt))
        };
        self.data
            .locked()
            .closes
            .locked()
            .push((code, reason.map(str::to_owned)));
        if refused {
            return Err(WebSocketError {
                message: format!("the scripted client close {attempt} failed"),
            });
        }
        Ok(())
    }
}

pub(crate) struct ScriptedBackend {
    pub(crate) log: Frames,
    pub(crate) failures: Vec<usize>,
    pub(crate) attempts: usize,
}

#[async_trait]
impl RealtimeBackend for ScriptedBackend {
    async fn send(&mut self, message: RealtimeMessage) -> Result<(), WebSocketError> {
        let text = match message {
            RealtimeMessage::Text(text) => text,
            RealtimeMessage::Binary(bytes) => {
                String::from_utf8(bytes).map_err(|error| WebSocketError {
                    message: error.to_string(),
                })?
            }
        };
        let attempt = self.attempts;
        self.attempts = self.attempts.saturating_add(1);
        if self.failures.contains(&attempt) {
            return Err(WebSocketError {
                message: format!("the scripted backend send {attempt} failed"),
            });
        }
        self.log.locked().push(text);
        Ok(())
    }

    async fn recv(&mut self, _decode: Option<bool>) -> Result<RealtimeMessage, WebSocketError> {
        Err(WebSocketError {
            message: String::from("the scripted backend does not read"),
        })
    }

    async fn close(&mut self) -> Result<(), WebSocketError> {
        Ok(())
    }
}
