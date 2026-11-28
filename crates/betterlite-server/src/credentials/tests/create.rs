#![cfg(test)]

use axum::http::{Method, StatusCode};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::model_secrets::DEFAULT_USER_ID;
use betterlite_error::proxy_error_types::common::CommonProxyErrors;
use rstest::rstest;
use secrecy::SecretString;

use super::support::{
    ErrorReply, FakeLookup, FakeStore, ListReply, StoreFailure, SuccessReply, USER_ID, call,
    create_body, credential, credential_values, decrypt_text, payload, principal, registry, row,
    state, text,
};
use crate::auth::auth_key_lookup::master_key_context;
use crate::auth::auth_key_lookup::tests::caller_through_key_stage;

#[tokio::test]
async fn create_stores_encrypted_values_and_records_the_actor() {
    let registry = registry(Vec::new());
    let store = FakeStore::seeded(Vec::new());
    let caller = principal(Some(USER_ID));
    let reply = call(
        state(&registry, Some(&store), None),
        Method::POST,
        "/credentials",
        &create_body("openai", r#"{"api_key":"sk-abcdefgh"}"#),
        Some(&caller),
    )
    .await;
    assert_eq!(StatusCode::OK, reply.status);
    let success: SuccessReply = reply.json();
    assert!(success.success);
    assert_eq!("Credential created successfully", success.message);
    let writes = store.created();
    assert_eq!(1, writes.len());
    assert_eq!(Some(USER_ID.to_owned()), writes[0].1);
    let JsonData::String(sealed) = &writes[0].0.fields.credential_values.0["api_key"] else {
        panic!("the store receives ciphertext");
    };
    assert_eq!("sk-abcdefgh", decrypt_text(sealed));
    assert_eq!(
        credential_values(&[("api_key", text("sk-abcdefgh"))]),
        registry.get_credential_values("openai")
    );
}

#[tokio::test]
async fn create_records_the_key_user_as_actor() {
    let registry = registry(Vec::new());
    let store = FakeStore::seeded(Vec::new());
    let caller = caller_through_key_stage(r#"{"user_id":"u1"}"#).principal;
    let reply = call(
        state(&registry, Some(&store), None),
        Method::POST,
        "/credentials",
        &create_body("openai", r#"{"api_key":"sk-abcdefgh"}"#),
        Some(&caller),
    )
    .await;
    assert_eq!(StatusCode::OK, reply.status);
    let writes = store.created();
    assert_eq!(1, writes.len());
    assert_eq!(Some("u1".to_owned()), writes[0].1);
}

#[tokio::test]
async fn create_records_the_master_key_admin_user_as_actor() {
    let registry = registry(Vec::new());
    let store = FakeStore::seeded(Vec::new());
    let master = SecretString::from("master".to_owned());
    let caller = master_key_context(Some(&master), &master)
        .expect("the master key authenticates")
        .principal;
    let reply = call(
        state(&registry, Some(&store), None),
        Method::POST,
        "/credentials",
        &create_body("openai", r#"{"api_key":"sk-abcdefgh"}"#),
        Some(&caller),
    )
    .await;
    assert_eq!(StatusCode::OK, reply.status);
    let writes = store.created();
    assert_eq!(1, writes.len());
    assert_eq!(Some(DEFAULT_USER_ID.to_owned()), writes[0].1);
}

#[tokio::test]
async fn create_lists_the_masked_credential_from_memory() {
    let registry = registry(Vec::new());
    let store = FakeStore::seeded(Vec::new());
    let state = state(&registry, Some(&store), None);
    let created = call(
        state.clone(),
        Method::POST,
        "/credentials",
        &create_body("openai", r#"{"api_key":"sk-abcdefgh"}"#),
        None,
    )
    .await;
    assert_eq!(StatusCode::OK, created.status);
    let reply = call(state, Method::GET, "/credentials", "", None).await;
    assert_eq!(StatusCode::OK, reply.status);
    let listed: ListReply = reply.json();
    assert!(listed.success);
    assert_eq!(1, listed.credentials.len());
    assert_eq!("openai", listed.credentials[0].name);
    assert_eq!(
        payload(&[("api_key", text("sk****gh"))]),
        listed.credentials[0].values
    );
}

#[tokio::test]
async fn create_from_a_model_id_takes_the_deployment_values() {
    let registry = registry(Vec::new());
    let store = FakeStore::seeded(Vec::new());
    let lookup = FakeLookup::serving(
        "dep-1",
        "gpt-4o",
        &[("api_key", text("sk-from-deployment"))],
    );
    let reply = call(
        state(&registry, Some(&store), Some(&lookup)),
        Method::POST,
        "/credentials",
        r#"{"credential_name":"m","credential_info":{},"credential_values":{"api_key":"sk-explicit"},"model_id":"dep-1"}"#,
        None,
    )
    .await;
    assert_eq!(StatusCode::OK, reply.status);
    assert_eq!(
        credential_values(&[("api_key", text("sk-from-deployment"))]),
        registry.get_credential_values("m")
    );
    let writes = store.created();
    let JsonData::String(sealed) = &writes[0].0.fields.credential_values.0["api_key"] else {
        panic!("the store receives ciphertext");
    };
    assert_eq!("sk-from-deployment", decrypt_text(sealed));
}

#[tokio::test]
async fn an_empty_model_id_uses_the_explicit_values() {
    let registry = registry(Vec::new());
    let store = FakeStore::seeded(Vec::new());
    let reply = call(
        state(&registry, Some(&store), None),
        Method::POST,
        "/credentials",
        r#"{"credential_name":"m","credential_info":{},"model_id":"","credential_values":{"api_key":"sk-explicit"}}"#,
        None,
    )
    .await;
    assert_eq!(StatusCode::OK, reply.status);
    assert_eq!(
        credential_values(&[("api_key", text("sk-explicit"))]),
        registry.get_credential_values("m")
    );
}

#[tokio::test]
async fn create_without_values_is_a_400() {
    let registry = registry(Vec::new());
    let store = FakeStore::seeded(Vec::new());
    let reply = call(
        state(&registry, Some(&store), None),
        Method::POST,
        "/credentials",
        r#"{"credential_name":"openai","credential_info":{}}"#,
        None,
    )
    .await;
    assert_eq!(StatusCode::BAD_REQUEST, reply.status);
    let body: ErrorReply = reply.json();
    assert_eq!("400", body.error.code);
    assert_eq!("internal_server_error", body.error.error_type);
    assert!(
        body.error
            .message
            .contains("Either credential_values or model_id must be set")
    );
    assert!(registry.credential_list().is_empty());
    assert!(store.names().is_empty());
}

#[tokio::test]
async fn a_duplicate_create_is_a_409_and_leaves_memory_alone() {
    let registry = registry(Vec::new());
    let store = FakeStore::seeded(Vec::new());
    store.fail(StoreFailure::Duplicate);
    let reply = call(
        state(&registry, Some(&store), None),
        Method::POST,
        "/credentials",
        &create_body("openai", r#"{"api_key":"sk-abcdefgh"}"#),
        None,
    )
    .await;
    assert_eq!(StatusCode::CONFLICT, reply.status);
    let body: ErrorReply = reply.json();
    assert_eq!("409", body.error.code);
    assert!(
        body.error
            .message
            .starts_with("Credential 'openai' already exists.")
    );
    assert!(registry.credential_list().is_empty());
    assert!(store.created().is_empty());
}

#[tokio::test]
async fn duplicate_create_preserves_the_existing_row_and_resident() {
    let original = credential(
        "openai",
        &[("team", text("ml"))],
        &[("api_key", text("old-secret"))],
    );
    let stored = row(&original);
    let registry = registry(vec![original.clone()]);
    let store = FakeStore::seeded(vec![stored.clone()]);
    let reply = call(
        state(&registry, Some(&store), None),
        Method::POST,
        "/credentials",
        &create_body("openai", r#"{"api_key":"replacement"}"#),
        None,
    )
    .await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    let error: ErrorReply = reply.json();
    assert_eq!(error.error.code, "409");
    assert_eq!(registry.credential_list(), vec![original]);
    assert_eq!(store.row("openai"), Some(stored));
    assert!(store.created().is_empty());
}

#[tokio::test]
async fn a_failed_create_is_a_500_and_leaves_memory_alone() {
    let registry = registry(Vec::new());
    let store = FakeStore::seeded(Vec::new());
    store.fail(StoreFailure::Create);
    let reply = call(
        state(&registry, Some(&store), None),
        Method::POST,
        "/credentials",
        &create_body("openai", r#"{"api_key":"sk-abcdefgh"}"#),
        None,
    )
    .await;
    assert_eq!(StatusCode::INTERNAL_SERVER_ERROR, reply.status);
    let body: ErrorReply = reply.json();
    assert_eq!("500", body.error.code);
    assert!(registry.credential_list().is_empty());
    assert!(store.created().is_empty());
}

#[tokio::test]
async fn a_create_without_a_database_is_a_500() {
    let registry = registry(Vec::new());
    let reply = call(
        state(&registry, None, None),
        Method::POST,
        "/credentials",
        &create_body("openai", r#"{"api_key":"sk-abcdefgh"}"#),
        None,
    )
    .await;
    assert_eq!(StatusCode::INTERNAL_SERVER_ERROR, reply.status);
    let body: ErrorReply = reply.json();
    assert_eq!(
        CommonProxyErrors::DbNotConnectedError.as_str(),
        body.error.message
    );
    assert_eq!("500", body.error.code);
    assert!(registry.credential_list().is_empty());
}

#[tokio::test]
async fn create_from_a_model_without_a_router_is_a_500() {
    let registry = registry(Vec::new());
    let store = FakeStore::seeded(Vec::new());
    let reply = call(
        state(&registry, Some(&store), None),
        Method::POST,
        "/credentials",
        r#"{"credential_name":"m","credential_info":{},"model_id":"dep-1"}"#,
        None,
    )
    .await;
    assert_eq!(StatusCode::INTERNAL_SERVER_ERROR, reply.status);
    let body: ErrorReply = reply.json();
    assert_eq!(
        "LLM router not found. Please ensure you have a valid router instance.",
        body.error.message
    );
    assert_eq!("500", body.error.code);
    assert!(store.created().is_empty());
}

// one byte past the 2 MiB body limit axum applies to a Json extractor
const LARGE_BODY: usize = 0x0020_0001;

#[rstest]
#[case::malformed_post(Method::POST, "/credentials", "{invalid".to_owned())]
#[case::large_post(Method::POST, "/credentials", " ".repeat(LARGE_BODY))]
#[case::malformed_patch(Method::PATCH, "/credentials/old", "{invalid".to_owned())]
#[case::large_patch(Method::PATCH, "/credentials/old", " ".repeat(LARGE_BODY))]
#[tokio::test]
async fn invalid_bodies_return_json_errors(
    #[case] method: Method,
    #[case] uri: &str,
    #[case] request: String,
) {
    let registry = registry(Vec::new());
    let store = FakeStore::seeded(Vec::new());
    let reply = call(
        state(&registry, Some(&store), None),
        method,
        uri,
        &request,
        None,
    )
    .await;
    assert_eq!(StatusCode::BAD_REQUEST, reply.status);
    let error: ErrorReply = reply.json();
    assert_eq!(error.error.code, "400");
    assert_eq!(error.error.error_type, "internal_server_error");
    assert!(store.created().is_empty());
}
