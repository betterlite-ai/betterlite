#![cfg(test)]

use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_config::sections::settings::JsonValue;
use chrono::{DateTime, FixedOffset};
use secrecy::SecretString;
use tracing::subscriber::{DefaultGuard, set_default};

use crate::audit_events::callbacks::{
    AuditCallbackConfig, AuditCallbackDispatcher, AuditCallbackError, AuditCallbackFuture,
    AuditCallbackRegistry, AuditLogCallback, AuditTaskSpawner,
};
use crate::audit_events::enums::{AuditAction, AuditTable};
use crate::audit_events::payload::AuditLogPayload;
use crate::audit_events::record::AuditLogRecord;
use crate::audit_events::settings::AuditSettings;
use crate::audit_events::sink::{
    AuditClock, AuditEventSink, AuditLogRepository, AuditRepositoryError,
};

pub(super) const SAMPLE_TIMESTAMP: &str = "2026-01-02T03:04:05.123456+00:00";

pub(super) fn parsed(text: &str) -> JsonValue {
    serde_json::from_str(text).unwrap()
}

pub(super) struct TestSettings {
    pub(super) enabled: bool,
    pub(super) callbacks: Vec<AuditCallbackConfig>,
    pub(super) s3: Option<JsonValue>,
}

impl AuditSettings for TestSettings {
    fn is_audit_logging_enabled(&self, explicit: Option<bool>) -> bool {
        explicit.unwrap_or(self.enabled)
    }

    fn audit_log_callbacks(&self) -> &[AuditCallbackConfig] {
        &self.callbacks
    }

    fn s3_audit_callback_params(&self) -> Option<&JsonValue> {
        self.s3.as_ref()
    }
}

pub(super) struct RecordingCallback {
    pub(super) label: String,
    pub(super) fails: bool,
    pub(super) delivered: Mutex<Vec<AuditLogPayload>>,
    pub(super) order: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl AuditLogCallback for RecordingCallback {
    async fn async_log_audit_log_event(
        &self,
        payload: AuditLogPayload,
    ) -> Result<(), AuditCallbackError> {
        if self.fails {
            return Err(AuditCallbackError::Failed);
        }
        self.order
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(self.label.clone());
        self.delivered
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(payload);
        Ok(())
    }
}

pub(super) struct TestRegistry {
    pub(super) instances: Mutex<BTreeMap<String, Arc<dyn AuditLogCallback>>>,
    pub(super) resolves: Mutex<Vec<String>>,
    pub(super) s3_calls: Mutex<Vec<JsonValue>>,
    pub(super) s3_logger: Arc<RecordingCallback>,
    pub(super) s3_fails: bool,
}

pub(super) fn recorder(label: &str, fails: bool) -> Arc<RecordingCallback> {
    let callback = RecordingCallback {
        label: label.to_owned(),
        fails,
        delivered: Mutex::new(Vec::new()),
        order: Arc::new(Mutex::new(Vec::new())),
    };
    Arc::new(callback)
}

pub(super) fn instance(callback: Arc<RecordingCallback>) -> AuditCallbackConfig {
    let erased: Arc<dyn AuditLogCallback> = callback;
    AuditCallbackConfig::Instance(erased)
}

pub(super) fn recording_repository() -> Arc<RecordingRepository> {
    let repository = RecordingRepository {
        available: true,
        fail: false,
        inserted: Mutex::new(Vec::new()),
    };
    Arc::new(repository)
}

pub(super) fn audit_sink(
    settings: TestSettings,
    repository: Arc<RecordingRepository>,
    s3_logger: Arc<RecordingCallback>,
    spawner: Arc<CollectingSpawner>,
) -> AuditEventSink {
    let registry = Arc::new(TestRegistry {
        instances: Mutex::new(BTreeMap::new()),
        resolves: Mutex::new(Vec::new()),
        s3_calls: Mutex::new(Vec::new()),
        s3_logger,
        s3_fails: false,
    });
    let now = DateTime::parse_from_rfc3339(SAMPLE_TIMESTAMP).unwrap();
    AuditEventSink {
        settings: Arc::new(settings),
        callbacks: Arc::new(AuditCallbackDispatcher::new(registry, spawner)),
        repository,
        clock: Arc::new(FixedClock { now }),
        new_id: Arc::new(|| "audit-id-1".to_owned()),
    }
}

impl AuditCallbackRegistry for TestRegistry {
    fn resolve(&self, name: &str) -> Option<Arc<dyn AuditLogCallback>> {
        self.resolves
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(name.to_owned());
        let instances = self
            .instances
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        instances.get(name).cloned()
    }

