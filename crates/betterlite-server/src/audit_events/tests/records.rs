#![cfg(test)]

use betterlite_config::sections::settings::JsonValue;
use chrono::DateTime;
use serde_json::json;

use crate::audit_events::record::{
    AuditRecordError, MAX_AUDIT_RECORD_BYTES, MAX_AUDIT_RECORD_DEPTH, decode_audit_record,
    json_timestamp,
};

const RECORD: &str = r#"{"id": "audit-1", "updated_at": "2026-01-02T03:04:05.123456+00:00", "action": "updated", "table_name": "LiteLLM_TeamTable", "object_id": "team-1"}"#;

const ACTIONS: [&str; 7] = [
    "created",
    "updated",
    "deleted",
    "blocked",
    "unblocked",
    "rotated",
    "kill_switch_fired",
];

const TABLES: [&str; 12] = [
    "LiteLLM_TeamTable",
    "LiteLLM_UserTable",
    "LiteLLM_VerificationToken",
    "LiteLLM_ProxyModelTable",
    "LiteLLM_ManagedFileTable",
    "LiteLLM_ToolTable",
    "LiteLLM_CacheConfig",
    "LiteLLM_ConfigOverrides",
    "LiteLLM_Config",
    "LiteLLM_SSOConfig",
    "LiteLLM_UISettings",
    "LiteLLM_AgentsTable",
];

fn record_text(action: &str, table_name: &str) -> String {
    json!({
        "id": "audit-1",
        "updated_at": "2026-01-02T03:04:05Z",
        "action": action,
        "table_name": table_name,
        "object_id": "team-1",
    })
    .to_string()
}

#[test]
fn audit_record_defaults_and_enums() {
    let record = decode_audit_record(RECORD.as_bytes()).unwrap();
    assert!(record.changed_by.is_none());
    assert!(record.changed_by_api_key.is_none());
    assert!(record.before_value.is_none());
    assert!(record.updated_values.is_none());
    for action in ACTIONS {
        let text = record_text(action, "LiteLLM_TeamTable");
        let decoded = decode_audit_record(text.as_bytes()).unwrap();
        assert_eq!(decoded.action.as_str(), action);
    }
    for table_name in TABLES {
        let text = record_text("updated", table_name);
        let decoded = decode_audit_record(text.as_bytes()).unwrap();
        assert_eq!(decoded.table_name.as_str(), table_name);
    }
}

