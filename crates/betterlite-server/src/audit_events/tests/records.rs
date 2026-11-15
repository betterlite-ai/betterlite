#![cfg(test)]

use betterlite_config::sections::settings::JsonValue;
use betterlite_logging::private_json::error::PrivateJsonError;
use betterlite_logging::private_json::safe_json::{MAX_JSON_BYTES, MAX_JSON_DEPTH};
use chrono::DateTime;
use rstest::rstest;
use secrecy::ExposeSecret;

use super::support::parsed;
use crate::audit_events::record::{AuditRecordError, decode_audit_record, timestamp_text};

const RECORD: &str = r#"{"id": "audit-1", "updated_at": "2026-01-02T03:04:05.123456+00:00", "action": "updated", "table_name": "LiteLLM_TeamTable", "object_id": "team-1"}"#;

fn record_text(action: &str, table_name: &str) -> String {
    format!(
        r#"{{"id": "audit-1", "updated_at": "2026-01-02T03:04:05Z", "action": "{action}", "table_name": "{table_name}", "object_id": "team-1"}}"#
    )
}

#[rstest]
#[case::action_created("created", "LiteLLM_TeamTable")]
#[case::action_updated("updated", "LiteLLM_TeamTable")]
#[case::action_deleted("deleted", "LiteLLM_TeamTable")]
#[case::action_blocked("blocked", "LiteLLM_TeamTable")]
#[case::action_unblocked("unblocked", "LiteLLM_TeamTable")]
#[case::action_rotated("rotated", "LiteLLM_TeamTable")]
#[case::action_kill_switch_fired("kill_switch_fired", "LiteLLM_TeamTable")]
#[case::table_user("updated", "LiteLLM_UserTable")]
#[case::table_key("updated", "LiteLLM_VerificationToken")]
#[case::table_proxy_model("updated", "LiteLLM_ProxyModelTable")]
#[case::table_managed_file("updated", "LiteLLM_ManagedFileTable")]
#[case::table_tool("updated", "LiteLLM_ToolTable")]
#[case::table_cache_config("updated", "LiteLLM_CacheConfig")]
#[case::table_config_overrides("updated", "LiteLLM_ConfigOverrides")]
#[case::table_config("updated", "LiteLLM_Config")]
#[case::table_sso_config("updated", "LiteLLM_SSOConfig")]
#[case::table_ui_settings("updated", "LiteLLM_UISettings")]
#[case::table_agent("updated", "LiteLLM_AgentsTable")]
fn audit_record_defaults_and_enums(#[case] action: &str, #[case] table_name: &str) {
    let text = record_text(action, table_name);
    let record = decode_audit_record(text.as_bytes()).unwrap();
    assert_eq!(record.id, "audit-1");
    assert_eq!(record.object_id, "team-1");
    assert_eq!(
        record.updated_at,
        DateTime::parse_from_rfc3339("2026-01-02T03:04:05Z").unwrap()
    );
    assert_eq!(record.action.as_str(), action);
    assert_eq!(record.table_name.as_str(), table_name);
    assert!(record.changed_by.is_none());
    assert!(record.changed_by_api_key.is_none());
    assert!(record.before_value.is_none());
    assert!(record.updated_values.is_none());
}

#[rstest]
#[case::string(r#""text""#, Some("text"))]
#[case::integer("42", Some("42"))]
#[case::float("1.5", Some("1.5"))]
#[case::true_bool("true", Some("True"))]
#[case::false_bool("false", Some("False"))]
#[case::null("null", None)]
#[case::object(r#"{"team": 1}"#, Some(r#"{"team":1}"#))]
#[case::array("[1, 2]", Some("[1,2]"))]
fn audit_changed_by_cast(#[case] value: &str, #[case] expected: Option<&str>) {
    let mut given = parsed(RECORD);
    given["changed_by"] = parsed(value);
    let decoded = decode_audit_record(given.to_string().as_bytes()).unwrap();
    assert_eq!(decoded.changed_by.as_deref(), expected);
}

#[test]
fn audit_changed_by_api_key_field() {
    let mut given = parsed(RECORD);
    given["changed_by_api_key"] = JsonValue::String("sk-key".to_owned());
    let decoded = decode_audit_record(given.to_string().as_bytes()).unwrap();
    assert_eq!(
        decoded
            .changed_by_api_key
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("sk-key")
    );
    given["changed_by_api_key"] = parsed("7");
    let error = decode_audit_record(given.to_string().as_bytes()).unwrap_err();
    assert!(matches!(
        error,
        AuditRecordError::FieldType {
            field: "changed_by_api_key"
        }
    ));
}

#[test]
fn audit_timestamp_text_formats_the_zone() {
    let utc = DateTime::parse_from_rfc3339("2026-01-02T03:04:05+00:00").unwrap();
    assert_eq!(timestamp_text(utc, "Z"), "2026-01-02T03:04:05Z");
    assert_eq!(timestamp_text(utc, "+00:00"), "2026-01-02T03:04:05+00:00");
    let micros = DateTime::parse_from_rfc3339("2026-01-02T03:04:05.123456Z").unwrap();
    assert_eq!(timestamp_text(micros, "Z"), "2026-01-02T03:04:05.123456Z");
    let offset = DateTime::parse_from_rfc3339("2026-01-02T05:04:05.100000+02:00").unwrap();
    assert_eq!(
        timestamp_text(offset, "Z"),
        "2026-01-02T05:04:05.100000+02:00"
    );
}

