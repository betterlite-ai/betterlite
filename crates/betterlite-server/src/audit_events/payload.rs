use betterlite_logging::azure_sentinel_logging::audit::StandardAuditLogPayload;
use secrecy::ExposeSecret as _;

use super::record::{AuditLogRecord, payload_text, timestamp_text};

pub type AuditLogPayload = StandardAuditLogPayload;

#[must_use]
pub fn build_audit_log_payload(record: &AuditLogRecord) -> AuditLogPayload {
    AuditLogPayload {
        id: record.id.clone(),
        updated_at: timestamp_text(record.updated_at, "+00:00"),
        changed_by: record.changed_by.clone().unwrap_or_default(),
        hashed_changed_by_api_key: record
            .changed_by_api_key
            .as_ref()
            .map(|api_key| api_key.expose_secret().to_owned())
            .unwrap_or_default(),
        action: record.action.as_str().to_owned(),
        table_name: record.table_name.as_str().to_owned(),
        object_id: record.object_id.clone(),
        before_value: record.before_value.as_ref().map(payload_text),
        updated_values: record.updated_values.as_ref().map(payload_text),
    }
}
