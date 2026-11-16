use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::sections::settings::JsonValue;
use betterlite_db::auxiliary_repository::repository::TableError;
use chrono::{DateTime, FixedOffset, Utc};

use super::callbacks::AuditCallbackDispatcher;
use super::enums::{AuditAction, AuditTable};
use super::identity::{CallerIdentity, get_audit_log_changed_by};
use super::payload::build_audit_log_payload;
use super::record::AuditLogRecord;
use super::settings::AuditSettings;

#[derive(Debug, thiserror::Error)]
pub enum AuditRepositoryError {
    #[error("no database connection")]
    Unavailable,
    #[error(transparent)]
    Table(#[from] TableError),
}

#[async_trait]
pub trait AuditLogRepository: Send + Sync {
    fn available(&self) -> bool;

    async fn insert_audit_log(&self, record: &AuditLogRecord) -> Result<(), AuditRepositoryError>;
}

pub trait AuditClock: Send + Sync {
    fn now(&self) -> DateTime<FixedOffset>;
}

#[derive(Debug, Default)]
pub struct SystemAuditClock;

impl AuditClock for SystemAuditClock {
    fn now(&self) -> DateTime<FixedOffset> {
        Utc::now().fixed_offset()
    }
}

pub struct AuditEventSink {
    pub settings: Arc<dyn AuditSettings>,
    pub callbacks: Arc<AuditCallbackDispatcher>,
    pub repository: Arc<dyn AuditLogRepository>,
    pub clock: Arc<dyn AuditClock>,
    pub new_id: Arc<dyn Fn() -> String + Send + Sync>,
}

pub struct ObjectAuditLog<'entry> {
    pub object_id: &'entry str,
    pub action: AuditAction,
    pub litellm_changed_by: Option<&'entry str>,
    pub caller: &'entry CallerIdentity,
    pub litellm_proxy_admin_name: Option<&'entry str>,
    pub table_name: AuditTable,
    pub before_value: Option<&'entry str>,
    pub after_value: Option<&'entry str>,
}

impl AuditEventSink {
    pub async fn create_object_audit_log(&self, entry: ObjectAuditLog<'_>) {
        if !self.settings.is_audit_logging_enabled(None) {
            return;
        }
        let changed_by = get_audit_log_changed_by(
            entry.litellm_changed_by,
            entry.caller,
            entry.litellm_proxy_admin_name,
        );
        let record = AuditLogRecord::masked(AuditLogRecord {
            id: (self.new_id)(),
            updated_at: self.clock.now(),
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
        });
        match record {
            Ok(masked) => self.create_audit_log_for_update(masked).await,
            Err(error) => tracing::error!("Failed Creating audit log {error}"),
        }
    }

    pub async fn create_audit_log_for_update(&self, record: AuditLogRecord) {
        if !self.settings.is_audit_logging_enabled(None) {
            return;
        }
        tracing::debug!("creating audit log for {record:?}");
        let payload = build_audit_log_payload(&record);
        self.callbacks.dispatch(
            self.settings.audit_log_callbacks(),
            self.settings.s3_audit_callback_params(),
            &payload,
        );
        if !self.repository.available() {
            tracing::error!("prisma_client is None, cannot write audit log to DB");
            return;
        }
        if let Err(error) = self.repository.insert_audit_log(&record).await {
            tracing::error!("Failed Creating audit log {error}");
        }
    }
}
