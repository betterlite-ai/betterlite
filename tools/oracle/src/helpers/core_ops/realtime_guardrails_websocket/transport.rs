use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use betterlite_core::api::realtime_contracts::error::WebSocketError;
use betterlite_core::api::realtime_contracts::transport::{
    ClientWebSocket, RealtimeBackend, RealtimeMessage, ScopedWebSocket, WebSocketScope,
};

// A poisoned lock is recovered instead of panicking, since no code here can leave the guarded data inconsistent
pub(crate) trait Locked<T> {
    fn locked(&self) -> MutexGuard<'_, T>;
}

impl<T> Locked<T> for Mutex<T> {
    fn locked(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

pub(crate) type Timeline = Arc<Mutex<Vec<&'static str>>>;

// The socket script: attempt failures by index, the frames the socket accepted and how often it closed
#[derive(Debug)]
pub(crate) struct WireLog {
    attempts: usize,
    failures: Vec<usize>,
    pub(crate) sent: Vec<String>,
    pub(crate) closes: usize,
}

impl WireLog {
    pub(crate) fn new(failures: Vec<usize>) -> Self {
        Self {
            attempts: 0,
            failures,
            sent: Vec::new(),
            closes: 0,
        }
    }

    pub(crate) fn begin(&mut self, target: &str) -> Result<(), WebSocketError> {
        let attempt = self.attempts;
        self.attempts = self.attempts.saturating_add(1);
        if self.failures.contains(&attempt) {
            return Err(WebSocketError {
                message: format!("the scripted {target} send failed"),
            });
        }
        Ok(())
    }

    fn accept(&mut self, target: &str, text: String) -> Result<(), WebSocketError> {
        self.begin(target)?;
        self.sent.push(text);
        Ok(())
    }
}

pub(crate) struct ScriptedBackend {
    pub(crate) log: Arc<Mutex<WireLog>>,
    pub(crate) timeline: Timeline,
    pub(crate) close_failure: bool,
}

#[async_trait]
impl RealtimeBackend for ScriptedBackend {
    async fn send(&mut self, message: RealtimeMessage) -> Result<(), WebSocketError> {
        let RealtimeMessage::Text(text) = message else {
            return Err(WebSocketError {
                message: String::from("the guardrail runner sends no binary frames"),
            });
        };
        self.log.locked().accept("backend", text)?;
        self.timeline.locked().push("backend");
        Ok(())
    }

    async fn recv(&mut self, _decode: Option<bool>) -> Result<RealtimeMessage, WebSocketError> {
        Err(WebSocketError {
            message: String::from("the scripted backend never receives"),
        })
    }

    async fn close(&mut self) -> Result<(), WebSocketError> {
        self.timeline.locked().push("backend-close");
        {
            let mut log = self.log.locked();
            log.closes = log.closes.saturating_add(1);
        }
        if self.close_failure {
            return Err(WebSocketError {
                message: String::from("the scripted backend close failed"),
            });
        }
        Ok(())
    }
}

pub(crate) struct ScriptedClient {
    pub(crate) log: Arc<Mutex<WireLog>>,
    pub(crate) timeline: Timeline,
    pub(crate) scope: WebSocketScope,
}

#[async_trait]
impl ClientWebSocket for ScriptedClient {
    async fn send_text(&mut self, data: &str) -> Result<(), WebSocketError> {
        self.log.locked().accept("client", data.to_owned())?;
        self.timeline.locked().push("client");
        Ok(())
    }

    async fn receive_text(&mut self) -> Result<String, WebSocketError> {
        Err(WebSocketError {
            message: String::from("the scripted client never receives"),
        })
    }

    async fn close(&mut self, _code: u16, _reason: Option<&str>) -> Result<(), WebSocketError> {
        let mut log = self.log.locked();
        log.closes = log.closes.saturating_add(1);
        Ok(())
    }
}

impl ScopedWebSocket for ScriptedClient {
    fn scope(&self) -> &WebSocketScope {
        &self.scope
    }
}
