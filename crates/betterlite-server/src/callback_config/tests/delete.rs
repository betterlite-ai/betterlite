use std::collections::BTreeMap;

use axum::body::Body;
use axum::http::{HeaderValue, Request, StatusCode, header};
use rstest::rstest;

use betterlite_config::sections::settings::JsonValue;

use super::support::{Harness, internal_key, json, master_key, object, request_with_body};
use crate::audit_events::enums::{AuditAction, AuditTable};
use crate::config_overlay_port::ConfigOverlayPortError;
use crate::proxy_config_state::error::{ConfigOwnedDetail, ConfigStateError};

const DB_MESSAGE: &str = "DB not connected. This endpoint needs a database; set DATABASE_URL to a PostgreSQL connection string (postgresql://...) to enable it. See https://docs.litellm.ai/docs/proxy/virtual_keys";
const NOT_ALLOWED: &str = "Admin-only endpoint. Not allowed to access this.";
const STORE_MESSAGE: &str = "Set `'STORE_MODEL_IN_DB='True'` in your env to enable this feature.";

fn delete_call(name: &str) -> Request<Body> {
    let body = format!(r#"{{"callback_name": "{name}"}}"#);
    request_with_body("POST", "/config/callback/delete", &body)
}

fn success_config(callbacks: &str) -> BTreeMap<String, JsonValue> {
    object(&[(
        "litellm_settings",
        json(&format!(
            r#"{{"success_callback": {callbacks}, "drop_params": true}}"#
        )),
    )])
}

#[tokio::test]
async fn refuses_without_a_database() {
    let harness = Harness::new(BTreeMap::new()).without_database();
    let (status, body) = harness
        .call(delete_call("langfuse"), Some(master_key()))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body,
        json(&format!(r#"{{"detail": {{"error": "{DB_MESSAGE}"}}}}"#))
    );
    assert_eq!(harness.reload_calls(), 0);
}

#[tokio::test]
async fn refuses_a_non_admin_with_its_role() {
    let harness = Harness::new(BTreeMap::new());
    let (status, body) = harness
        .call(delete_call("langfuse"), Some(internal_key()))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body,
        json(&format!(
            r#"{{"detail": {{"error": "{NOT_ALLOWED}, your role=internal_user"}}}}"#
        ))
    );
    assert_eq!(harness.reload_calls(), 0);
}

#[tokio::test]
async fn refuses_when_store_model_in_db_is_off() {
    let harness = Harness::new(BTreeMap::new()).without_store_model_in_db();
    let (status, body) = harness
        .call(delete_call("langfuse"), Some(master_key()))
        .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        body,
        json(&format!(r#"{{"detail": {{"error": "{STORE_MESSAGE}"}}}}"#))
    );
    assert_eq!(harness.reload_calls(), 0);
}

#[tokio::test]
async fn removes_the_first_match_and_saves_the_config() {
    let harness = Harness::new(success_config(r#"["langfuse", "other", "langfuse"]"#));
    let (status, body) = harness
        .call(delete_call("Langfuse"), Some(master_key()))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        json(
            r#"{"message": "Successfully deleted callback: langfuse", "removed_callback": "langfuse", "remaining_callbacks": ["other", "langfuse"], "deleted_at": "2026-01-02T03:04:05.123456"}"#
        )
    );
    let saved = harness.saved();
    assert_eq!(saved.len(), 1);
    assert_eq!(
        saved[0].values["litellm_settings"]["success_callback"],
        json(r#"["other", "langfuse"]"#)
    );
    assert_eq!(
        saved[0].values["litellm_settings"]["drop_params"],
        json("true")
    );
    assert_eq!(harness.deployment_calls(), 1);
}

// docs/deviations/B8596.yaml: LiteLLM answers 422 for a body whose content type is not JSON, betterlite parses it anyway
#[tokio::test]
async fn reads_the_body_whatever_the_content_type() {
    let harness = Harness::new(success_config(r#"["langfuse"]"#));
    let mut request = delete_call("langfuse");
    request
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain"));
    let (status, body) = harness.call(request, Some(master_key())).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["removed_callback"], json(r#""langfuse""#));
}

#[tokio::test]
async fn answers_404_for_a_callback_that_is_not_configured() {
    let harness = Harness::new(success_config(r#"["langfuse"]"#));
    let (status, body) = harness
        .call(delete_call("Unknown"), Some(master_key()))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        body,
        json(r#"{"detail": {"error": "Callback 'unknown' not found in active configuration"}}"#)
    );
    assert_eq!(harness.saved().len(), 0);
    assert_eq!(harness.audit_records().len(), 0);
    assert_eq!(harness.deployment_calls(), 0);
}

// the request name is lower-cased, the stored names are compared exactly as they are stored
#[tokio::test]
async fn does_not_lower_case_stored_names() {
    let harness = Harness::new(success_config(r#"["Langfuse"]"#));
    let (status, body) = harness
        .call(delete_call("langfuse"), Some(master_key()))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        body,
        json(r#"{"detail": {"error": "Callback 'langfuse' not found in active configuration"}}"#)
    );
    assert_eq!(harness.saved().len(), 0);
}

#[tokio::test]
async fn audits_the_success_callback_before_and_after() {
    let harness = Harness::new(success_config(r#"["langfuse", "other"]"#));
    let (_status, _body) = harness
        .call(delete_call("langfuse"), Some(master_key()))
        .await;
    let records = harness.audit_records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].object_id, "litellm_settings");
    assert_eq!(records[0].action, AuditAction::Deleted);
    assert_eq!(records[0].table_name, AuditTable::Config);
    assert_eq!(records[0].changed_by.as_deref(), Some("default_user_id"));
    assert_eq!(
        records[0].before_value,
        Some(json(r#"{"success_callback": ["langfuse", "other"]}"#))
    );
    assert_eq!(
        records[0].updated_values,
        Some(json(r#"{"success_callback": ["other"]}"#))
    );
}

#[tokio::test]
async fn answers_the_config_owned_detail_when_the_file_owns_the_key() {
    let detail = ConfigOwnedDetail {
        error: "success_callback is set in the config file".to_owned(),
        keys: vec!["success_callback".to_owned()],
        section: "litellm_settings".to_owned(),
        stored_database_values_ignored: Vec::new(),
        resolution: "Remove success_callback from the config file".to_owned(),
    };
    let harness = Harness::new(success_config(r#"["langfuse"]"#));
    harness.failing_save(ConfigStateError::ConfigOwned(Box::new(detail.clone())));
    let (status, body) = harness
        .call(delete_call("langfuse"), Some(master_key()))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body["detail"],
        serde_json::to_value(&detail).expect("the detail serializes")
    );
    assert_eq!(harness.audit_records().len(), 0);
    assert_eq!(harness.deployment_calls(), 0);
}

#[tokio::test]
async fn answers_a_proxy_exception_when_the_save_fails() {
    let harness = Harness::new(success_config(r#"["langfuse"]"#));
    harness.failing_save(ConfigStateError::InvalidStoredSection {
        section: "litellm_settings",
    });
    let (status, body) = harness
        .call(delete_call("langfuse"), Some(master_key()))
        .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        body,
        json(
            r#"{"error": {"message": "Error deleting callback: the stored litellm_settings config row is not a JSON object", "type": "internal_server_error", "param": "callback_name", "code": "500"}}"#
        )
    );
    assert_eq!(harness.audit_records().len(), 0);
    assert_eq!(harness.deployment_calls(), 0);
}

#[tokio::test]
async fn answers_a_proxy_exception_when_the_reload_fails() {
    let harness =
        Harness::new(BTreeMap::new()).failing_reload(ConfigOverlayPortError::DatabaseUnavailable);
    let (status, body) = harness
        .call(delete_call("langfuse"), Some(master_key()))
        .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        body,
        json(
            r#"{"error": {"message": "Error deleting callback: the config database is not reachable", "type": "internal_server_error", "param": "callback_name", "code": "500"}}"#
        )
    );
}

// A single string is not the list success_callback declares, so the value is rejected with its field named.
#[tokio::test]
async fn names_the_field_when_success_callback_is_not_a_list() {
    let harness = Harness::new(object(&[(
        "litellm_settings",
        json(r#"{"success_callback": "langfuse"}"#),
    )]));
    let (status, body) = harness
        .call(delete_call("langfuse"), Some(master_key()))
        .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        body["error"]["message"],
        json(
            r#""Error deleting callback: litellm_settings.success_callback must be a list of strings""#
        )
    );
}

#[rstest]
#[case(
    "",
    r#"{"detail": [{"type": "missing", "loc": ["body"], "msg": "Field required", "input": null}]}"#
)]
#[case("{}", r#"{"detail": [{"type": "missing", "loc": ["body", "callback_name"], "msg": "Field required", "input": {}}]}"#)]
#[case(
    "null",
    r#"{"detail": [{"type": "model_attributes_type", "loc": ["body"], "msg": "Input should be a valid dictionary or object to extract fields from", "input": null}]}"#
)]
#[case(
    "5",
    r#"{"detail": [{"type": "model_attributes_type", "loc": ["body"], "msg": "Input should be a valid dictionary or object to extract fields from", "input": 5}]}"#
)]
#[case(
    r#"{"callback_name": 5}"#,
    r#"{"detail": [{"type": "string_type", "loc": ["body", "callback_name"], "msg": "Input should be a valid string", "input": 5}]}"#
)]
#[case(
    r#"{"callback_name": null}"#,
    r#"{"detail": [{"type": "string_type", "loc": ["body", "callback_name"], "msg": "Input should be a valid string", "input": null}]}"#
)]
#[case(
    "{bad",
    r#"{"detail": [{"type": "json_invalid", "loc": ["body"], "msg": "JSON decode error", "input": null}]}"#
)]
#[tokio::test]
async fn answers_fastapi_validation_errors(#[case] payload: &str, #[case] expected: &str) {
    let harness = Harness::new(success_config(r#"["langfuse"]"#));
    let request = request_with_body("POST", "/config/callback/delete", payload);
    let (status, body) = harness.call(request, Some(master_key())).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body, json(expected));
    assert_eq!(harness.reload_calls(), 0);
}

#[tokio::test]
async fn requires_authentication() {
    let harness = Harness::new(success_config(r#"["langfuse"]"#));
    let (status, _body) = harness.call(delete_call("langfuse"), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(harness.reload_calls(), 0);
}

#[rstest]
#[case("2026-01-02T03:04:05+02:00", "2026-01-02T03:04:05")]
#[case("2026-01-02T03:04:05.000100+02:00", "2026-01-02T03:04:05.000100")]
#[case("2026-01-02T03:04:05.900000+02:00", "2026-01-02T03:04:05.900000")]
#[tokio::test]
async fn formats_deleted_at_like_isoformat(#[case] instant: &str, #[case] expected: &str) {
    let harness = Harness::new(success_config(r#"["langfuse"]"#))
        .with_clock(chrono::DateTime::parse_from_rfc3339(instant).expect("a valid timestamp"));
    let (status, body) = harness
        .call(delete_call("langfuse"), Some(master_key()))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["deleted_at"], JsonValue::String(expected.to_owned()));
}
