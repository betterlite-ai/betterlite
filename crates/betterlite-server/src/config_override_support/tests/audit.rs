#![cfg(test)]

use std::collections::BTreeMap;
use std::fs::read_to_string;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::audit::Settings as AuditSettings;
use betterlite_config::sections::audit_events::Settings as SectionSettings;
use betterlite_config::sections::settings::JsonValue;
use betterlite_logging::audit::callbacks::{
    AuditLoggerBuildError, DedicatedAuditLoggerBuilder, NamedAuditCallbacks,
};
use betterlite_logging::audit::delivery::AuditDelivery;
use betterlite_logging::callback_logger::CallbackLogger;
use betterlite_logging::callback_logger::resolver::LoggerResolver;
use betterlite_logging::registry::catalog::CatalogError;
use betterlite_logging::registry::error::RegistryError;
use betterlite_logging::worker::{LoggingWorker, Rejection, WorkerSettings};
use betterlite_openai::openai_base_config::Object as JsonMap;
use chrono::{DateTime, FixedOffset};
use secrecy::SecretString;
use tempfile::NamedTempFile;
use tracing::subscriber::{DefaultGuard, set_default};

use super::{object, value};
use crate::audit::service::{AuditService, AuditServiceDeps, AuditSubmission};
use crate::audit_events::enums::{AuditAction, AuditTable};
use crate::audit_events::record::AuditLogRecord;
use crate::audit_events::sink::{AuditClock, AuditLogRepository, AuditRepositoryError};
use crate::config_override_support::audit::{
    AUDIT_REDACTED, ConfigOverrideAudit, OverrideAuditEntry, redact_config,
    submit_override_audit_log,
};

struct RecordingAudit {
    records: Mutex<Vec<AuditLogRecord>>,
    submission: AuditSubmission,
}

impl RecordingAudit {
    const fn new(submission: AuditSubmission) -> Self {
        Self {
            records: Mutex::new(Vec::new()),
            submission,
        }
    }
}

impl ConfigOverrideAudit for RecordingAudit {
    fn new_record_id(&self) -> String {
        "record-id".to_owned()
    }

    fn record_time(&self) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339("2024-01-01T00:00:00+00:00").unwrap()
    }

    fn submit(&self, record: AuditLogRecord) -> AuditSubmission {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(record);
        self.submission
    }
}

fn entry() -> OverrideAuditEntry<'static> {
    OverrideAuditEntry {
        object_id: "cyberark",
        action: AuditAction::Created,
        before_config: None,
        after_config: None,
        litellm_changed_by: None,
        user_id: None,
        admin_name: None,
        api_key: None,
    }
}

fn snapshot(stored: Option<&JsonValue>) -> JsonValue {
    match stored {
        Some(JsonValue::String(text)) => value(text),
        other => panic!("the audit snapshot is not JSON text: {other:?}"),
    }
}

fn submitted_record(
    audit: &RecordingAudit,
    before: Option<&JsonMap>,
    after: Option<&JsonMap>,
) -> AuditLogRecord {
    let submission = submit_override_audit_log(
        audit,
        OverrideAuditEntry {
            object_id: "hashicorp_vault",
            action: AuditAction::Updated,
            before_config: before,
            after_config: after,
            litellm_changed_by: Some("header-user"),
            user_id: Some("user-id"),
            admin_name: Some("admin"),
            api_key: Some(SecretString::from("sk-123")),
        },
    );
    assert_eq!(submission, AuditSubmission::Accepted);
    let records = audit.records.lock().unwrap_or_else(PoisonError::into_inner);
    records[0].clone()
}

#[test]
fn the_audit_record_carries_the_entry_fields() {
    let audit = RecordingAudit::new(AuditSubmission::Accepted);
    let record = submitted_record(&audit, None, None);
    assert_eq!(record.id, "record-id");
    assert_eq!(record.table_name, AuditTable::ConfigOverrides);
    assert_eq!(record.action, AuditAction::Updated);
    assert_eq!(record.object_id, "hashicorp_vault");
    assert_eq!(record.changed_by.as_deref(), Some("header-user"));
    assert!(record.changed_by_api_key.is_some());
    assert_eq!(
        record.updated_at,
        DateTime::parse_from_rfc3339("2024-01-01T00:00:00+00:00").unwrap()
    );
}

