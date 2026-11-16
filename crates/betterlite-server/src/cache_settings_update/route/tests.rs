#![cfg(test)]

use axum::http::StatusCode;
use betterlite_auth::resolved_key::credential::safe_hash_api_key;
use betterlite_config::sections::settings::JsonValue;
use betterlite_models::decisions::Json;
use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::audit_events::enums::AuditAction;
use crate::cache_settings_update::manager::params_equal;
use crate::cache_settings_update::support;

const NO_DATABASE: &str =
    r#"{"detail":{"error":"Database not connected. Please connect a database."}}"#;
const STORE_MODEL_IN_DB: &str =
    r#"{"detail":{"error":"Set `'STORE_MODEL_IN_DB='True'` in your env to enable this feature."}}"#;
const UNAUTHENTICATED: &str = r#"{"error":{"message":"the API key is not valid","type":"auth_error","param":"None","code":"401"}}"#;

fn string(value: &str) -> JsonValue {
    JsonValue::String(value.to_owned())
}

fn saved_target() -> JsonValue {
    support::stored_row(&[
        ("type", string("redis")),
        ("host", string("saved-host")),
        ("password", string(&support::sealed("saved-secret"))),
        ("redis_type", string("node")),
    ])
}

#[tokio::test]
async fn route_refuses_a_request_without_a_key() {
    let (fake, repository) = support::fake_repository();
    let harness = support::harness(Some(repository), support::reinit(), support::audit(true));
    let (status, text) = support::post(&harness, None, "{").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(text, UNAUTHENTICATED);
    assert_eq!(fake.read_count(), 0);
}

#[tokio::test]
async fn route_refuses_a_non_admin_key() {
    let (fake, repository) = support::fake_repository();
    let harness = support::harness(Some(repository), support::reinit(), support::audit(true));
    let body = support::request_body(support::json(r#"{"type":"local"}"#));
    let (status, text) = support::post(&harness, Some(support::non_admin()), &body).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        text,
        r#"{"detail":"Admin access required to update cache settings"}"#
    );
    assert_eq!(fake.read_count(), 0);
}

#[rstest]
#[case::empty_body(
    "",
    r#"{"detail":[{"type":"missing","loc":["body"],"msg":"Field required"}]}"#
)]
#[case::null_body(
    "null",
    r#"{"detail":[{"type":"missing","loc":["body"],"msg":"Field required"}]}"#
)]
#[case::missing_field(
    "{}",
    r#"{"detail":[{"type":"missing","loc":["body","cache_settings"],"msg":"Field required"}]}"#
)]
#[case::null_settings(
    r#"{"cache_settings":null}"#,
    r#"{"detail":[{"type":"dict_type","loc":["body","cache_settings"],"msg":"Input should be a valid dictionary"}]}"#
)]
#[case::not_an_object(
    "[]",
    r#"{"detail":[{"type":"model_attributes_type","loc":["body"],"msg":"Input should be a valid dictionary or object to extract fields from"}]}"#
)]
#[case::invalid_json(
    "{",
    r#"{"detail":[{"type":"json_invalid","loc":["body"],"msg":"JSON decode error"}]}"#
)]
#[tokio::test]
async fn route_answers_fastapi_validation_detail(#[case] body: &str, #[case] expected: &str) {
    let harness = support::harness(None, support::reinit(), support::audit(true));
    let (status, text) = support::post(&harness, Some(support::master_key()), body).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(text, expected);
}

