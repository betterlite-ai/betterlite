#![cfg(test)]

use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_config::sections::audit_events::Settings;
use betterlite_config::sections::settings::JsonValue;
use chrono::{DateTime, FixedOffset};
use secrecy::SecretString;

use crate::audit_events::callbacks::{
    AuditCallbackDispatcher, AuditCallbackError, AuditCallbackFuture, AuditCallbackRegistry,
    AuditLogCallback, AuditTaskSpawner,
};
use crate::audit_events::identity::CallerIdentity;
use crate::audit_events::record::AuditLogRecord;
use crate::audit_events::settings::ConfiguredAuditSettings;
use crate::audit_events::sink::{
    AuditClock, AuditEventSink, AuditLogRepository, AuditRepositoryError,
};

const SAMPLE_TIMESTAMP: &str = "2026-01-02T03:04:05.123456+00:00";

pub(super) fn parsed(source: &str) -> JsonValue {
    serde_json::from_str(source).expect("valid JSON fixture")
}

pub(super) struct RecordingRepository {
    pub(super) inserted: Mutex<Vec<AuditLogRecord>>,
}

#[async_trait]
impl AuditLogRepository for RecordingRepository {
    fn available(&self) -> bool {
        true
    }

    async fn insert_audit_log(&self, record: &AuditLogRecord) -> Result<(), AuditRepositoryError> {
        self.inserted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(record.clone());
        Ok(())
    }
}

pub(super) struct NoCallbacks;

impl AuditCallbackRegistry for NoCallbacks {
    fn resolve(&self, _name: &str) -> Option<Arc<dyn AuditLogCallback>> {
        None
    }

    fn build_dedicated_s3_logger(
        &self,
        _params: &JsonValue,
    ) -> Result<Arc<dyn AuditLogCallback>, AuditCallbackError> {
        Err(AuditCallbackError::Failed)
    }
}

// audit logging is on and no callback is configured, so only the row write matters here
pub(super) struct DroppingSpawner;

impl AuditTaskSpawner for DroppingSpawner {
    fn spawn(&self, task: AuditCallbackFuture) {
        drop(task);
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

pub(super) fn enabled_sink(repository: Arc<RecordingRepository>) -> AuditEventSink {
    let now = DateTime::parse_from_rfc3339(SAMPLE_TIMESTAMP).expect("a valid timestamp");
    AuditEventSink {
        settings: Arc::new(ConfiguredAuditSettings {
            config: Settings {
                store_audit_logs: Some(true),
                store_audit_logs_env: None,
                audit_log_callbacks: Vec::new(),
                s3_audit_callback_params: None,
            },
            callbacks: Vec::new(),
        }),
        callbacks: Arc::new(AuditCallbackDispatcher::new(
            Arc::new(NoCallbacks),
            Arc::new(DroppingSpawner),
        )),
        repository,
        clock: Arc::new(FixedClock { now }),
        new_id: Arc::new(|| "audit-id-1".to_owned()),
    }
}

pub(super) fn caller(user_id: Option<&str>) -> CallerIdentity {
    CallerIdentity {
        user_id: user_id.map(str::to_owned),
        api_key: Some(SecretString::from("sk-key".to_owned())),
        metadata: None,
        team_metadata: None,
    }
}
