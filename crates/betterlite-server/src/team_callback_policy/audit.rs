use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::sections::settings::JsonValue;
use betterlite_models::decisions::Json;
use serde::Serialize;

use crate::audit_events::callbacks::{AuditCallbackFuture, AuditTaskSpawner};
use crate::audit_events::enums::{AuditAction, AuditTable};
use crate::audit_events::identity::CallerIdentity;
use crate::audit_events::record::AuditLogRecord;
use crate::audit_events::settings::AuditSettings;
use crate::audit_events::sink::{AuditClock, AuditEventSink, AuditRepositoryError};
use crate::team_callback_policy::error::TeamCallbackError;
use crate::team_callback_policy::redaction::redact_callback_secrets;

// where the scheduled row goes; the sink logs its own database failures, a writer that can fail reports them here
#[async_trait]
pub trait TeamAuditWriter: Send + Sync {
    async fn write(&self, record: AuditLogRecord) -> Result<(), AuditRepositoryError>;
}

#[async_trait]
impl TeamAuditWriter for AuditEventSink {
    async fn write(&self, record: AuditLogRecord) -> Result<(), AuditRepositoryError> {
        self.create_audit_log_for_update(record).await;
        Ok(())
    }
}

pub struct TeamCallbackAudit {
    pub settings: Arc<dyn AuditSettings>,
    pub clock: Arc<dyn AuditClock>,
    pub new_id: Arc<dyn Fn() -> String + Send + Sync>,
    pub writer: Arc<dyn TeamAuditWriter>,
    pub spawner: Arc<dyn AuditTaskSpawner>,
    pub litellm_proxy_admin_name: Option<String>,
}

pub struct TeamCallbackChange<'change> {
    pub team_id: &'change str,
    pub before_metadata: &'change Json,
    pub after_metadata: &'change Json,
    pub caller: &'change CallerIdentity,
    pub litellm_changed_by: Option<&'change str>,
}

#[derive(Serialize)]
struct MetadataSnapshot {
    metadata: Json,
}

impl TeamCallbackAudit {
    // _emit_team_callback_audit_log: the row is scheduled, not awaited, so a failed write never undoes the committed change
    pub fn emit(&self, change: &TeamCallbackChange<'_>) -> Result<(), TeamCallbackError> {
        if !self.settings.is_audit_logging_enabled(None) {
            return Ok(());
        }
        let before_value = snapshot(change.before_metadata)?;
        let updated_values = snapshot(change.after_metadata)?;
        // these routes take a non-empty litellm-changed-by header without the metadata opt-in other audit rows need
        let changed_by = change
            .litellm_changed_by
            .filter(|name| !name.is_empty())
            .or_else(|| change.caller.user_id.as_deref().filter(|id| !id.is_empty()))
            .or(self.litellm_proxy_admin_name.as_deref())
            .map(str::to_owned);
        let record = AuditLogRecord::masked(AuditLogRecord {
            id: (self.new_id)(),
            updated_at: self.clock.now(),
            changed_by,
            changed_by_api_key: change.caller.api_key.clone(),
            action: AuditAction::Updated,
            table_name: AuditTable::Team,
            object_id: change.team_id.to_owned(),
            before_value: Some(before_value),
            updated_values: Some(updated_values),
        })
        .map_err(|source| TeamCallbackError::AuditRecord { source })?;
        let writer = Arc::clone(&self.writer);
        let task: AuditCallbackFuture = Box::pin(async move {
            if let Err(error) = writer.write(record).await {
                tracing::warn!("Failed to write team-callback audit log: {error}");
            }
        });
        self.spawner.spawn(task);
        Ok(())
    }
}

fn snapshot(metadata: &Json) -> Result<JsonValue, TeamCallbackError> {
    let redacted = MetadataSnapshot {
        metadata: redact_callback_secrets(metadata),
    };
    serde_json::to_string(&redacted)
        .map(JsonValue::String)
        .map_err(|source| TeamCallbackError::Snapshot { source })
}
