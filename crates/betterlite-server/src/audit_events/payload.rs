use betterlite_logging::azure_sentinel_logging::audit::StandardAuditLogPayload;

use super::record::{AuditLogRecord, isoformat_timestamp, payload_text};

pub type AuditLogPayload = StandardAuditLogPayload;

#[must_use]
pub fn build_audit_log_payload(record: &AuditLogRecord) -> AuditLogPayload {
    AuditLogPayload {
        id: record.id.clone(),
        updated_at: isoformat_timestamp(record.updated_at),
        changed_by: record.changed_by.clone().unwrap_or_default(),
        hashed_changed_by_api_key: record.changed_by_api_key.clone().unwrap_or_default(),
        action: record.action.as_str().to_owned(),
        table_name: record.table_name.as_str().to_owned(),
        object_id: record.object_id.clone(),
        before_value: record.before_value.as_ref().map(payload_text),
        updated_values: record.updated_values.as_ref().map(payload_text),
    }
}
