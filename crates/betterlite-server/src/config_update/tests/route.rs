#![cfg(test)]

use std::collections::BTreeSet;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use betterlite_config::sections::settings::{JsonValue, Section};
use secrecy::ExposeSecret as _;
use tower::ServiceExt as _;

use super::super::routes;
use super::support::{
    Harness, RecordingStore, admin, config_state, error_field, error_text, key_user, object,
    parsed, send, settings, user,
};
use crate::audit_events::enums::{AuditAction, AuditTable};
use crate::organization_create::caller::DEFAULT_ADMIN_USER_ID;

const REFUSED_SETTINGS: &str =
    r#"{"general_settings": {"a": 1}, "router_settings": {"weights": {}, "nope": true}}"#;

#[tokio::test]
async fn only_proxy_admins_may_update_config() {
    let harness = Harness::new(config_state());
    let (status, body) = send(&harness, Some(user()), r#"{"general_settings": {"a": 1}}"#).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(error_text(&body), "Only proxy admins can update config");
    assert_eq!(error_field(&body, "type").as_str(), Some("auth_error"));
    assert_eq!(error_field(&body, "param").as_str(), Some("None"));
    assert_eq!(error_field(&body, "code").as_str(), Some("403"));
    assert!(harness.store.upserts().is_empty());
}

#[tokio::test]
async fn an_unauthenticated_call_is_refused() {
    let harness = Harness::new(config_state());
    let (status, body) = send(&harness, None, r#"{"general_settings": {}}"#).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(error_field(&body, "type").as_str(), Some("auth_error"));
}

#[tokio::test]
async fn a_bad_body_is_a_422_even_for_a_non_admin() {
    let harness = Harness::new(config_state());
    let (status, body) = send(
        &harness,
        Some(user()),
        r#"{"router_settings": {"allowed_fails": "three"}}"#,
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let detail = body.get("detail").and_then(|detail| detail.get(0));
    let loc = detail
        .and_then(|issue| issue.get("loc"))
        .expect("a location");
    assert_eq!(
        loc,
        &parsed(r#"["body", "router_settings", "allowed_fails"]"#)
    );
}

#[tokio::test]
async fn unsupported_router_settings_are_refused_before_any_write() {
    let harness = Harness::new(config_state());
    let (status, body) = send(&harness, Some(admin()), REFUSED_SETTINGS).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        error_text(&body),
        "Unsupported router settings: nope, weights are not valid router settings"
    );
    assert_eq!(error_field(&body, "code").as_str(), Some("400"));
    assert!(harness.store.upserts().is_empty());
    assert!(harness.store.reads().is_empty());
}

#[tokio::test]
async fn invalid_routing_groups_are_refused_with_the_parser_message() {
    let harness = Harness::new(config_state());
    let body_text = r#"{"router_settings": {"routing_groups": [
        {"group_name": "g", "models": ["m1"], "routing_strategy": "simple-shuffle"},
        {"group_name": "g", "models": ["m2"], "routing_strategy": "simple-shuffle"}
    ]}}"#;
    let (status, body) = send(&harness, Some(admin()), body_text).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(error_text(&body).contains("group names must be unique"));
    assert!(harness.store.upserts().is_empty());
}

