#![cfg(test)]

use serde_json::json;

use crate::audit_events::record::{AuditRecordError, decode_audit_record};

const NULL: &[u8] = include_bytes!("../../../fuzz/corpus/audit_record/null");
const MALFORMED: &[u8] = include_bytes!("../../../fuzz/corpus/audit_record/malformed");
const NESTED_SECRET: &[u8] = include_bytes!("../../../fuzz/corpus/audit_record/nested-secret");
const UNKNOWN_ENUM: &[u8] = include_bytes!("../../../fuzz/corpus/audit_record/unknown-enum");

#[test]
fn replays_the_audit_record_corpus() {
    assert!(matches!(
        decode_audit_record(NULL),
        Err(AuditRecordError::NotAnObject)
    ));
    assert!(matches!(
        decode_audit_record(MALFORMED),
        Err(AuditRecordError::Json(_))
    ));
    assert!(matches!(
        decode_audit_record(UNKNOWN_ENUM),
        Err(AuditRecordError::UnknownValue { field: "action" })
    ));
    let record = decode_audit_record(NESTED_SECRET).unwrap();
    assert_eq!(record.changed_by.as_deref(), Some("42"));
    assert_eq!(
        record.before_value,
        Some(json!({"api_key": "sk-1*****7890", "nested": {"key": "********"}}))
    );
}