fn masked_fields(before_text: &str, updated_text: &str) -> (Option<JsonValue>, Option<JsonValue>) {
    let mut given = parsed(RECORD);
    given["before_value"] = JsonValue::String(before_text.to_owned());
    given["updated_values"] = JsonValue::String(updated_text.to_owned());
    let decoded = decode_audit_record(given.to_string().as_bytes()).unwrap();
    (decoded.before_value, decoded.updated_values)
}

#[test]
fn audit_json_masking() {
    let (before, updated) = masked_fields(
        r#"{"api_key": "sk-1234567890", "name": "team", "input_cost_per_token": "0.0001", "nested": {"key": "abcdefgh"}}"#,
        r#"{"auth_token": "abcdefghijkl"}"#,
    );
    assert_eq!(
        before,
        Some(parsed(
            r#"{"api_key": "sk-1*****7890", "name": "team", "input_cost_per_token": "0.0001", "nested": {"key": "********"}}"#
        ))
    );
    assert_eq!(updated, Some(parsed(r#"{"auth_token": "abcd****ijkl"}"#)));
    let (list, null_text) = masked_fields(r#"[{"api_key": "sk-1234567890"}]"#, "null");
    assert_eq!(list, Some(parsed(r#"[{"api_key": "sk-1*****7890"}]"#)));
    assert_eq!(null_text, None);
    let (quoted_null, _) = masked_fields(r#""null""#, "null");
    assert_eq!(quoted_null, Some(JsonValue::String("null".to_owned())));
    let (nulls, _) = masked_fields(r#"{"name": null, "key": null}"#, "null");
    assert_eq!(nulls, Some(parsed(r#"{"name": "None", "key": ""}"#)));
}

#[test]
fn audit_double_encoded_json_masking() {
    let (before, updated) = masked_fields(
        r#""{\"api_key\": \"sk-1234567890\", \"name\": \"team\"}""#,
        r#""{\"key\": \"abcdefghijkl\"}""#,
    );
    assert_eq!(
        before,
        Some(parsed(r#"{"api_key": "sk-1*****7890", "name": "team"}"#))
    );
    assert_eq!(updated, Some(parsed(r#"{"key": "abcd****ijkl"}"#)));
    let (text, _) = masked_fields(r#""\"admin\"""#, "null");
    assert_eq!(text, Some(JsonValue::String("admin".to_owned())));
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
}

#[test]
fn audit_invalid_record_enums_and_time() {
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
}

#[test]
fn audit_invalid_record_payloads() {
    assert!(matches!(
        decode_audit_record(br#"{"id":"a","updated_at":"2026-01-02T03:04:05Z","action":"updated","table_name":"LiteLLM_TeamTable","object_id":"t","before_value":"{oops"}"#),
        Err(AuditRecordError::JsonField {
            field: "before_value",
            ..
        })
    ));
    assert!(matches!(
        decode_audit_record(br#"{"id":"a","updated_at":"2026-01-02T03:04:05Z","action":"updated","table_name":"LiteLLM_TeamTable","object_id":"t","before_value":"\"{oops\""}"#),
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
        "{\"x\":".repeat(MAX_JSON_DEPTH + 1),
        "null",
        "}".repeat(MAX_JSON_DEPTH + 1)
    );
    assert!(matches!(
        decode_audit_record(deep.as_bytes()),
        Err(AuditRecordError::Json(PrivateJsonError::TooDeep(_)))
    ));
    let nested = format!(
        "{}{}{}",
        "{\"k\":".repeat(MAX_JSON_DEPTH - 1),
        "null",
        "}".repeat(MAX_JSON_DEPTH - 1)
    );
    let at_depth = format!(
        "{{\"id\":\"a\",\"updated_at\":\"2026-01-02T03:04:05Z\",\"action\":\"updated\",\"table_name\":\"LiteLLM_TeamTable\",\"object_id\":\"t\",\"before_value\":{nested}}}"
    );
    let depth_record = decode_audit_record(at_depth.as_bytes()).unwrap();
    assert_eq!(depth_record.object_id, "t");
    let mut at_size = record_text("updated", "LiteLLM_TeamTable").into_bytes();
    at_size.resize(MAX_JSON_BYTES, b' ');
    let size_record = decode_audit_record(&at_size).unwrap();
    assert_eq!(size_record.object_id, "team-1");
    let oversized = vec![b' '; MAX_JSON_BYTES + 1];
    assert!(matches!(
        decode_audit_record(&oversized),
        Err(AuditRecordError::Json(PrivateJsonError::TooLarge(..)))
    ));
    assert!(matches!(
        decode_audit_record(&[0xff]),
        Err(AuditRecordError::Json(PrivateJsonError::Decode(_)))
    ));
}
