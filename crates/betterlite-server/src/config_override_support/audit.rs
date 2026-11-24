use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::Object as JsonMap;
use chrono::{DateTime, FixedOffset};
use secrecy::SecretString;

use crate::audit::service::{AuditService, AuditSubmission};
use crate::audit_events::enums::{AuditAction, AuditTable};
use crate::audit_events::record::AuditLogRecord;

pub const AUDIT_REDACTED: &str = "***REDACTED***";

pub struct OverrideAuditEntry<'entry> {
    pub object_id: &'entry str,
    pub action: AuditAction,
    pub before_config: Option<&'entry JsonMap>,
    pub after_config: Option<&'entry JsonMap>,
    pub litellm_changed_by: Option<&'entry str>,
    pub user_id: Option<&'entry str>,
    pub admin_name: Option<&'entry str>,
    pub api_key: Option<SecretString>,
}

// The record parts the caller supplies; B065's service owns the queue, so this port keeps it replaceable.
pub trait ConfigOverrideAudit: Send + Sync {
    fn new_record_id(&self) -> String;

    fn record_time(&self) -> DateTime<FixedOffset>;

    fn submit(&self, record: AuditLogRecord) -> AuditSubmission;
}

impl ConfigOverrideAudit for AuditService {
    fn new_record_id(&self) -> String {
        (self.new_id)()
    }

    fn record_time(&self) -> DateTime<FixedOffset> {
        self.clock.now()
    }

    fn submit(&self, record: AuditLogRecord) -> AuditSubmission {
        self.try_submit(record)
    }
}

// Keys stay so an auditor sees which fields changed; every value is replaced.
#[must_use]
pub fn redact_config(config: Option<&JsonMap>) -> JsonMap {
    config
        .filter(|present| !present.is_empty())
        .map(|present| {
            present
                .keys()
                .map(|key| (key.clone(), JsonValue::String(AUDIT_REDACTED.to_owned())))
                .collect()
        })
        .unwrap_or_default()
}

#[must_use]
pub fn submit_override_audit_log(
    audit: &dyn ConfigOverrideAudit,
    entry: OverrideAuditEntry<'_>,
) -> AuditSubmission {
    let record = AuditLogRecord {
        id: audit.new_record_id(),
        updated_at: audit.record_time(),
        changed_by: changed_by(entry.litellm_changed_by, entry.user_id, entry.admin_name),
        changed_by_api_key: entry.api_key,
        action: entry.action,
        table_name: AuditTable::ConfigOverrides,
        object_id: entry.object_id.to_owned(),
        before_value: Some(JsonValue::String(config_snapshot(entry.before_config))),
        updated_values: Some(JsonValue::String(config_snapshot(entry.after_config))),
    };
    let submission = audit.submit(record);
    if matches!(submission, AuditSubmission::Rejected(_)) {
        tracing::warn!("Failed to write config override audit log");
    }
    submission
}

// The trusted admin attribution this helper uses, unlike B065's metadata-gated litellm-changed-by.
fn changed_by(
    litellm_changed_by: Option<&str>,
    user_id: Option<&str>,
    admin_name: Option<&str>,
) -> Option<String> {
    [litellm_changed_by, user_id, admin_name]
        .into_iter()
        .flatten()
        .find(|value| !value.is_empty())
        .map(str::to_owned)
}

fn config_snapshot(config: Option<&JsonMap>) -> String {
    let redacted = redact_config(config);
    let mut snapshot = JsonMap::new();
    snapshot.insert("config".to_owned(), JsonValue::Object(redacted));
    JsonValue::Object(snapshot).to_string()
}
