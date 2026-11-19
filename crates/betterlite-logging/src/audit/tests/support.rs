#![cfg(test)]

use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::sections::settings::JsonValue;
use tracing::subscriber::{DefaultGuard, set_default};

use crate::audit::callbacks::{AuditLoggerBuildError, DedicatedAuditLoggerBuilder};
use crate::audit::payload::StandardAuditLogPayload;
use crate::callback_logger::CallbackLogger;
use crate::callback_logger::error::LoggerError;
use crate::callback_logger::resolver::LoggerResolver;
use crate::registry::error::RegistryError;
use crate::worker::WorkerSettings;

pub(super) fn payload() -> StandardAuditLogPayload {
    StandardAuditLogPayload {
        id: "audit-1".to_owned(),
        updated_at: "2026-01-02T03:04:05+00:00".to_owned(),
        changed_by: "admin".to_owned(),
        hashed_changed_by_api_key: "sk-key".to_owned(),
        action: "updated".to_owned(),
        table_name: "LiteLLM_TeamTable".to_owned(),
        object_id: "team-1".to_owned(),
        before_value: Some(r#"{"api_key":"sk-1*****7890"}"#.to_owned()),
        updated_values: None,
    }
}

pub(super) fn settings(queue: usize) -> WorkerSettings {
    WorkerSettings {
        max_queue_size: queue,
        concurrency: 1,
        event_timeout: Duration::from_secs(5),
        shutdown_drain: Duration::from_secs(5),
        timeout_summary_window: Duration::from_secs(1),
    }
}

pub(super) struct RecordingLogger {
    pub(super) name: String,
    pub(super) fails: bool,
    pub(super) delivered: Mutex<Vec<StandardAuditLogPayload>>,
}

impl RecordingLogger {
    pub(super) fn new(name: &str, fails: bool) -> Self {
        Self {
            name: name.to_owned(),
            fails,
            delivered: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl CallbackLogger for RecordingLogger {
    fn callback_name(&self) -> &str {
        &self.name
    }

    async fn async_log_audit_log_event(
        &self,
        audit_log: &StandardAuditLogPayload,
    ) -> Result<(), LoggerError> {
        if self.fails {
            return Err(LoggerError::Delivery {
                callback: self.name.clone(),
                message: "the audit log callback failed".to_owned(),
            });
        }
        self.delivered
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(audit_log.clone());
        Ok(())
    }
}

pub(super) struct PlainLogger;

#[async_trait]
impl CallbackLogger for PlainLogger {
    fn callback_name(&self) -> &'static str {
        "plain"
    }
}

pub(super) struct FakeResolver {
    pub(super) loggers: BTreeMap<String, Arc<dyn CallbackLogger>>,
    pub(super) calls: Mutex<Vec<String>>,
    pub(super) refuse: bool,
}

impl FakeResolver {
    pub(super) fn new(loggers: Vec<(&str, Arc<dyn CallbackLogger>)>, refuse: bool) -> Self {
        Self {
            loggers: loggers
                .into_iter()
                .map(|(name, logger)| (name.to_owned(), logger))
                .collect(),
            calls: Mutex::new(Vec::new()),
            refuse,
        }
    }
}

impl LoggerResolver for FakeResolver {
    fn resolve(
        &self,
        name: &str,
        _params: &BTreeMap<String, String>,
    ) -> Result<Option<Arc<dyn CallbackLogger>>, RegistryError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(name.to_owned());
        if self.refuse {
            return Err(RegistryError::UnavailableAdapter {
                name: name.to_owned(),
            });
        }
        Ok(self.loggers.get(name).cloned())
    }
}

pub(super) struct FakeBuilder {
    pub(super) calls: Mutex<Vec<JsonValue>>,
    pub(super) logger: Arc<RecordingLogger>,
}

impl FakeBuilder {
    pub(super) fn new(logger: Arc<RecordingLogger>) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            logger,
        }
    }
}

impl DedicatedAuditLoggerBuilder for FakeBuilder {
    fn build(&self, params: &JsonValue) -> Result<Arc<dyn CallbackLogger>, AuditLoggerBuildError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(params.clone());
        let logger: Arc<dyn CallbackLogger> = Arc::<RecordingLogger>::clone(&self.logger);
        Ok(logger)
    }
}

pub(super) struct FailingBuilder;

impl DedicatedAuditLoggerBuilder for FailingBuilder {
    fn build(&self, _params: &JsonValue) -> Result<Arc<dyn CallbackLogger>, AuditLoggerBuildError> {
        Err(AuditLoggerBuildError::Parameters {
            source: serde_json::from_str::<JsonValue>("{oops").unwrap_err(),
        })
    }
}

#[derive(Clone, Default)]
pub(super) struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    pub(super) fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) fn capture() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .finish();
    (captured, set_default(subscriber))
}
