use std::sync::Arc;

use axum::http::{Method, StatusCode};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_error::proxy_error_types::common::CommonProxyErrors;

use super::super::registry::CredentialRegistry;
use super::support::{
    ErrorReply, FakeLookup, FakeStore, StoreFailure, SuccessReply, USER_ID, call, credential,
    decrypt_text, patch_body, payload, principal, registry, sealed, state, text,
};

fn seeded() -> (Arc<CredentialRegistry>, Arc<FakeStore>) {
    let registry = registry(vec![credential(
        "openai",
        &[("team", text("ml"))],
        &[
            ("api_key", text("old-secret")),
            ("api_base", text("https://a")),
        ],
    )]);
    let store = FakeStore::seeded(vec![credential(
        "openai",
        &[("team", text("ml"))],
        &[
            ("api_key", text(&sealed("old-secret"))),
            ("api_base", text("https://a")),
        ],
    )]);
    (registry, store)
}

#[tokio::test]
async fn patch_encrypts_only_the_supplied_values() {
    let (registry, store) = seeded();
    let caller = principal(Some(USER_ID));
    let reply = call(
        state(&registry, Some(&store), None),
        Method::PATCH,
        "/credentials/openai",
        &patch_body("", "{}", r#"{"api_key":"new-secret"}"#),
        Some(&caller),
    )
    .await;
    assert_eq!(StatusCode::OK, reply.status);
    let success: SuccessReply = reply.json();
    assert!(success.success);
    assert_eq!("Credential updated successfully", success.message);
    let writes = store.updated();
    assert_eq!(1, writes.len());
    assert_eq!("openai", writes[0].0);
    assert_eq!(Some(USER_ID.to_owned()), writes[0].2);
    let values = &writes[0].1.fields.credential_values.0;
    let JsonData::String(sealed_value) = &values["api_key"] else {
        panic!("the store receives ciphertext");
    };
    assert_eq!("new-secret", decrypt_text(sealed_value));
    assert_eq!(text("https://a"), values["api_base"]);
    assert_eq!(
        payload(&[("team", text("ml"))]),
        writes[0].1.fields.base.credential_info
    );
    assert_eq!(
        payload(&[
            ("api_key", text("new-secret")),
            ("api_base", text("https://a")),
        ]),
        registry.get_credential_values("openai")
    );
}

#[tokio::test]
async fn patch_of_a_database_only_credential_does_not_warm_memory() {
    let registry = registry(Vec::new());
    let store = FakeStore::seeded(vec![credential(
        "openai",
        &[],
        &[("api_key", text(&sealed("old-secret")))],
    )]);
    let reply = call(
        state(&registry, Some(&store), None),
        Method::PATCH,
        "/credentials/openai",
        &patch_body("", "{}", r#"{"api_key":"new-secret"}"#),
        None,
    )
    .await;
    assert_eq!(StatusCode::OK, reply.status);
    assert_eq!(1, store.updated().len());
    assert!(registry.find_credential("openai").is_none());
    assert!(registry.credential_list().is_empty());
}

#[tokio::test]
async fn patch_rename_writes_under_the_original_name() {
    let (registry, store) = seeded();
    let reply = call(
        state(&registry, Some(&store), None),
        Method::PATCH,
        "/credentials/openai",
        &patch_body("renamed", "{}", r#"{"api_key":"new-secret"}"#),
        None,
    )
    .await;
    assert_eq!(StatusCode::OK, reply.status);
    assert_eq!("openai", store.updated()[0].0);
    assert!(store.row("openai").is_none());
    let renamed = store.row("renamed").expect("the row is renamed");
    assert_eq!("renamed", renamed.fields.base.credential_name);
    assert!(registry.find_credential("openai").is_none());
    assert_eq!(
        payload(&[
            ("api_key", text("new-secret")),
            ("api_base", text("https://a")),
        ]),
        registry.get_credential_values("renamed")
    );
}

#[tokio::test]
async fn patch_of_a_missing_credential_is_a_404() {
    let registry = registry(Vec::new());
    let store = FakeStore::seeded(Vec::new());
    let reply = call(
        state(&registry, Some(&store), None),
        Method::PATCH,
        "/credentials/missing",
        &patch_body("", "{}", r#"{"api_key":"new-secret"}"#),
        None,
    )
    .await;
    assert_eq!(StatusCode::NOT_FOUND, reply.status);
    let body: ErrorReply = reply.json();
    assert_eq!("Credential not found in DB.", body.error.message);
    assert_eq!("404", body.error.code);
    assert!(store.updated().is_empty());
}

#[tokio::test]
async fn a_failed_update_is_a_500_and_leaves_memory_alone() {
    let (registry, store) = seeded();
    store.fail(StoreFailure::Update);
    let reply = call(
        state(&registry, Some(&store), None),
        Method::PATCH,
        "/credentials/openai",
        &patch_body("", "{}", r#"{"api_key":"new-secret"}"#),
        None,
    )
    .await;
    assert_eq!(StatusCode::INTERNAL_SERVER_ERROR, reply.status);
    let body: ErrorReply = reply.json();
    assert_eq!("500", body.error.code);
    assert!(store.updated().is_empty());
    assert_eq!(
        payload(&[
            ("api_key", text("old-secret")),
            ("api_base", text("https://a")),
        ]),
        registry.get_credential_values("openai")
    );
    let stored = store.row("openai").expect("the row stays");
    let JsonData::String(sealed_value) = &stored.fields.credential_values.0["api_key"] else {
        panic!("the row stays sealed");
    };
    assert_eq!("old-secret", decrypt_text(sealed_value));
}

#[tokio::test]
async fn patch_without_a_database_is_a_500() {
    let (registry, _store) = seeded();
    let reply = call(
        state(&registry, None, None),
        Method::PATCH,
        "/credentials/openai",
        &patch_body("", "{}", r#"{"api_key":"new-secret"}"#),
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
    assert_eq!(
        payload(&[
            ("api_key", text("old-secret")),
            ("api_base", text("https://a")),
        ]),
        registry.get_credential_values("openai")
    );
}

#[tokio::test]
async fn patch_drops_the_old_info_in_the_store_but_not_in_memory() {
    let (registry, store) = seeded();
    let reply = call(
        state(&registry, Some(&store), None),
        Method::PATCH,
        "/credentials/openai",
        &patch_body("", r#"{"owner":"x"}"#, "{}"),
        None,
    )
    .await;
    assert_eq!(StatusCode::OK, reply.status);
    let stored = store.row("openai").expect("the row stays");
    assert_eq!(
        payload(&[("owner", text("x"))]),
        stored.fields.base.credential_info
    );
    assert_eq!(
        payload(&[("team", text("ml")), ("owner", text("x"))]),
        registry
            .find_credential("openai")
            .expect("the credential stays")
            .fields
            .base
            .credential_info
    );
}

#[tokio::test]
async fn patch_keeps_the_old_info_that_carries_the_literal_key() {
    let registry = registry(vec![credential(
        "openai",
        &[
            ("credential_info", JsonData::Bool(true)),
            ("team", text("ml")),
        ],
        &[("api_key", text("old-secret"))],
    )]);
    let store = FakeStore::seeded(vec![credential(
        "openai",
        &[
            ("credential_info", JsonData::Bool(true)),
            ("team", text("ml")),
        ],
        &[("api_key", text(&sealed("old-secret")))],
    )]);
    let reply = call(
        state(&registry, Some(&store), None),
        Method::PATCH,
        "/credentials/openai",
        &patch_body("", r#"{"owner":"x"}"#, "{}"),
        None,
    )
    .await;
    assert_eq!(StatusCode::OK, reply.status);
    let stored = store.row("openai").expect("the row stays");
    assert_eq!(
        payload(&[
            ("credential_info", JsonData::Bool(true)),
            ("team", text("ml")),
            ("owner", text("x")),
        ]),
        stored.fields.base.credential_info
    );
}

#[tokio::test]
async fn patch_from_a_model_id_takes_the_deployment_values() {
    let (registry, store) = seeded();
    let lookup = FakeLookup::serving(
        "dep-1",
        "gpt-4o",
        &[("api_key", text("sk-from-deployment"))],
    );
    let reply = call(
        state(&registry, Some(&store), Some(&lookup)),
        Method::PATCH,
        "/credentials/openai",
        r#"{"credential_name":"","credential_info":{},"credential_values":null,"model_id":"dep-1"}"#,
        None,
    )
    .await;
    assert_eq!(StatusCode::OK, reply.status);
    assert_eq!(
        payload(&[
            ("api_key", text("sk-from-deployment")),
            ("api_base", text("https://a")),
        ]),
        registry.get_credential_values("openai")
    );
    let stored = store.row("openai").expect("the row stays");
    let JsonData::String(sealed_value) = &stored.fields.credential_values.0["api_key"] else {
        panic!("the store receives ciphertext");
    };
    assert_eq!("sk-from-deployment", decrypt_text(sealed_value));
}

#[tokio::test]
async fn patch_with_an_unknown_model_is_a_404() {
    let (registry, store) = seeded();
    let lookup = FakeLookup::serving("other", "gpt-4o", &[("api_key", text("sk-abcdefgh"))]);
    let reply = call(
        state(&registry, Some(&store), Some(&lookup)),
        Method::PATCH,
        "/credentials/openai",
        r#"{"credential_name":"","credential_info":{},"credential_values":null,"model_id":"dep-1"}"#,
        None,
    )
    .await;
    assert_eq!(StatusCode::NOT_FOUND, reply.status);
    let body: ErrorReply = reply.json();
    assert_eq!("Model not found", body.error.message);
    assert!(store.updated().is_empty());
    assert_eq!(
        payload(&[
            ("api_key", text("old-secret")),
            ("api_base", text("https://a")),
        ]),
        registry.get_credential_values("openai")
    );
}

#[tokio::test]
async fn patch_for_a_model_without_a_router_is_a_500() {
    let (registry, store) = seeded();
    let reply = call(
        state(&registry, Some(&store), None),
        Method::PATCH,
        "/credentials/openai",
        r#"{"credential_name":"","credential_info":{},"credential_values":null,"model_id":"dep-1"}"#,
        None,
    )
    .await;
    assert_eq!(StatusCode::INTERNAL_SERVER_ERROR, reply.status);
    let body: ErrorReply = reply.json();
    assert_eq!(
        "LLM router not found. Please ensure you have a valid router instance.",
        body.error.message
    );
    assert!(store.updated().is_empty());
}
