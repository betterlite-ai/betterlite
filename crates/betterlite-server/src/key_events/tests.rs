#![cfg(test)]

mod support;

use std::sync::Arc;
use std::sync::atomic::Ordering;

use betterlite_config::sections::settings::JsonValue;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::key::{Expiry, LiteLLMVerificationToken};
use chrono::DateTime;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tokio_util::task::TaskTracker;
use tracing::Level;
use tracing::subscriber::set_default;
use tracing_subscriber::layer::SubscriberExt as _;

use self::support::{
    AUDIT, BOTH, Call, DebugLines, NOW, RecordingMailer, STORE, caller, harness, lock,
    sink_fixture, wired,
};
use crate::audit_events::enums::{AuditAction, AuditTable};
use crate::audit_events::identity::CallerIdentity;
use crate::key_event_sinks::email::{CreatedKey, LegacyEmail, Reported};
use crate::key_event_sinks::error::EmailError;
use crate::key_events::effects::{KeyEmailError, KeyEventEmail};
use crate::key_events::error::KeyEventError;
use crate::key_events::types::{
    DeletedEvent, GeneratedEvent, KeyEvent, KeyEventCaller, RotatedEvent, UpdatedEvent,
};
use crate::key_fields::decode::TypedValue;
use crate::key_fields::regenerate::RegenerateKeyRequest;
use crate::key_fields::request::GenerateKeyRequest;
use crate::key_fields::response::GenerateKeyResponse;
use crate::key_fields::shared::hash_token_if_needed;
use crate::key_fields::update::UpdateKeyRequest;

const ROW: &str = r#"{"token": "sk-old-key", "key_alias": "old-alias", "team_id": "team-a"}"#;
const RESPONSE: &str =
    r#"{"key": "sk-new-key", "token_id": "tok-1", "key_alias": "alias-a", "team_id": "team-a"}"#;

fn value(text: &str) -> TypedValue {
    serde_json::from_str(text).expect("a JSON value")
}

fn exposed(secret: &Option<SecretString>) -> Option<&str> {
    secret.as_ref().map(|value| value.expose_secret().as_str())
}

fn data(text: &str) -> JsonData {
    serde_json::from_str(text).expect("JSON data")
}

fn object(text: &str) -> JsonValue {
    serde_json::from_str(text).expect("a JSON object")
}

fn key_row(text: &str) -> LiteLLMVerificationToken {
    serde_json::from_str(text).expect("a stored key row")
}

fn generated(request: &str, response: &str) -> KeyEvent {
    KeyEvent::Generated(GeneratedEvent {
        data: GenerateKeyRequest::read(&value(request)).expect("a generate request"),
        response: GenerateKeyResponse::read(&value(response)).expect("a generate response"),
    })
}

fn updated(request: &str, row: &str, response: &str) -> KeyEvent {
    KeyEvent::Updated(UpdatedEvent {
        data: UpdateKeyRequest::read(&value(request)).expect("an update request"),
        row: key_row(row),
        response: data(response),
    })
}

fn rotated(request: Option<&str>, row: &str, response: &str) -> KeyEvent {
    KeyEvent::Rotated(RotatedEvent {
        data: request
            .map(|text| RegenerateKeyRequest::read(&value(text)).expect("a regenerate request")),
        row: key_row(row),
        response: GenerateKeyResponse::read(&value(response)).expect("a generate response"),
    })
}

