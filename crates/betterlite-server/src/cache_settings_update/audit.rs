use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::sections::settings::JsonValue;
use secrecy::SecretString;

use crate::audit_events::callbacks::{AuditCallbackFuture, AuditTaskSpawner};
use crate::audit_events::enums::{AuditAction, AuditTable};
use crate::audit_events::record::AuditLogRecord;
use crate::audit_events::sink::AuditEventSink;

pub const CACHE_CONFIG_OBJECT_ID: &str = "cache_config";
pub const DEFAULT_USER_ID: &str = "default_user_id";

#[derive(Clone, Debug)]
pub struct CacheSettingsAuditEntry {
    pub action: AuditAction,
    pub litellm_changed_by: Option<String>,
    pub user_id: Option<String>,
    pub api_key: Option<SecretString>,
    pub before_value: String,
    pub updated_values: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CacheSettingsAuditError {
    #[error("{message}")]
    Write { message: String },
}

#[async_trait]
pub trait CacheSettingsAudit: Send + Sync {
    fn enabled(&self) -> bool;

    async fn write(&self, entry: CacheSettingsAuditEntry) -> Result<(), CacheSettingsAuditError>;
}

pub struct SinkCacheSettingsAudit {
    pub sink: Arc<AuditEventSink>,
}

// The POST route's audit row goes straight to B2330's update path: _redact_settings already removed the values,
// so the record is not masked a second time and no changed-by metadata opt-in applies.
#[async_trait]
impl CacheSettingsAudit for SinkCacheSettingsAudit {
    fn enabled(&self) -> bool {
        self.sink.settings.is_audit_logging_enabled(None)
    }

    async fn write(&self, entry: CacheSettingsAuditEntry) -> Result<(), CacheSettingsAuditError> {
        let record = AuditLogRecord {
            id: (self.sink.new_id)(),
            updated_at: self.sink.clock.now(),
            changed_by: Some(changed_by(
                entry.litellm_changed_by.as_deref(),
                entry.user_id.as_deref(),
            )),
            changed_by_api_key: entry.api_key,
            action: entry.action,
            table_name: AuditTable::CacheConfig,
            object_id: CACHE_CONFIG_OBJECT_ID.to_owned(),
            before_value: Some(JsonValue::String(entry.before_value)),
            updated_values: Some(JsonValue::String(entry.updated_values)),
        };
        self.sink.create_audit_log_for_update(record).await;
        Ok(())
    }
}

fn changed_by(litellm_changed_by: Option<&str>, user_id: Option<&str>) -> String {
    let header = litellm_changed_by.filter(|value| !value.is_empty());
    let user = user_id.filter(|value| !value.is_empty());
    header
        .or(user)
        .map_or_else(|| DEFAULT_USER_ID.to_owned(), str::to_owned)
}

pub fn spawn_cache_settings_audit(
    audit: Arc<dyn CacheSettingsAudit>,
    spawner: &dyn AuditTaskSpawner,
    entry: CacheSettingsAuditEntry,
) {
    if !audit.enabled() {
        return;
    }
    let task: AuditCallbackFuture = Box::pin(async move {
        if let Err(error) = audit.write(entry).await {
            tracing::warn!("Failed to write cache-settings audit log: {error}");
        }
    });
    spawner.spawn(task);
}

mod tests;