#[tokio::test]
async fn route_refuses_a_request_without_a_database() {
    let harness = support::harness(None, support::reinit(), support::audit(true));
    let body = support::request_body(support::json(r#"{"type":"local"}"#));
    let (status, text) = support::post(&harness, Some(support::master_key()), &body).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(text, NO_DATABASE);
}

#[tokio::test]
async fn route_refuses_a_request_while_store_model_in_db_is_off() {
    let (fake, repository) = support::fake_repository();
    let mut harness = support::harness(Some(repository), support::reinit(), support::audit(true));
    harness.disable_store_model_in_db();
    let body = support::request_body(support::json(r#"{"type":"local"}"#));
    let (status, text) = support::post(&harness, Some(support::master_key()), &body).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(text, STORE_MODEL_IN_DB);
    assert_eq!(fake.read_count(), 0);
}

#[tokio::test]
async fn route_creates_the_row_and_reinitializes_the_cache() {
    let (fake, repository) = support::fake_repository();
    let reinit = support::reinit();
    let audit = support::audit(true);
    let harness = support::harness(Some(repository), reinit.clone(), audit.clone());
    let body = support::request_body(support::json(
        r#"{"type":"redis","host":"h","password":"new-secret","ttl":60}"#,
    ));
    let (status, text) = support::post(&harness, Some(support::master_key()), &body).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        text,
        r#"{"message":"Cache settings updated successfully","status":"success","settings":{"host":"h","password":"***REDACTED***","ttl":60,"type":"redis"}}"#
    );
    let upserted = fake.upserted();
    assert_eq!(upserted.len(), 1);
    let stored = support::decrypt_all(&support::json(&upserted[0]));
    assert_eq!(stored.get("password"), Some(&string("new-secret")));
    assert_eq!(stored.get("host"), Some(&string("h")));
    assert_ne!(
        support::json(&upserted[0]).get("password"),
        Some(&string("new-secret"))
    );
    let expected =
        support::params(r#"{"type":"redis","host":"h","password":"new-secret","ttl":60}"#);
    assert_eq!(
        reinit.calls(),
        vec![
            "init_cache".to_owned(),
            "switch_on_response_caching".to_owned()
        ]
    );
    assert!(params_equal(&reinit.params()[0], &expected));
    let stored_params = harness
        .manager
        .last_params()
        .expect("the params were stored");
    assert!(params_equal(&stored_params, &expected));
    assert_eq!(harness.spawner.len(), 1);
    support::run_tasks(&harness.spawner).await;
    let entries = audit.entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].action, AuditAction::Created);
    assert_eq!(entries[0].before_value, r#"{"settings":{}}"#);
    assert_eq!(
        entries[0].updated_values,
        r#"{"settings":{"host":"***REDACTED***","password":"***REDACTED***","ttl":"***REDACTED***","type":"***REDACTED***"}}"#
    );
}

#[tokio::test]
async fn route_marks_an_existing_row_as_updated() {
    let (fake, repository) = support::fake_repository();
    fake.push_read(Ok(Some(JsonValue::Object(Default::default()))));
    let audit = support::audit(true);
    let harness = support::harness(Some(repository), support::reinit(), audit.clone());
    let body = support::request_body(support::json(r#"{"type":"local"}"#));
    let (status, _text) = support::post(&harness, Some(support::master_key()), &body).await;
    assert_eq!(status, StatusCode::OK);
    support::run_tasks(&harness.spawner).await;
    let entries = audit.entries();
    assert_eq!(entries[0].action, AuditAction::Updated);
}

#[tokio::test]
async fn route_merges_the_saved_secret_and_keeps_redis_type_out_of_the_cache() {
    let (fake, repository) = support::fake_repository();
    fake.push_read(Ok(Some(saved_target())));
    let reinit = support::reinit();
    let harness = support::harness(Some(repository), reinit.clone(), support::audit(true));
    let body = support::request_body(support::json(
        r#"{"type":"redis","host":"saved-host","password":"***REDACTED***","redis_type":"cluster"}"#,
    ));
    let (status, text) = support::post(&harness, Some(support::master_key()), &body).await;
    assert_eq!(status, StatusCode::OK);
    assert!(text.contains(r#""password":"***REDACTED***""#));
    let initialized = &reinit.params()[0];
    assert_eq!(
        initialized.get("password"),
        Some(&Json::Text("saved-secret".to_owned()))
    );
    assert_eq!(
        initialized.get("host"),
        Some(&Json::Text("saved-host".to_owned()))
    );
    assert!(initialized.get("redis_type").is_none());
    let stored = support::decrypt_all(&support::json(&fake.upserted()[0]));
    assert_eq!(stored.get("redis_type"), Some(&string("cluster")));
    assert_eq!(stored.get("password"), Some(&string("saved-secret")));
}

#[tokio::test]
async fn route_answers_a_string_detail_when_the_cache_refuses_the_params() {
    let (fake, repository) = support::fake_repository();
    let harness = support::harness(
        Some(repository),
        support::failing_reinit(),
        support::audit(true),
    );
    let body = support::request_body(support::json(r#"{"type":"local"}"#));
    let (status, text) = support::post(&harness, Some(support::master_key()), &body).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        text,
        r#"{"detail":"Error updating cache settings: the cache could not be initialised: the cache refused the parameters"}"#
    );
    assert!(harness.manager.last_params().is_none());
    assert_eq!(fake.upserted().len(), 1);
}

#[tokio::test]
async fn route_answers_a_string_detail_when_the_row_cannot_be_read() {
    let (fake, repository) = support::fake_repository();
    fake.push_read(Err(support::refused_error("the row is unreadable")));
    let harness = support::harness(Some(repository), support::reinit(), support::audit(true));
    let body = support::request_body(support::json(r#"{"type":"local"}"#));
    let (status, text) = support::post(&harness, Some(support::master_key()), &body).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        text,
        r#"{"detail":"Error updating cache settings: the row is unreadable"}"#
    );
    assert!(fake.upserted().is_empty());
}

#[tokio::test]
async fn route_audits_the_key_identity_and_the_changed_by_header() {
    let (_fake, repository) = support::fake_repository();
    let audit = support::audit(true);
    let harness = support::harness(Some(repository), support::reinit(), audit.clone());
    let body = support::request_body(support::json(r#"{"type":"local"}"#));
    let (status, _text) = support::post_with_header(
        &harness,
        Some(support::user_key("admin-user", "sk-test-key")),
        &body,
        Some("reviewer"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    support::run_tasks(&harness.spawner).await;
    let entries = audit.entries();
    assert_eq!(entries[0].litellm_changed_by.as_deref(), Some("reviewer"));
    assert_eq!(entries[0].user_id.as_deref(), Some("admin-user"));
    assert_eq!(
        entries[0]
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
        Some(safe_hash_api_key("sk-test-key"))
    );
}
