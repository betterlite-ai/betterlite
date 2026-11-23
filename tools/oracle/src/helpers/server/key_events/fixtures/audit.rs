use std::sync::Arc;

use betterlite_server::audit_events::record::{AuditLogRecord, timestamp_text};
use betterlite_server::key_events::audit::KeyAuditScheduler;
use secrecy::ExposeSecret;
use serde_json::{Value, json};

use super::board::Board;

pub(crate) struct RecordingScheduler {
    pub(crate) board: Arc<Board>,
}

impl KeyAuditScheduler for RecordingScheduler {
    fn schedule(&self, record: AuditLogRecord) {
        self.board.record(json!(["audit", record_view(&record)]));
    }
}

fn record_view(record: &AuditLogRecord) -> Value {
    json!({
        "id": record.id,
        "updated_at": timestamp_text(record.updated_at, "Z"),
        "changed_by": record.changed_by,
        "changed_by_api_key": record.changed_by_api_key.as_ref().map(ExposeSecret::expose_secret),
        "action": record.action.as_str(),
        "table_name": record.table_name.as_str(),
        "object_id": record.object_id,
        "before_value": record.before_value,
        "updated_values": record.updated_values,
    })
}