#[tokio::test]
async fn a_missing_database_is_a_400_with_the_authentication_prefix() {
    let harness = Harness::without_database(config_state());
    let (status, body) = send(&harness, Some(admin()), r#"{"general_settings": {}}"#).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error_text(&body), "Authentication Error, No DB Connected");
}

#[tokio::test]
async fn config_owned_keys_are_refused_before_any_write() {
    let mut config = config_state();
    config
        .general_settings
        .load_yaml(settings(&[("store_model_in_db", JsonValue::Bool(true))]));
    let harness = Harness::new(config);
    let (status, body) = send(
        &harness,
        Some(admin()),
        r#"{"general_settings": {"store_model_in_db": false}}"#,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(error_text(&body).contains("store_model_in_db"));
    assert!(harness.store.upserts().is_empty());
    harness.spawner.run_all().await;
    assert!(harness.repository.records().is_empty());
}

#[tokio::test]
async fn general_settings_are_merged_stored_and_audited() {
    let store = RecordingStore::holding(&[(
        Section::GeneralSettings,
        settings(&[("global_max_parallel_requests", parsed("4"))]),
    )]);
    let harness = Harness::holding(config_state(), store);
    let (status, body) = send(
        &harness,
        Some(admin()),
        r#"{"general_settings": {"alert_to_webhook_url": {"budget_alerts": "https://hooks/x"}}}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body.get("message").and_then(JsonValue::as_str),
        Some("Config updated successfully")
    );
    let expected = object(
        r#"{"global_max_parallel_requests": 4, "alerting": ["slack"], "alert_to_webhook_url": {"budget_alerts": "https://hooks/x"}}"#,
    );
    assert_eq!(
        harness.store.stored(Section::GeneralSettings),
        Some(expected)
    );
    let upserts = harness.store.upserts();
    assert_eq!(
        upserts.first().map(|(name, _)| name.as_str()),
        Some("general_settings")
    );
    harness.spawner.run_all().await;
    let record = harness
        .repository
        .records()
        .into_iter()
        .next()
        .expect("one audit record");
    assert_eq!(record.object_id, "general_settings");
    assert_eq!(record.action, AuditAction::Updated);
    assert_eq!(record.table_name, AuditTable::Config);
    assert_eq!(record.changed_by.as_deref(), Some(DEFAULT_ADMIN_USER_ID));
    assert_eq!(
        record.before_value,
        Some(parsed(r#"{"global_max_parallel_requests": 4}"#))
    );
    assert_eq!(
        record.updated_values,
        Some(parsed(
            r#"{"global_max_parallel_requests": 4, "alerting": ["slack"], "alert_to_webhook_url": "REDACTED"}"#
        ))
    );
}

#[tokio::test]
async fn environment_variables_are_sealed_then_merged_and_audited() {
    let store = RecordingStore::holding(&[(
        Section::EnvironmentVariables,
        settings(&[("STORED", JsonValue::String("sealed:stored".to_owned()))]),
    )]);
    let harness = Harness::holding(config_state(), store);
    let (status, _body) = send(
        &harness,
        Some(admin()),
        r#"{"environment_variables": {"NEW": "value"}}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        harness.store.seals(),
        vec![settings(&[("NEW", JsonValue::String("value".to_owned()))])]
    );
    assert_eq!(
        harness.store.stored(Section::EnvironmentVariables),
        Some(object(
            r#"{"STORED": "sealed:stored", "NEW": "sealed:value"}"#
        ))
    );
    harness.spawner.run_all().await;
    let record = harness
        .repository
        .records()
        .into_iter()
        .next()
        .expect("one audit record");
    assert_eq!(record.object_id, "environment_variables");
    let audited = record
        .updated_values
        .as_ref()
        .map(JsonValue::to_string)
        .unwrap_or_default();
    assert!(!audited.contains("sealed:value"));
}

#[tokio::test]
async fn litellm_settings_union_the_success_callbacks_and_are_audited() {
    let store = RecordingStore::holding(&[(
        Section::LitellmSettings,
        settings(&[("success_callback", parsed(r#"["S3"]"#))]),
    )]);
    let harness = Harness::holding(config_state(), store);
    let (status, _body) = send(
        &harness,
        Some(admin()),
        r#"{"litellm_settings": {"success_callback": ["New"], "drop_params": true}}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let stored = harness
        .store
        .stored(Section::LitellmSettings)
        .expect("the stored litellm_settings");
    let callbacks: BTreeSet<String> = stored
        .get("success_callback")
        .and_then(JsonValue::as_array)
        .expect("the stored callbacks")
        .iter()
        .filter_map(|callback| callback.as_str().map(str::to_owned))
        .collect();
    assert_eq!(
        callbacks,
        BTreeSet::from(["new".to_owned(), "s3".to_owned()])
    );
    assert_eq!(stored.get("drop_params"), Some(&JsonValue::Bool(true)));
    let stored_value = JsonValue::Object(
        stored
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
    );
    harness.spawner.run_all().await;
    let record = harness
        .repository
        .records()
        .into_iter()
        .next()
        .expect("one audit record");
    assert_eq!(record.object_id, "litellm_settings");
    assert_eq!(record.updated_values, Some(stored_value));
}

#[tokio::test]
async fn router_settings_store_the_typed_and_raw_merge_and_audit_the_upserted_value() {
    let store = RecordingStore::holding(&[(
        Section::RouterSettings,
        settings(&[("num_retries", parsed("1"))]),
    )]);
    let harness = Harness::holding(config_state(), store);
    let (status, _body) = send(
        &harness,
        Some(admin()),
        r#"{"router_settings": {"timeout": 5, "max_retries": null, "enable_weighted_failover": true}}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let expected =
        object(r#"{"num_retries": 1, "timeout": 5.0, "enable_weighted_failover": true}"#);
    assert_eq!(
        harness.store.stored(Section::RouterSettings),
        Some(expected)
    );
    harness.spawner.run_all().await;
    let record = harness
        .repository
        .records()
        .into_iter()
        .next()
        .expect("one audit record");
    assert_eq!(record.object_id, "router_settings");
    assert_eq!(
        record.updated_values,
        Some(parsed(
            r#"{"num_retries": 1, "timeout": 5.0, "enable_weighted_failover": true}"#
        ))
    );
}

#[tokio::test]
async fn sections_are_written_in_litellm_order_and_unsent_sections_stay_untouched() {
    let harness = Harness::new(config_state());
    let body_text = r#"{"router_settings": {}, "litellm_settings": {}, "environment_variables": {"A": "a"}, "general_settings": {}}"#;
    let (status, _body) = send(&harness, Some(admin()), body_text).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        harness.store.reads(),
        [
            "general_settings",
            "environment_variables",
            "litellm_settings",
            "router_settings"
        ]
    );
    assert_eq!(harness.store.upserts().len(), 4);
    harness.spawner.run_all().await;
    let ids: Vec<String> = harness
        .repository
        .records()
        .into_iter()
        .map(|record| record.object_id)
        .collect();
    assert_eq!(
        ids,
        [
            "general_settings",
            "environment_variables",
            "litellm_settings",
            "router_settings"
        ]
    );
}

#[tokio::test]
async fn a_successful_update_runs_the_deployment_sync_once() {
    let harness = Harness::new(config_state());
    let (status, _body) = send(&harness, Some(admin()), r#"{"general_settings": {}}"#).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(harness.deployments.count(), 1);
}

#[tokio::test]
async fn a_refused_update_never_runs_the_deployment_sync() {
    let harness = Harness::new(config_state());
    let (status, _body) = send(&harness, Some(admin()), REFUSED_SETTINGS).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(harness.deployments.count(), 0);
}

#[tokio::test]
async fn a_failing_audit_row_does_not_fail_the_update() {
    let harness = Harness::failing_audit(config_state());
    let (status, _body) = send(&harness, Some(admin()), r#"{"general_settings": {"a": 1}}"#).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(harness.store.upserts().len(), 1);
    harness.spawner.run_all().await;
    assert!(harness.repository.records().is_empty());
}

#[tokio::test]
async fn the_router_serves_the_config_update_path() {
    let harness = Harness::new(config_state());
    let request = Request::builder()
        .method("POST")
        .uri("/config/update")
        .body(Body::empty())
        .expect("the request builds");
    let response = routes(harness.state.clone())
        .oneshot(request)
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn the_audit_row_names_the_calling_key() {
    let harness = Harness::new(config_state());
    let caller = key_user(r#"{"api_key": "sk-raw", "user_id": "u9"}"#);
    let (status, _body) = send(&harness, Some(caller), r#"{"general_settings": {"a": 1}}"#).await;
    assert_eq!(status, StatusCode::OK);
    harness.spawner.run_all().await;
    let record = harness
        .repository
        .records()
        .into_iter()
        .next()
        .expect("one audit record");
    assert_eq!(record.changed_by.as_deref(), Some("u9"));
    let stored_key = record.changed_by_api_key.expect("the stored key");
    assert!(!stored_key.expose_secret().is_empty());
    assert_ne!(stored_key.expose_secret(), "sk-raw");
}