    fn build_dedicated_s3_logger(
        &self,
        params: &JsonValue,
    ) -> Result<Arc<dyn AuditLogCallback>, AuditCallbackError> {
        self.s3_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(params.clone());
        if self.s3_fails {
            return Err(AuditCallbackError::Failed);
        }
        let logger: Arc<dyn AuditLogCallback> = Arc::clone(&self.s3_logger);
        Ok(logger)
    }
}

#[derive(Default)]
pub(super) struct CollectingSpawner {
    pub(super) tasks: Mutex<Vec<AuditCallbackFuture>>,
}

impl CollectingSpawner {
    pub(super) async fn run_all(&self) {
        let tasks: Vec<AuditCallbackFuture> = self
            .tasks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain(..)
            .collect();
        for task in tasks {
            task.await;
        }
    }
}

impl AuditTaskSpawner for CollectingSpawner {
    fn spawn(&self, task: AuditCallbackFuture) {
        self.tasks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(task);
    }
}

pub(super) struct DroppingSpawner;

impl AuditTaskSpawner for DroppingSpawner {
    fn spawn(&self, task: AuditCallbackFuture) {
        drop(task);
    }
}

pub(super) struct RecordingRepository {
    pub(super) available: bool,
    pub(super) fail: bool,
    pub(super) inserted: Mutex<Vec<AuditLogRecord>>,
}

#[async_trait]
impl AuditLogRepository for RecordingRepository {
    fn available(&self) -> bool {
        self.available
    }

    async fn insert_audit_log(&self, record: &AuditLogRecord) -> Result<(), AuditRepositoryError> {
        if self.fail {
            return Err(AuditRepositoryError::Unavailable);
        }
        self.inserted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(record.clone());
        Ok(())
    }
}

pub(super) struct FixedClock {
    pub(super) now: DateTime<FixedOffset>,
}

impl AuditClock for FixedClock {
    fn now(&self) -> DateTime<FixedOffset> {
        self.now
    }
}

pub(super) fn plain_record() -> AuditLogRecord {
    AuditLogRecord::masked(AuditLogRecord {
        id: "audit-1".to_owned(),
        updated_at: DateTime::parse_from_rfc3339(SAMPLE_TIMESTAMP).unwrap(),
        changed_by: None,
        changed_by_api_key: None,
        action: AuditAction::Updated,
        table_name: AuditTable::Team,
        object_id: "team-1".to_owned(),
        before_value: None,
        updated_values: None,
    })
    .unwrap()
}

pub(super) fn sample_record(
    before_value: Option<&str>,
    updated_values: Option<&str>,
) -> AuditLogRecord {
    AuditLogRecord::masked(AuditLogRecord {
        id: "audit-1".to_owned(),
        updated_at: DateTime::parse_from_rfc3339(SAMPLE_TIMESTAMP).unwrap(),
        changed_by: Some("admin".to_owned()),
        changed_by_api_key: Some(SecretString::from("sk-key".to_owned())),
        action: AuditAction::Updated,
        table_name: AuditTable::Team,
        object_id: "team-1".to_owned(),
        before_value: before_value.map(|text| JsonValue::String(text.to_owned())),
        updated_values: updated_values.map(|text| JsonValue::String(text.to_owned())),
    })
    .unwrap()
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
