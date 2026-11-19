#![cfg(test)]

use std::sync::Arc;

use axum::http::{Method, StatusCode};
use betterlite_error::proxy_error_types::common::CommonProxyErrors;

use super::super::registry::CredentialRegistry;
use super::support::{
    ErrorReply, FakeStore, StoreFailure, SuccessReply, call, credential, registry, row, sealed,
    state, text,
};

fn seeded() -> (Arc<CredentialRegistry>, Arc<FakeStore>) {
    let registry = registry(vec![credential(
        "azure/prod",
        &[],
        &[("api_key", text("sk-abcdefgh"))],
    )]);
    let store = FakeStore::seeded(vec![row(&credential(
        "azure/prod",
        &[],
        &[("api_key", text(&sealed("sk-abcdefgh")))],
    ))]);
    (registry, store)
}

#[tokio::test]
async fn delete_removes_the_row_and_the_memory_entry() {
    let (registry, store) = seeded();
    let reply = call(
        state(&registry, Some(&store), None),
        Method::DELETE,
        "/credentials/azure/prod",
        "",
        None,
    )
    .await;
    assert_eq!(StatusCode::OK, reply.status);
    let success: SuccessReply = reply.json();
    assert!(success.success);
    assert_eq!("Credential deleted successfully", success.message);
    assert_eq!(vec!["azure/prod".to_owned()], store.deleted());
    assert!(store.row("azure/prod").is_none());
    assert!(registry.find_credential("azure/prod").is_none());
}

#[tokio::test]
async fn a_second_delete_is_a_404_with_its_name() {
    let (registry, store) = seeded();
    let state = state(&registry, Some(&store), None);
    let first = call(
        state.clone(),
        Method::DELETE,
        "/credentials/azure/prod",
        "",
        None,
    )
    .await;
    assert_eq!(StatusCode::OK, first.status);
    let second = call(state, Method::DELETE, "/credentials/azure/prod", "", None).await;
    assert_eq!(StatusCode::NOT_FOUND, second.status);
    let body: ErrorReply = second.json();
    assert_eq!(
        "Credential not found. Got credential name: azure/prod",
        body.error.message
    );
    assert_eq!("404", body.error.code);
    assert!(registry.find_credential("azure/prod").is_none());
}

#[tokio::test]
async fn a_failed_delete_is_a_500_and_leaves_memory_alone() {
    let (registry, store) = seeded();
    store.fail(StoreFailure::Delete);
    let reply = call(
        state(&registry, Some(&store), None),
        Method::DELETE,
        "/credentials/azure/prod",
        "",
        None,
    )
    .await;
    assert_eq!(StatusCode::INTERNAL_SERVER_ERROR, reply.status);
    let body: ErrorReply = reply.json();
    assert_eq!("500", body.error.code);
    assert!(store.row("azure/prod").is_some());
    assert!(registry.find_credential("azure/prod").is_some());
}

#[tokio::test]
async fn delete_without_a_database_is_a_500() {
    let (registry, _store) = seeded();
    let reply = call(
        state(&registry, None, None),
        Method::DELETE,
        "/credentials/azure/prod",
        "",
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
    assert!(registry.find_credential("azure/prod").is_some());
}