#[test]
fn audit_changed_by_cast() {
    for (value, expected) in [
        (json!("text"), Some("text")),
        (json!(42), Some("42")),
        (json!(1.5), Some("1.5")),
        (json!(true), Some("True")),
        (json!(false), Some("False")),
        (json!(null), None),
        (json!({"team": 1}), Some(r#"{"team":1}"#)),
        (json!([1, 2]), Some("[1,2]")),
    ] {
        let mut given: JsonValue = serde_json::from_str(RECORD).unwrap();
        given["changed_by"] = value;
        let decoded = decode_audit_record(given.to_string().as_bytes()).unwrap();
        assert_eq!(decoded.changed_by.as_deref(), expected);
    }
}

#[test]
fn audit_changed_by_api_key_field() {
    let mut given: JsonValue = serde_json::from_str(RECORD).unwrap();
    given["changed_by_api_key"] = json!("sk-key");
    let decoded = decode_audit_record(given.to_string().as_bytes()).unwrap();
    assert_eq!(decoded.changed_by_api_key.as_deref(), Some("sk-key"));
    given["changed_by_api_key"] = json!(7);
    let error = decode_audit_record(given.to_string().as_bytes()).unwrap_err();
    assert!(matches!(
        error,
        AuditRecordError::FieldType {
            field: "changed_by_api_key"
        }
    ));
}

#[test]
fn audit_json_timestamp_formats_the_zone() {
    let utc = DateTime::parse_from_rfc3339("2026-01-02T03:04:05+00:00").unwrap();
    assert_eq!(json_timestamp(utc), "2026-01-02T03:04:05Z");
    let micros = DateTime::parse_from_rfc3339("2026-01-02T03:04:05.123456Z").unwrap();
    assert_eq!(json_timestamp(micros), "2026-01-02T03:04:05.123456Z");
    let offset = DateTime::parse_from_rfc3339("2026-01-02T05:04:05.100000+02:00").unwrap();
    assert_eq!(json_timestamp(offset), "2026-01-02T05:04:05.100000+02:00");
}

fn masked_record(before_value: JsonValue, updated_values: JsonValue) -> JsonValue {
    let text = json!({
        "id": "audit-1",
        "updated_at": "2026-01-02T03:04:05Z",
        "action": "updated",
        "table_name": "LiteLLM_TeamTable",
        "object_id": "team-1",
        "before_value": before_value.to_string(),
        "updated_values": updated_values.to_string(),
    })
    .to_string();
    let record = decode_audit_record(text.as_bytes()).unwrap();
    json!({"before": record.before_value, "updated": record.updated_values})
}

#[test]
fn audit_json_masking() {
    let masked = masked_record(
        json!({
            "api_key": "sk-1234567890",
            "name": "team",
            "input_cost_per_token": "0.0001",
            "nested": {"key": "abcdefgh"},
        }),
        json!({"auth_token": "abcdefghijkl"}),
    );
    assert_eq!(
        masked,
        json!({
            "before": {
                "api_key": "sk-1*****7890",
                "name": "team",
                "input_cost_per_token": "0.0001",
                "nested": {"key": "********"},
            },
            "updated": {"auth_token": "abcd****ijkl"},
        })
    );
    let list = masked_record(json!([{"api_key": "sk-1234567890"}]), json!(null));
    assert_eq!(
        list,
        json!({"before": [{"api_key": "sk-1*****7890"}], "updated": null})
    );
    let nulls = masked_record(json!({"name": null, "key": null}), json!(null));
    assert_eq!(
        nulls,
        json!({"before": {"name": "None", "key": ""}, "updated": null})
    );
}

#[test]
fn audit_invalid_record() {
    assert!(matches!(
        decode_audit_record(b"null"),
        Err(AuditRecordError::NotAnObject)
    ));
    assert!(matches!(
        decode_audit_record(b"{"),
        Err(AuditRecordError::Json(_))
    ));
    assert!(matches!(
        decode_audit_record(br#"{"updated_at":"2026-01-02T03:04:05Z","action":"updated","table_name":"LiteLLM_TeamTable","object_id":"t"}"#),
        Err(AuditRecordError::MissingField { field: "id" })
    ));
    assert!(matches!(
        decode_audit_record(br#"{"id":1,"updated_at":"2026-01-02T03:04:05Z","action":"updated","table_name":"LiteLLM_TeamTable","object_id":"t"}"#),
        Err(AuditRecordError::FieldType { field: "id" })
    ));
    assert!(matches!(
        decode_audit_record(br#"{"id":"a","updated_at":"2026-01-02T03:04:05Z","action":"changed","table_name":"LiteLLM_TeamTable","object_id":"t"}"#),
        Err(AuditRecordError::UnknownValue { field: "action" })
    ));
    assert!(matches!(
        decode_audit_record(br#"{"id":"a","updated_at":"2026-01-02T03:04:05Z","action":"updated","table_name":"LiteLLM_KeysTable","object_id":"t"}"#),
        Err(AuditRecordError::UnknownValue {
            field: "table_name"
        })
    ));
    assert!(matches!(
        decode_audit_record(br#"{"id":"a","updated_at":"2026-01-02T03:04:05","action":"updated","table_name":"LiteLLM_TeamTable","object_id":"t"}"#),
        Err(AuditRecordError::Timestamp { field: "updated_at", .. })
    ));
    assert!(matches!(
        decode_audit_record(br#"{"id":"a","updated_at":"2026-01-02T03:04:05Z","action":"updated","table_name":"LiteLLM_TeamTable","object_id":"t","before_value":"{oops"}"#),
        Err(AuditRecordError::JsonField {
            field: "before_value",
            ..
        })
    ));
    assert!(matches!(
        decode_audit_record(br#"{"id":"a","updated_at":"2026-01-02T03:04:05Z","action":"updated","table_name":"LiteLLM_TeamTable","object_id":"t","extra":1}"#),
        Err(AuditRecordError::UnknownField { .. })
    ));
}

#[test]
fn audit_invalid_record_bounds() {
    let deep = format!(
        "{}{}{}",
        "{\"x\":".repeat(MAX_AUDIT_RECORD_DEPTH + 1),
        "null",
        "}".repeat(MAX_AUDIT_RECORD_DEPTH + 1)
    );
    assert!(matches!(
        decode_audit_record(deep.as_bytes()),
        Err(AuditRecordError::TooDeep)
    ));
    let nested = format!(
        "{}{}{}",
        "{\"k\":".repeat(MAX_AUDIT_RECORD_DEPTH - 1),
        "null",
        "}".repeat(MAX_AUDIT_RECORD_DEPTH - 1)
    );
    let at_depth = format!(
        "{{\"id\":\"a\",\"updated_at\":\"2026-01-02T03:04:05Z\",\"action\":\"updated\",\"table_name\":\"LiteLLM_TeamTable\",\"object_id\":\"t\",\"before_value\":{nested}}}"
    );
    assert!(decode_audit_record(at_depth.as_bytes()).is_ok());
    let mut at_size = record_text("updated", "LiteLLM_TeamTable").into_bytes();
    at_size.resize(MAX_AUDIT_RECORD_BYTES, b' ');
    assert!(decode_audit_record(&at_size).is_ok());
    let oversized = vec![b' '; MAX_AUDIT_RECORD_BYTES + 1];
    assert!(matches!(
        decode_audit_record(&oversized),
        Err(AuditRecordError::TooLarge { .. })
    ));
    assert!(matches!(
        decode_audit_record(&[0xff]),
        Err(AuditRecordError::InvalidUtf8(_))
    ));
}