fn deleted(text: &str) -> KeyEvent {
    let JsonData::Array(rows) = data(text) else {
        panic!("the scripted rows are an array");
    };
    KeyEvent::Deleted(DeletedEvent {
        request: data(r#"{"keys": ["request-marker"]}"#),
        rows,
        response: data(r#"{"deleted_keys": ["response-marker"]}"#),
    })
}

fn caller_with_metadata() -> KeyEventCaller {
    KeyEventCaller {
        identity: CallerIdentity {
            user_id: Some("user-1".to_owned()),
            api_key: Some(SecretString::from("caller-api-key")),
            metadata: Some(object(r#"{"allow_litellm_changed_by_header": true}"#)),
            team_metadata: None,
        },
        token: Some(SecretString::from("caller-token")),
    }
}

#[rstest]
#[case::absent(r#"{"key_alias": "alias-a"}"#, 0)]
#[case::explicit_null(r#"{"key_alias": "alias-a", "send_invite_email": null}"#, 0)]
#[case::disabled(r#"{"key_alias": "alias-a", "send_invite_email": false}"#, 0)]
#[case::enabled(r#"{"key_alias": "alias-a", "send_invite_email": true}"#, 1)]
#[tokio::test]
async fn generated_email_true_only(#[case] request: &str, #[case] expected: usize) {
    let harness = harness();
    harness
        .dispatch(&generated(request, RESPONSE), &caller())
        .await
        .unwrap();
    assert_eq!(harness.created_emails(), expected, "{request}");
}

#[tokio::test]
async fn generated_email_failure_continues() {
    let harness = wired(BOTH);
    harness.email.fail_created.store(true, Ordering::Relaxed);
    let event = generated(
        r#"{"key_alias": "alias-a", "send_invite_email": true}"#,
        RESPONSE,
    );
    harness.dispatch(&event, &caller()).await.unwrap();
    assert_eq!(
        harness.steps(),
        ["email:created", "audit:tok-1", "write:litellm/alias-a"]
    );
    assert_eq!(harness.records().len(), 1);
    assert!(
        harness.warnings()[0].starts_with("Failed to send key created email"),
        "{:?}",
        harness.warnings()
    );
}

#[tokio::test]
async fn generated_audit_projection() {
    let harness = wired(AUDIT);
    let event = generated(r#"{"key_alias": "alias-a", "team_id": "team-a"}"#, RESPONSE);
    harness.dispatch(&event, &caller()).await.unwrap();
    let records = harness.records();
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(record.id, "audit-1");
    assert_eq!(
        record.updated_at,
        DateTime::parse_from_rfc3339(NOW).unwrap()
    );
    assert_eq!(record.action, AuditAction::Created);
    assert_eq!(record.table_name, AuditTable::Key);
    assert_eq!(record.object_id, "tok-1");
    assert_eq!(record.changed_by.as_deref(), Some("user-1"));
    assert_eq!(exposed(&record.changed_by_api_key), Some("caller-api-key"));
    assert_eq!(record.before_value, None);
    let Some(JsonValue::Object(values)) = &record.updated_values else {
        panic!("the created values are a masked object");
    };
    assert_eq!(values.get("key"), Some(&serde_json::json!("sk-n**-key")));
    assert_eq!(values.get("key_alias"), Some(&serde_json::json!("*******")));
    assert_eq!(values.get("team_id"), Some(&serde_json::json!("team-a")));
    assert_eq!(values.get("spend"), Some(&serde_json::json!(0.0)));
    assert!(values.values().all(|value| !value.is_null()));

    let header = wired(AUDIT);
    let event = generated(r#"{"key_alias": "alias-a"}"#, RESPONSE);
    header
        .dispatch_as(&event, &caller_with_metadata(), Some("proxy-admin"))
        .await
        .unwrap();
    assert_eq!(
        header.records()[0].changed_by.as_deref(),
        Some("proxy-admin")
    );
}

#[tokio::test]
async fn generated_null_token_fallback() {
    let harness = wired(BOTH);
    let event = generated(
        r#"{"key_alias": ""}"#,
        r#"{"key": "sk-new-key", "key_alias": ""}"#,
    );
    harness.dispatch(&event, &caller()).await.unwrap();
    assert_eq!(
        harness.steps(),
        ["audit:", "write:litellm/virtual-key-None"]
    );
    assert_eq!(harness.records()[0].object_id, "");

    let with_token = wired(BOTH);
    let event = generated("{}", r#"{"key": "sk-new-key", "token_id": "tok-2"}"#);
    with_token.dispatch(&event, &caller()).await.unwrap();
    assert_eq!(
        with_token.steps(),
        ["audit:tok-2", "write:litellm/virtual-key-tok-2"]
    );
}

#[tokio::test]
async fn generated_store_failure_warning() {
    let mut harness = wired(BOTH);
    harness.manager.failing.insert("litellm/alias-a".to_owned());
    let event = generated(r#"{"key_alias": "alias-a"}"#, RESPONSE);
    harness.dispatch(&event, &caller()).await.unwrap();
    assert_eq!(harness.steps(), ["audit:tok-1", "write:litellm/alias-a"]);
    assert_eq!(harness.records().len(), 1);
    assert!(
        harness.warnings()[0].starts_with("Failed to store virtual key in secret manager"),
        "{:?}",
        harness.warnings()
    );
}

#[tokio::test]
async fn update_alias_unchanged_no_rename() {
    let harness = wired(BOTH);
    let event = updated(
        r#"{"key": "sk-1", "key_alias": "same"}"#,
        r#"{"token": "sk-1", "key_alias": "same", "team_id": "team-a"}"#,
        "{}",
    );
    harness.dispatch(&event, &caller()).await.unwrap();
    assert!(harness.calls().is_empty(), "{:?}", harness.calls());
    assert_eq!(harness.records()[0].action, AuditAction::Updated);
}

#[tokio::test]
async fn update_empty_alias_rename_old_team() {
    let mut harness = wired(STORE).with_teams();
    harness
        .manager
        .secrets
        .insert("litellm/old-alias".to_owned(), "sk-stored".to_owned());
    let event = updated(
        r#"{"key": "sk-1", "key_alias": "", "team_id": "team-new"}"#,
        ROW,
        "{}",
    );
    harness.dispatch(&event, &caller()).await.unwrap();
    assert_eq!(
        harness.calls(),
        [
            Call::Read {
                name: "litellm/old-alias".to_owned(),
            },
            Call::Rotate {
                current: "litellm/old-alias".to_owned(),
                new: "litellm/".to_owned(),
                value: "sk-stored".to_owned(),
            },
        ]
    );
    assert_eq!(
        harness.steps(),
        [
            "team:team-a",
            "read:litellm/old-alias",
            "rotate:litellm/old-alias>litellm/"
        ]
    );
    assert_eq!(harness.team_lookups(), ["team-a"]);

    let mut fallback = wired(STORE);
    fallback.manager.secrets.insert(
        "litellm/virtual-key-sk-old".to_owned(),
        "sk-stored".to_owned(),
    );
    let event = updated(
        r#"{"key": "sk-1", "key_alias": "renamed"}"#,
        r#"{"token": "sk-old", "key_alias": ""}"#,
        "{}",
    );
    fallback.dispatch(&event, &caller()).await.unwrap();
    assert_eq!(
        fallback.calls(),
        [
            Call::Read {
                name: "litellm/virtual-key-sk-old".to_owned(),
            },
            Call::Rotate {
                current: "litellm/virtual-key-sk-old".to_owned(),
                new: "litellm/renamed".to_owned(),
                value: "sk-stored".to_owned(),
            },
        ]
    );
}

#[tokio::test]
async fn update_rename_failure_continues() {
    let mut harness = wired(BOTH);
    harness
        .manager
        .failing
        .insert("litellm/old-alias".to_owned());
    let event = updated(r#"{"key": "sk-1", "key_alias": "renamed"}"#, ROW, "{}");
    harness.dispatch(&event, &caller()).await.unwrap();
    let records = harness.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].object_id, hash_token_if_needed("sk-1"));
    assert!(
        harness.warnings()[0].starts_with("Failed to rename virtual key in secret manager"),
        "{:?}",
        harness.warnings()
    );
}

#[tokio::test]
async fn update_project_null_survives_masking() {
    let harness = wired(AUDIT);
    let event = updated(
        r#"{"key": "sk-1", "key_alias": "alias-b", "project_id": null}"#,
        ROW,
        "{}",
    );
    harness.dispatch(&event, &caller()).await.unwrap();
    let records = harness.records();
    let Some(JsonValue::Object(values)) = &records[0].updated_values else {
        panic!("the updated values are a masked object");
    };
    assert_eq!(values.get("project_id"), Some(&JsonValue::Null));
    assert_eq!(values.get("key"), Some(&serde_json::json!("****")));
    assert_eq!(values.get("models"), Some(&serde_json::json!([])));
    assert_eq!(values.get("team_id"), None);
}

#[tokio::test]
async fn update_before_value_encoding() {
    let harness = wired(AUDIT);
    let event = updated(
        r#"{"key": "sk-1", "key_alias": "renamed"}"#,
        r#"{"token": "sk-1", "key_alias": "old-alias", "spend": 1.5, "expires": "2026-03-01", "created_at": "2026-01-02T03:04:05+00:00", "budget_reset_at": "2026-01-03T04:05:06.123456Z"}"#,
        "{}",
    );
    harness.dispatch(&event, &caller()).await.unwrap();
    let records = harness.records();
    let Some(JsonValue::Object(values)) = &records[0].before_value else {
        panic!("the before values are a masked object");
    };
    assert_eq!(
        values.get("created_at"),
        Some(&serde_json::json!("2026-01-02 03:04:05+00:00"))
    );
    assert_eq!(
        values.get("budget_reset_at"),
        Some(&serde_json::json!("2026-01-03 04:05:06.123456+00:00"))
    );
    assert_eq!(
        values.get("expires"),
        Some(&serde_json::json!("2026-03-01"))
    );
    assert_eq!(
        values.get("key_alias"),
        Some(&serde_json::json!("old-*lias"))
    );
    assert_eq!(values.get("token"), Some(&serde_json::json!("****")));
    assert_eq!(values.get("spend"), Some(&serde_json::json!(1.5)));
    assert_eq!(values.get("rotation_count"), Some(&serde_json::json!(0)));
    assert_eq!(values.get("auto_rotate"), Some(&serde_json::json!(false)));
    assert!(values.values().all(|value| !value.is_null()));
    assert!(
        !records[0]
            .before_value
            .as_ref()
            .unwrap()
            .to_string()
            .contains("sk-1")
    );
}

#[tokio::test]
async fn update_response_unused() {
    let harness = wired(AUDIT);
    let request = r#"{"key": "sk-1", "key_alias": "renamed"}"#;
    harness
        .dispatch(
            &updated(request, ROW, r#"{"key": "sk-other", "marker": "zzz"}"#),
            &caller(),
        )
        .await
        .unwrap();
    harness
        .dispatch(&updated(request, ROW, "{}"), &caller())
        .await
        .unwrap();
    let records = harness.records();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].updated_values, records[1].updated_values);
    assert_eq!(records[0].before_value, records[1].before_value);
    let masked = records[0].updated_values.as_ref().unwrap().to_string();
    assert!(!masked.contains("marker"), "{masked}");
    assert!(!masked.contains("sk-other"), "{masked}");
}

#[rstest]
#[case::response_alias_wins(Some(r#"{"key_alias": "req"}"#), "resp", "litellm/resp")]
#[case::requested_alias_when_response_blank(Some(r#"{"key_alias": "req"}"#), "", "litellm/req")]
#[case::response_alias_without_request(None, "resp", "litellm/resp")]
#[case::blank_requested_alias_falls_to_response(
    Some(r#"{"key_alias": ""}"#),
    "resp",
    "litellm/resp"
)]
#[case::initial_name_when_both_blank(None, "", "litellm/old-alias")]
#[case::initial_name_for_blank_alias(Some(r#"{"key_alias": ""}"#), "", "litellm/old-alias")]
#[tokio::test]
async fn rotation_name_precedence(
    #[case] request: Option<&str>,
    #[case] response_alias: &str,
    #[case] expected: &str,
) {
    let harness = wired(STORE);
    let response =
        format!(r#"{{"key": "sk-new", "token_id": "tok-new", "key_alias": "{response_alias}"}}"#);
    harness
        .dispatch(&rotated(request, ROW, &response), &caller())
        .await
        .unwrap();
    assert_eq!(
        harness.calls(),
        [Call::Rotate {
            current: "litellm/old-alias".to_owned(),
            new: expected.to_owned(),
            value: "sk-new".to_owned(),
        }],
        "{request:?} {response_alias:?}"
    );
    assert!(
        harness.records().is_empty(),
        "{request:?} {response_alias:?}"
    );

    let fallback = wired(STORE);
    fallback
        .dispatch(
            &rotated(
                None,
                r#"{"token": "sk-old", "key_alias": ""}"#,
                r#"{"key": "sk-new", "token_id": "tok-new"}"#,
            ),
            &caller(),
        )
        .await
        .unwrap();
    assert_eq!(
        fallback.calls(),
        [Call::Rotate {
            current: "litellm/virtual-key-sk-old".to_owned(),
            new: "litellm/virtual-key-sk-old".to_owned(),
            value: "sk-new".to_owned(),
        }]
    );
}

#[tokio::test]
async fn rotation_no_new_token_skips_store() {
    let harness = wired(BOTH);
    let event = rotated(
        None,
        r#"{"token": "sk-old", "key_alias": "old-alias"}"#,
        r#"{"key": "sk-new", "key_alias": "resp"}"#,
    );
    harness.dispatch(&event, &caller()).await.unwrap();
    assert!(harness.calls().is_empty(), "{:?}", harness.calls());
    assert_eq!(harness.steps(), ["email:rotated", "audit:sk-old"]);
    assert_eq!(harness.rotated_emails(), 1);
}

#[tokio::test]
async fn rotation_store_warning_still_emails() {
    let mut harness = wired(BOTH);
    harness
        .manager
        .failing
        .insert("litellm/old-alias".to_owned());
    let event = rotated(Some(r#"{"key_alias": "req"}"#), ROW, RESPONSE);
    harness.dispatch(&event, &caller()).await.unwrap();
    assert_eq!(
        harness.steps(),
        [
            "rotate:litellm/old-alias>litellm/alias-a",
            "email:rotated",
            "audit:sk-old-key",
        ]
    );
    assert_eq!(harness.rotated_emails(), 1);
    assert_eq!(harness.records().len(), 1);
    assert!(
        harness.warnings()[0].starts_with("Failed to rotate virtual key in secret manager"),
        "{:?}",
        harness.warnings()
    );
}

#[tokio::test]
async fn rotation_email_warning_still_audits() {
    let harness = wired(BOTH);
    harness.email.fail_rotated.store(true, Ordering::Relaxed);
    let event = rotated(None, ROW, RESPONSE);
    harness.dispatch(&event, &caller()).await.unwrap();
    assert_eq!(
        harness.steps(),
        [
            "rotate:litellm/old-alias>litellm/alias-a",
            "email:rotated",
            "audit:sk-old-key",
        ]
    );
    assert_eq!(harness.records().len(), 1);
    assert!(
        harness.warnings()[0].starts_with("Failed to send key rotated email"),
        "{:?}",
        harness.warnings()
    );
}

#[tokio::test]
async fn rotation_missing_old_token_skips_audit() {
    let harness = wired(BOTH);
    let event = rotated(None, r#"{"key_alias": "old-alias"}"#, RESPONSE);
    harness.dispatch(&event, &caller()).await.unwrap();
    assert_eq!(
        harness.steps(),
        ["rotate:litellm/old-alias>litellm/alias-a", "email:rotated",]
    );
    assert!(harness.records().is_empty());
}

#[tokio::test]
async fn rotation_audit_identity() {
    let harness = wired(AUDIT);
    let event = rotated(
        None,
        r#"{"token": "sk-old-key", "key_alias": "old-alias", "created_at": "2026-01-02T03:04:05+00:00"}"#,
        RESPONSE,
    );
    harness.dispatch(&event, &caller()).await.unwrap();
    let records = harness.records();
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(record.action, AuditAction::Rotated);
    assert_eq!(record.object_id, "sk-old-key");
    assert_eq!(exposed(&record.changed_by_api_key), Some("caller-token"));
    let Some(JsonValue::Object(before)) = &record.before_value else {
        panic!("the rotated before values are a masked object");
    };
    assert_eq!(
        before.get("created_at"),
        Some(&serde_json::json!("2026-01-02T03:04:05Z"))
    );
    assert_eq!(before.get("token"), Some(&serde_json::json!("sk-o**-key")));
    let Some(JsonValue::Object(after)) = &record.updated_values else {
        panic!("the rotated values are a masked object");
    };
    assert_eq!(after.get("key"), Some(&serde_json::json!("sk-n**-key")));
    assert_eq!(after.get("token_id"), Some(&serde_json::json!("*****")));
}

#[tokio::test]
async fn delete_audit_first() {
    let harness = wired(BOTH);
    let event = deleted(
        r#"[{"token": "tok-a", "key_alias": "alias-a"}, {"token": "tok-b", "key_alias": "alias-b"}]"#,
    );
    harness.dispatch(&event, &caller()).await.unwrap();
    assert_eq!(
        harness.steps(),
        [
            "audit:tok-a",
            "audit:tok-b",
            "delete:litellm/alias-a",
            "delete:litellm/alias-b",
        ]
    );
    let records = harness.records();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].action, AuditAction::Deleted);
    assert_eq!(records[0].updated_values, Some(serde_json::json!({})));
    assert_eq!(
        exposed(&records[0].changed_by_api_key),
        Some("caller-token")
    );
}

#[tokio::test]
async fn delete_disabled_skips_validation() {
    let harness = wired(STORE);
    let event = deleted(r#"[{"token": "tok-a", "key_alias": "alias-a", "spend": "not-a-number"}]"#);
    harness.dispatch(&event, &caller()).await.unwrap();
    assert_eq!(harness.steps(), ["delete:litellm/alias-a"]);
    assert!(harness.records().is_empty());
}

#[tokio::test]
async fn delete_null_vs_empty_token() {
    let harness = wired(BOTH);
    let event = deleted(
        r#"[{"token": null, "key_alias": "alias-null"}, {"token": "", "key_alias": "alias-empty"}]"#,
    );
    harness.dispatch(&event, &caller()).await.unwrap();
    assert_eq!(
        harness.steps(),
        [
            "audit:",
            "delete:litellm/alias-null",
            "delete:litellm/alias-empty",
        ]
    );
    let records = harness.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].object_id, "");
}

#[tokio::test]
async fn delete_invalid_later_row_partial() {
    let harness = wired(BOTH);
    let event = deleted(
        r#"[{"token": "tok-a", "key_alias": "alias-a"}, {"token": "tok-b", "key_alias": "alias-b", "spend": "bad"}]"#,
    );
    let error = harness.dispatch(&event, &caller()).await.unwrap_err();
    assert!(matches!(error, KeyEventError::Row(_)));
    assert_eq!(harness.steps(), ["audit:tok-a"]);
    assert_eq!(harness.records().len(), 1);
}

#[tokio::test]
async fn delete_sink_exception_propagates() {
    let mut harness = wired(BOTH);
    harness.manager.failing.insert("litellm/alias-a".to_owned());
    let event = deleted(r#"[{"token": "tok-a", "key_alias": "alias-a"}]"#);
    let error = harness.dispatch(&event, &caller()).await.unwrap_err();
    assert!(matches!(error, KeyEventError::Sink { .. }));
    assert_eq!(harness.steps(), ["audit:tok-a", "delete:litellm/alias-a"]);
}

#[tokio::test]
async fn delete_unused_request_fields() {
    let harness = wired(BOTH);
    let event = deleted(r#"[{"token": "tok-a", "key_alias": "alias-a"}]"#);
    harness.dispatch(&event, &caller()).await.unwrap();
    let records = harness.records();
    assert_eq!(records.len(), 1);
    let before = records[0].before_value.as_ref().unwrap().to_string();
    assert!(!before.contains("marker"), "{before}");
    assert_eq!(harness.steps(), ["audit:tok-a", "delete:litellm/alias-a"]);
}

#[tokio::test]
async fn audit_no_raw_key() {
    let harness = harness();
    let fixture = sink_fixture();
    let created = generated(
        r#"{"key_alias": "alias-a"}"#,
        r#"{"key": "sk-raw-secret-value-1234", "token_id": "tok-1"}"#,
    );
    harness
        .dispatch_with(&fixture.audit, &created, &caller())
        .await
        .unwrap();
    let rotated_event = rotated(
        None,
        r#"{"token": "sk-old-secret-value-9999", "key_alias": "old-alias"}"#,
        r#"{"key": "sk-raw-secret-value-1234", "token_id": "tok-2", "key_alias": "resp"}"#,
    );
    harness
        .dispatch_with(&fixture.audit, &rotated_event, &caller())
        .await
        .unwrap();
    fixture.spawner.drain().await;
    let records = lock(&fixture.repository.records).clone();
    assert_eq!(records.len(), 2);
    let texts = [
        records[0].before_value.as_ref(),
        records[0].updated_values.as_ref(),
        records[1].before_value.as_ref(),
        records[1].updated_values.as_ref(),
    ]
    .into_iter()
    .flatten()
    .map(JsonValue::to_string)
    .collect::<Vec<_>>();
    assert!(
        texts
            .iter()
            .all(|text| !text.contains("raw-secret-value") && !text.contains("old-secret-value")),
        "{texts:?}"
    );
    let Some(JsonValue::Object(created_values)) = &records[0].updated_values else {
        panic!("the created values are a masked object");
    };
    let masked = created_values
        .get("key")
        .and_then(JsonValue::as_str)
        .expect("a masked key");
    assert!(masked.starts_with("sk-r"), "{masked}");
    assert!(masked.ends_with("1234"), "{masked}");
    assert_eq!(
        created_values.get("token_id"),
        Some(&serde_json::json!("*****"))
    );
}

#[tokio::test]
async fn generated_nested_model_nulls_dropped() {
    let harness = wired(AUDIT);
    let event = generated(
        r#"{"key_alias": "alias-a"}"#,
        r#"{"key": "sk-new-key", "token_id": "tok-1", "object_permission": {"models": ["gpt-4"]}, "router_settings": {"num_retries": null}}"#,
    );
    harness.dispatch(&event, &caller()).await.unwrap();
    let Some(JsonValue::Object(values)) = &harness.records()[0].updated_values else {
        panic!("the created values are a masked object");
    };
    assert_eq!(
        values.get("object_permission"),
        Some(&serde_json::json!({"models": ["gpt-4"]}))
    );
    assert_eq!(
        values.get("router_settings"),
        Some(&serde_json::json!({"model_group_alias": {}}))
    );
}

#[tokio::test]
async fn update_budget_limit_window_rendering() {
    let harness = wired(AUDIT);
    let event = updated(
        r#"{"key": "sk-1", "key_alias": "renamed", "budget_limits": [{"budget_duration": "1d", "max_budget": 10, "reset_at": "2026-01-02T03:04:05Z"}]}"#,
        ROW,
        "{}",
    );
    harness.dispatch(&event, &caller()).await.unwrap();
    let Some(JsonValue::Object(values)) = &harness.records()[0].updated_values else {
        panic!("the updated values are a masked object");
    };
    let Some(JsonValue::Array(limits)) = values.get("budget_limits") else {
        panic!("the updated values carry the budget limits");
    };
    let Some(JsonValue::Object(first)) = limits.first() else {
        panic!("the first budget limit is an object");
    };
    assert_eq!(
        first.get("reset_at"),
        Some(&serde_json::json!("2026-01-02 03:04:05+00:00"))
    );
}

#[tokio::test]
async fn naive_row_instants_render_in_both_forms() {
    let row = r#"{"token": "sk-1", "key_alias": "old-alias", "created_at": "2026-01-02T03:04:05"}"#;
    let updated_harness = wired(AUDIT);
    let event = updated(r#"{"key": "sk-1", "key_alias": "renamed"}"#, row, "{}");
    updated_harness.dispatch(&event, &caller()).await.unwrap();
    let Some(JsonValue::Object(before_updated)) = &updated_harness.records()[0].before_value else {
        panic!("the before values are a masked object");
    };
    assert_eq!(
        before_updated.get("created_at"),
        Some(&serde_json::json!("2026-01-02 03:04:05"))
    );

    let rotated_harness = wired(AUDIT);
    rotated_harness
        .dispatch(&rotated(None, row, RESPONSE), &caller())
        .await
        .unwrap();
    let Some(JsonValue::Object(before_rotated)) = &rotated_harness.records()[0].before_value else {
        panic!("the rotated before values are a masked object");
    };
    assert_eq!(
        before_rotated.get("created_at"),
        Some(&serde_json::json!("2026-01-02T03:04:05"))
    );
}

#[tokio::test]
async fn row_expiry_datetime_rendered() {
    let mut row = key_row(ROW);
    row.expires = Some(Expiry::DateTime(
        DateTime::parse_from_rfc3339("2026-03-01T00:00:00+00:00").unwrap(),
    ));
    let event = KeyEvent::Updated(UpdatedEvent {
        data: UpdateKeyRequest::read(&value(r#"{"key": "sk-1", "key_alias": "renamed"}"#))
            .expect("an update request"),
        row,
        response: data("{}"),
    });
    let harness = wired(AUDIT);
    harness.dispatch(&event, &caller()).await.unwrap();
    let Some(JsonValue::Object(before)) = &harness.records()[0].before_value else {
        panic!("the before values are a masked object");
    };
    assert_eq!(
        before.get("expires"),
        Some(&serde_json::json!("2026-03-01 00:00:00+00:00"))
    );
}

#[tokio::test]
async fn legacy_email_effects_created_and_rotated() {
    let mailer = Arc::new(RecordingMailer::default());
    let tasks = TaskTracker::new();
    let alerting = vec!["email".to_owned()];
    let email = LegacyEmail {
        alerting: &alerting,
        mailer: Arc::clone(&mailer),
        tasks: &tasks,
    };
    let rejected = CreatedKey {
        token: Reported::Missing,
        spend: Reported::Null(()),
        max_budget: Reported::Missing,
        user_id: Reported::Missing,
        team_id: Reported::Missing,
        key_alias: Reported::Missing,
    };
    assert!(matches!(
        KeyEventEmail::send_created(&email, rejected).await,
        Err(KeyEmailError::Rejected(EmailError::NullSpend))
    ));
    let created: CreatedKey =
        serde_json::from_str(r#"{"spend": 1.5, "token": "hashed"}"#).expect("a created key");
    KeyEventEmail::send_created(&email, created).await.unwrap();
    tasks.close();
    tasks.wait().await;
    assert_eq!(lock(&mailer.events).len(), 1);

    let logs = DebugLines::default();
    let _logging = set_default(tracing_subscriber::registry().with(logs.clone()));
    KeyEventEmail::send_rotated(&email, "sk-new", Some("old-alias"))
        .await
        .unwrap();
    assert_eq!(
        logs.lines(),
        [(
            Level::DEBUG,
            "Enterprise package not installed, skipping key rotated email".to_owned(),
        )]
    );
}