#[test]
fn redacts_every_value_and_keeps_the_keys() {
    let before = object(r#"{"vault_token": "hunter2", "vault_addr": "https://vault.example.com"}"#);
    let after = object(r#"{"vault_token": "hunter3"}"#);
    let audit = RecordingAudit::new(AuditSubmission::Accepted);
    let record = submitted_record(&audit, Some(&before), Some(&after));
    assert_eq!(
        snapshot(record.before_value.as_ref()),
        value(&format!(
            r#"{{"config": {{"vault_addr": "{AUDIT_REDACTED}", "vault_token": "{AUDIT_REDACTED}"}}}}"#
        ))
    );
    assert_eq!(
        snapshot(record.updated_values.as_ref()),
        value(&format!(
            r#"{{"config": {{"vault_token": "{AUDIT_REDACTED}"}}}}"#
        ))
    );
}

#[test]
fn an_empty_or_missing_snapshot_is_an_empty_config() {
    assert!(redact_config(None).is_empty());
    assert!(redact_config(Some(&JsonMap::new())).is_empty());
    let audit = RecordingAudit::new(AuditSubmission::Accepted);
    assert_eq!(
        submit_override_audit_log(
            &audit,
            OverrideAuditEntry {
                object_id: "hashicorp_vault",
                action: AuditAction::Deleted,
                before_config: None,
                after_config: Some(&JsonMap::new()),
                litellm_changed_by: None,
                user_id: None,
                admin_name: None,
                api_key: None,
            },
        ),
        AuditSubmission::Accepted
    );
    let record = {
        let records = audit.records.lock().unwrap_or_else(PoisonError::into_inner);
        records[0].clone()
    };
    assert_eq!(
        snapshot(record.before_value.as_ref()),
        value(r#"{"config": {}}"#)
    );
    assert_eq!(
        snapshot(record.updated_values.as_ref()),
        value(r#"{"config": {}}"#)
    );
    assert_eq!(record.changed_by, None);
}

#[test]
fn changed_by_falls_back_from_the_header_to_the_user_then_the_admin() {
    let audit = RecordingAudit::new(AuditSubmission::Accepted);
    let entries = [
        OverrideAuditEntry {
            litellm_changed_by: Some(""),
            user_id: Some("user-id"),
            admin_name: Some("admin"),
            ..entry()
        },
        OverrideAuditEntry {
            litellm_changed_by: Some("header-user"),
            user_id: None,
            admin_name: Some("admin"),
            ..entry()
        },
        OverrideAuditEntry {
            litellm_changed_by: None,
            user_id: None,
            admin_name: Some("admin"),
            ..entry()
        },
    ];
    let submissions = entries.map(|submitted| submit_override_audit_log(&audit, submitted));
    assert_eq!(submissions, [AuditSubmission::Accepted; 3]);
    let names = {
        let records = audit.records.lock().unwrap_or_else(PoisonError::into_inner);
        records
            .iter()
            .map(|record| record.changed_by.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        names,
        vec![
            Some("user-id".to_owned()),
            Some("header-user".to_owned()),
            Some("admin".to_owned())
        ]
    );
}

fn capture_warnings() -> (NamedTempFile, DefaultGuard) {
    let log = NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    (log, set_default(subscriber))
}

#[test]
fn a_rejected_audit_record_warns_once() {
    let (log, _guard) = capture_warnings();
    assert_eq!(
        submit_override_audit_log(&RecordingAudit::new(AuditSubmission::Accepted), entry()),
        AuditSubmission::Accepted
    );
    assert_eq!(
        submit_override_audit_log(&RecordingAudit::new(AuditSubmission::Disabled), entry()),
        AuditSubmission::Disabled
    );
    let rejected = AuditSubmission::Rejected(Rejection::QueueFull);
    assert_eq!(
        submit_override_audit_log(&RecordingAudit::new(rejected), entry()),
        rejected
    );
    let output = read_to_string(log.path()).unwrap();
    assert_eq!(
        output
            .matches("Failed to write config override audit log")
            .count(),
        1
    );
    assert!(output.contains("WARN"));
}

struct RecordingRepository {
    inserted: Mutex<Vec<AuditLogRecord>>,
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

struct InertResolver;

impl LoggerResolver for InertResolver {
    fn resolve(
        &self,
        _name: &str,
        _params: &BTreeMap<String, String>,
    ) -> Result<Option<Arc<dyn CallbackLogger>>, RegistryError> {
        Ok(None)
    }
}

struct RefusingDedicatedLogger;

impl DedicatedAuditLoggerBuilder for RefusingDedicatedLogger {
    fn build(&self, _params: &JsonValue) -> Result<Arc<dyn CallbackLogger>, AuditLoggerBuildError> {
        Err(AuditLoggerBuildError::Build {
            source: CatalogError::Unavailable {
                name: "none".to_owned(),
            },
        })
    }
}

fn fixed_time() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2024-05-06T07:08:09.123+00:00").unwrap()
}

struct FixedClock;

impl AuditClock for FixedClock {
    fn now(&self) -> DateTime<FixedOffset> {
        fixed_time()
    }
}

fn audit_service() -> (AuditService, Arc<RecordingRepository>) {
    let workers = WorkerSettings {
        max_queue_size: 4,
        concurrency: 1,
        event_timeout: Duration::from_secs(5),
        shutdown_drain: Duration::from_secs(5),
        timeout_summary_window: Duration::from_secs(1),
    };
    let repository = Arc::new(RecordingRepository {
        inserted: Mutex::new(Vec::new()),
    });
    let delivery = Arc::new(AuditDelivery {
        named: NamedAuditCallbacks {
            resolver: Arc::new(InertResolver),
            dedicated: Arc::new(RefusingDedicatedLogger),
            cache: Mutex::new(BTreeMap::new()),
        },
        worker: LoggingWorker::start(workers).expect("the delivery starts"),
    });
    let service = AuditService::start(AuditServiceDeps {
        settings: AuditSettings {
            config: SectionSettings {
                store_audit_logs: Some(true),
                store_audit_logs_env: None,
                audit_log_callbacks: Vec::new(),
                s3_audit_callback_params: None,
            },
            audit_log_callbacks_present: false,
        },
        callbacks: Vec::new(),
        delivery,
        repository: Arc::clone(&repository),
        clock: Arc::new(FixedClock),
        new_id: Arc::new(|| "adapter-id".to_owned()),
        worker_settings: workers,
    })
    .expect("the audit service starts");
    (service, repository)
}

#[tokio::test]
async fn the_audit_service_adapter_uses_the_service_id_and_clock() {
    let (service, repository) = audit_service();
    assert_eq!(service.new_record_id(), "adapter-id");
    assert_eq!(service.record_time(), fixed_time());
    assert_eq!(
        submit_override_audit_log(&service, entry()),
        AuditSubmission::Accepted
    );
    service.shutdown().await;
    let inserted = repository
        .inserted
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    assert_eq!(inserted.len(), 1);
    assert_eq!(inserted[0].id, "adapter-id");
    assert_eq!(inserted[0].updated_at, fixed_time());
}
