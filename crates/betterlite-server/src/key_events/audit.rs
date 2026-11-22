use std::sync::Arc;

use betterlite_config::sections::settings::JsonValue;
use secrecy::SecretString;

use crate::audit_events::callbacks::{AuditCallbackFuture, AuditTaskSpawner};
use crate::audit_events::enums::{AuditAction, AuditTable};
use crate::audit_events::identity::{CallerIdentity, get_audit_log_changed_by};
use crate::audit_events::record::{AuditLogRecord, AuditRecordError};
use crate::audit_events::settings::AuditSettings;
use crate::audit_events::sink::{AuditClock, AuditEventSink};

// where a prepared row goes; the hook schedules it and never awaits the write, and the seam is the
// submission point the oracle compares before B2330's premium gate inside the sink
pub trait KeyAuditScheduler: Send + Sync {
    fn schedule(&self, record: AuditLogRecord);
}

// create_audit_log_for_update on the proxy's task tracker
pub struct SinkScheduler {
    pub sink: Arc<AuditEventSink>,
    pub spawner: Arc<dyn AuditTaskSpawner>,
}

impl KeyAuditScheduler for SinkScheduler {
    fn schedule(&self, record: AuditLogRecord) {
        let sink = Arc::clone(&self.sink);
        let task: AuditCallbackFuture =
            Box::pin(async move { sink.create_audit_log_for_update(record).await });
        self.spawner.spawn(task);
    }
}

pub struct KeyEventAudit {
    pub settings: Arc<dyn AuditSettings>,
    pub clock: Arc<dyn AuditClock>,
    pub new_id: Arc<dyn Fn() -> String + Send + Sync>,
    pub scheduler: Arc<dyn KeyAuditScheduler>,
    pub litellm_proxy_admin_name: Option<String>,
}

pub struct KeyAuditEntry<'entry> {
    pub action: AuditAction,
    pub object_id: &'entry str,
    pub changed_by_api_key: Option<&'entry SecretString>,
    pub before_value: Option<JsonValue>,
    pub updated_values: Option<JsonValue>,
    pub litellm_changed_by: Option<&'entry str>,
    pub caller: &'entry CallerIdentity,
}

impl KeyEventAudit {
    // LiteLLM_AuditLogs(...): the constructor itself masks, so the returned row is what a sink sees
    pub fn record(&self, entry: KeyAuditEntry<'_>) -> Result<AuditLogRecord, AuditRecordError> {
        let changed_by = get_audit_log_changed_by(
            entry.litellm_changed_by,
            entry.caller,
            self.litellm_proxy_admin_name.as_deref(),
        );
        AuditLogRecord::masked(AuditLogRecord {
            id: (self.new_id)(),
            updated_at: self.clock.now(),
            changed_by,
            changed_by_api_key: entry.changed_by_api_key.cloned(),
            action: entry.action,
            table_name: AuditTable::Key,
            object_id: entry.object_id.to_owned(),
            before_value: entry.before_value,
            updated_values: entry.updated_values,
        })
    }
}
