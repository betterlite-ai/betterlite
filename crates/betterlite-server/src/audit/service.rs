use std::convert::Infallible;
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::audit::Settings as AuditSettings;
use betterlite_config::sections::settings::JsonValue;
use betterlite_logging::audit::delivery::AuditDelivery;
use betterlite_logging::worker::{
    Admission, Callback, LoggingWorker, Rejection, ShutdownReport, WorkerError, WorkerSettings,
};

use super::callbacks::AuditCallbackEntry;
use super::write::{AuditWriteTarget, write_audit_log};
use crate::audit_events::identity::get_audit_log_changed_by;
use crate::audit_events::record::{AuditLogRecord, AuditRecordError};
use crate::audit_events::sink::{AuditClock, AuditLogRepository, ObjectAuditLog};
use crate::python_hooks::slots::LoadedCallback;

pub struct AuditServiceDeps {
    pub settings: AuditSettings,
    pub callbacks: Vec<AuditCallbackEntry>,
    pub delivery: Arc<AuditDelivery>,
    pub repository: Arc<dyn AuditLogRepository>,
    pub clock: Arc<dyn AuditClock>,
    pub new_id: Arc<dyn Fn() -> String + Send + Sync>,
    pub worker_settings: WorkerSettings,
}

pub struct AuditService {
    pub settings: AuditSettings,
    pub callbacks: Arc<Mutex<Vec<AuditCallbackEntry>>>,
    pub delivery: Arc<AuditDelivery>,
    pub repository: Arc<dyn AuditLogRepository>,
    pub clock: Arc<dyn AuditClock>,
    pub new_id: Arc<dyn Fn() -> String + Send + Sync>,
    pub writes: LoggingWorker<AuditWriteJob>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditSubmission {
    Disabled,
    Accepted,
    Rejected(Rejection),
}

impl AuditService {
    pub fn start(deps: AuditServiceDeps) -> Result<Self, WorkerError> {
        let writes = LoggingWorker::start(deps.worker_settings)?;
        let callbacks = Arc::new(Mutex::new(deps.callbacks));
        Ok(Self {
            settings: deps.settings,
            callbacks,
            delivery: deps.delivery,
            repository: deps.repository,
            clock: deps.clock,
            new_id: deps.new_id,
            writes,
        })
    }

    pub fn clear_audit_log_callbacks(&self) {
        self.callbacks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }

    pub fn append_audit_log_callback(&self, callback: LoadedCallback) {
        self.callbacks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(AuditCallbackEntry::from(callback));
    }

    pub async fn create_object_audit_log(&self, entry: ObjectAuditLog<'_>) {
        if !self.settings.is_audit_logging_enabled(None) {
            return;
        }
        match object_record(&entry, self.clock.as_ref(), self.new_id.as_ref()) {
            Ok(record) => self.create_audit_log_for_update(record).await,
            Err(error) => tracing::error!("Failed Creating audit log {error}"),
        }
    }

    pub async fn create_audit_log_for_update(&self, record: AuditLogRecord) {
        let entries = self
            .callbacks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        write_audit_log(
            AuditWriteTarget {
                settings: &self.settings,
                entries: &entries,
                delivery: &self.delivery,
                repository: self.repository.as_ref(),
            },
            record,
        )
        .await;
    }

    // A caller that cannot wait schedules the whole record: the queue's admission is synchronous, the write is not
    pub fn try_submit(&self, record: AuditLogRecord) -> AuditSubmission {
        if !self.settings.is_audit_logging_enabled(None) {
            return AuditSubmission::Disabled;
        }
        let job = AuditWriteJob {
            settings: self.settings.clone(),
            callbacks: Arc::clone(&self.callbacks),
            delivery: Arc::clone(&self.delivery),
            repository: Arc::clone(&self.repository),
            record,
        };
        match self.writes.enqueue(job) {
            Admission::Accepted => AuditSubmission::Accepted,
            Admission::Rejected(rejection) => {
                tracing::error!(
                    ?rejection,
                    "the audit log record was rejected; it reaches no callback or database"
                );
                AuditSubmission::Rejected(rejection)
            }
        }
    }

    pub async fn shutdown(&self) -> ShutdownReport {
        let report = self.writes.stop().await;
        let callbacks = self.delivery.shutdown().await;
        tracing::info!(
            writes = report.completed,
            callbacks = callbacks.completed,
            "the audit service stopped"
        );
        report
    }
}

pub struct AuditWriteJob {
    settings: AuditSettings,
    callbacks: Arc<Mutex<Vec<AuditCallbackEntry>>>,
    delivery: Arc<AuditDelivery>,
    repository: Arc<dyn AuditLogRepository>,
    record: AuditLogRecord,
}

impl Callback for AuditWriteJob {
    type Error = Infallible;

    fn name(&self) -> &str {
        "audit_log_write"
    }

    fn call(self) -> impl Future<Output = Result<(), Infallible>> + Send + 'static {
        async move {
            let entries = self
                .callbacks
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            write_audit_log(
                AuditWriteTarget {
                    settings: &self.settings,
                    entries: &entries,
                    delivery: &self.delivery,
                    repository: self.repository.as_ref(),
                },
                self.record,
            )
            .await;
            Ok(())
        }
    }
}

// create_object_audit_log: a fresh UUID and aware UTC time, the caller's api_key, and the attributed actor
fn object_record(
    entry: &ObjectAuditLog<'_>,
    clock: &dyn AuditClock,
    new_id: &(dyn Fn() -> String + Send + Sync),
) -> Result<AuditLogRecord, AuditRecordError> {
    let changed_by = get_audit_log_changed_by(
        entry.litellm_changed_by,
        entry.caller,
        entry.litellm_proxy_admin_name,
    );
    AuditLogRecord::masked(AuditLogRecord {
        id: new_id(),
        updated_at: clock.now(),
        changed_by,
        changed_by_api_key: entry.caller.api_key.clone(),
        action: entry.action,
        table_name: entry.table_name,
        object_id: entry.object_id.to_owned(),
        before_value: entry
            .before_value
            .map(|text| JsonValue::String(text.to_owned())),
        updated_values: entry
            .after_value
            .map(|text| JsonValue::String(text.to_owned())),
    })
}
