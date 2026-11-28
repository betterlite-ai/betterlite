#![cfg(test)]

use std::sync::Arc;

use axum::http::StatusCode;
use serde_json::Value;

use super::{CacheSettingsReadState, router};
use crate::auth::auth_key_lookup::tests::master_caller;
use crate::cache_settings::REDACTED_VALUE;
use crate::cache_settings_read::test_support::{
    FakeCacheConfigStore, StoreReply, SwitchableEnvironment, caller, cipher_settings, get, lookup,
    parse,
};

fn catalog_field<'body>(body: &'body Value, name: &str) -> &'body Value {
    body.get("fields")
        .and_then(Value::as_array)
        .expect("the fields list")
        .iter()
        .find(|field| field.get("field_name").and_then(Value::as_str) == Some(name))
        .expect("the catalog field")
}

fn field_value(body: &Value, name: &str) -> Value {
    catalog_field(body, name)
        .get("field_value")
        .cloned()
        .expect("the field_value member")
}

fn current_values(body: &Value) -> Value {
    body.get("current_values")
        .cloned()
        .expect("the current_values member")
}

#[tokio::test]
async fn the_response_carries_the_fields_without_values() {
    let state = CacheSettingsReadState::new(None, cipher_settings(None), lookup(&[]));
    let (status, text) = get(router(state), Some(master_caller())).await;
    assert_eq!(status, StatusCode::OK);
    let body = parse(&text);
    let fields = body
        .get("fields")
        .and_then(Value::as_array)
        .expect("the fields list");
    assert_eq!(fields.len(), 27);
    assert_eq!(
        fields
            .first()
            .and_then(|field| field.get("field_name"))
            .and_then(Value::as_str),
        Some("redis_type")
    );
    assert_eq!(
        fields
            .last()
            .and_then(|field| field.get("field_name"))
            .and_then(Value::as_str),
        Some("aws_iam_serverless")
    );
    assert!(
        fields
            .iter()
            .all(|field| field.as_object().is_some_and(|members| members.len() == 9))
    );
    assert!(
        fields
            .iter()
            .all(|field| field.get("field_value") == Some(&Value::Null))
    );
    assert_eq!(current_values(&body), parse("{}"));
}

#[tokio::test]
async fn the_response_carries_the_redis_type_descriptions() {
    let state = CacheSettingsReadState::new(None, cipher_settings(None), lookup(&[]));
    let (status, text) = get(router(state), Some(master_caller())).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        parse(&text).get("redis_type_descriptions"),
        Some(&parse(
            r#"{
                "node": "Standard Redis node/single instance",
                "cluster": "Redis Cluster mode for high availability and horizontal scaling",
                "sentinel": "Redis Sentinel mode for high availability with automatic failover"
            }"#
        ))
    );
}

#[tokio::test]
async fn without_a_caller_the_route_answers_401() {
    let state = CacheSettingsReadState::new(None, cipher_settings(None), lookup(&[]));
    let (status, text) = get(router(state), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        parse(&text),
        parse(
            r#"{"error": {"message": "the API key is not valid", "type": "auth_error", "param": "None", "code": "401"}}"#
        )
    );
}

// LiteLLM refuses a non-admin key in the auth dependency's route check, so the handler itself must not repeat it.
#[tokio::test]
async fn the_route_adds_no_role_check_of_its_own() {
    let state = CacheSettingsReadState::new(None, cipher_settings(None), lookup(&[]));
    let user = caller("key-user", Vec::new());
    let (status, text) = get(router(state), Some(user)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(current_values(&parse(&text)), parse("{}"));
}

#[tokio::test]
async fn the_environment_is_read_on_every_request() {
    let environment = Arc::new(SwitchableEnvironment::new(&[("REDIS_HOST", "first-host")]));
    let state = CacheSettingsReadState::new(
        None,
        cipher_settings(None),
        Arc::<SwitchableEnvironment>::clone(&environment),
    );
    let application = router(state);
    let (first_status, first_text) = get(application.clone(), Some(master_caller())).await;
    assert_eq!(first_status, StatusCode::OK);
    assert_eq!(
        current_values(&parse(&first_text)),
        parse(r#"{"host": "first-host", "type": "redis", "redis_type": "node"}"#)
    );
    environment.replace(&[("REDIS_HOST", "second-host")]);
    let (second_status, second_text) = get(application, Some(master_caller())).await;
    assert_eq!(second_status, StatusCode::OK);
    assert_eq!(
        current_values(&parse(&second_text)),
        parse(r#"{"host": "second-host", "type": "redis", "redis_type": "node"}"#)
    );
}

#[tokio::test]
async fn stored_values_reach_the_fields_redacted() {
    let stored = parse(
        r#"{"type": "redis", "host": "stored-host", "password": "secret", "sentinel_nodes": [["localhost", 26379]]}"#,
    );
    let state = CacheSettingsReadState::new(
        Some(Arc::new(FakeCacheConfigStore::new(StoreReply::Settings(
            stored,
        )))),
        cipher_settings(None),
        lookup(&[]),
    );
    let (status, text) = get(router(state), Some(master_caller())).await;
    assert_eq!(status, StatusCode::OK);
    let body = parse(&text);
    assert_eq!(
        current_values(&body),
        parse(
            r#"{
                "type": "redis",
                "host": "stored-host",
                "password": "***REDACTED***",
                "sentinel_nodes": [["localhost", 26379]],
                "redis_type": "sentinel"
            }"#
        )
    );
    assert_eq!(
        field_value(&body, "password"),
        Value::String(REDACTED_VALUE.to_owned())
    );
    assert_eq!(
        field_value(&body, "redis_type"),
        Value::String("sentinel".to_owned())
    );
    assert_eq!(
        field_value(&body, "host"),
        Value::String("stored-host".to_owned())
    );
    assert_eq!(field_value(&body, "url"), Value::Null);
}
