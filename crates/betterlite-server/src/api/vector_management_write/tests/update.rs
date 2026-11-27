#![cfg(test)]

use std::sync::Arc;

use axum::http::{HeaderMap, StatusCode};
use betterlite_models::decisions::Json;
use serde_json::Number;

use super::support::{
    CacheFailure, Events, FakeCache, FakeDatabase, Setup, UPDATED_AT, build, capture_logs,
    config_store, db_store, internal, team_store,
};
use super::{Column, at, decoded, field, stored_column, text};
use crate::api::vector_management_boundaries::redaction::REDACTED_BY_LITELM;
use crate::api::vector_management_write::UPDATE_PATH;
use crate::api::vector_management_write::rows::{FieldChange, VectorStoreChanges};
use crate::api::vector_management_write::state::VectorManagementWriteState;
use crate::auth::auth_identity::AuthContext;

type WriteState = Arc<VectorManagementWriteState<FakeDatabase>>;

async fn call_update(
    state: &WriteState,
    caller: &AuthContext,
    payload: &str,
) -> (StatusCode, HeaderMap, Json) {
    super::support::call(state, caller, UPDATE_PATH, payload).await
}

#[tokio::test]
async fn update_uses_exclude_unset() {
    let events = Events::default();
    let database = FakeDatabase::new(vec![db_store("vs_a")]);
    let cache = FakeCache::new(vec![db_store("vs_a")]);
    let mut setup = Setup::default();
    setup.database = Some(database.clone());
    setup.cache = Some(cache.clone());
    setup.registry = Some(Vec::new());
    setup.events = events.clone();
    let state = build(setup);
    let (status, _headers, body) = call_update(
        &state,
        &internal("u1", None),
        r#"{"vector_store_id":"vs_a","vector_store_description":null,"litellm_params":null,"vector_store_metadata":null}"#,
    )
    .await;
    assert_eq!(StatusCode::OK, status);
    assert_eq!(
        "Vector store vs_a updated successfully",
        text(at(&body, "message"))
    );
    let changes = database.updates().pop().expect("the update ran").1;
    assert_eq!(FieldChange::Null, changes.vector_store_description);
    assert_eq!(FieldChange::Null, changes.vector_store_metadata);
    assert_eq!(FieldChange::Omitted, changes.vector_store_name);
    assert_eq!(FieldChange::Omitted, changes.custom_llm_provider);
    // the id alone never reaches the written columns
    assert_eq!(
        vec!["custom_llm_provider", "vector_store_name"],
        written_names(&changes)
    );
    let row = database.rows().pop().expect("the row stays");
    assert_eq!(None, row.vector_store_description);
    assert_eq!(None, row.vector_store_metadata);
    assert_eq!(Some("db-vs_a".to_owned()), row.vector_store_name);
    assert_eq!(Some(UPDATED_AT.to_owned()), row.updated_at);
    // pinned LiteLLM's VectorStoreUpdateRequest has no litellm_params field, so the stored params survive
    let params = stored_column(&row, Column::Params);
    assert_eq!(
        Some(&Json::Text("db-secret-vs_a".to_owned())),
        field(&params, "api_key")
    );
    let cached = cache.stores().pop().expect("the registry row is replaced");
    assert_eq!(None, cached.vector_store_description);
    assert_eq!(
        Some(&Json::Text("db-secret-vs_a".to_owned())),
        field(&stored_column(&cached, Column::Params), "api_key")
    );
    let updated = at(&body, "vector_store");
    assert_eq!("vs_a", text(at(updated, "vector_store_id")));
    assert_eq!(
        Some(&Json::Null),
        field(updated, "vector_store_description")
    );
    assert_eq!(Some(&Json::Null), field(updated, "vector_store_metadata"));
    assert_eq!("db-vs_a", text(at(updated, "vector_store_name")));
    let response_params = decoded(at(updated, "litellm_params"));
    assert_eq!(
        Some(&Json::Text(REDACTED_BY_LITELM.to_owned())),
        field(&response_params, "api_key")
    );
    assert_eq!(
        Some(&Json::Text("us".to_owned())),
        field(&response_params, "region")
    );
    assert_eq!(
        vec![
            "db:find".to_owned(),
            "db:update".to_owned(),
            "cache:update".to_owned(),
        ],
        events.entries()
    );
}

fn written_names(changes: &VectorStoreChanges) -> Vec<&'static str> {
    let candidates = [
        ("custom_llm_provider", &changes.custom_llm_provider),
        ("vector_store_name", &changes.vector_store_name),
        (
            "vector_store_description",
            &changes.vector_store_description,
        ),
        ("vector_store_metadata", &changes.vector_store_metadata),
    ];
    candidates
        .into_iter()
        .filter(|(_, change)| !matches!(change, FieldChange::Omitted))
        .map(|(name, _)| name)
        .collect()
}

#[tokio::test]
async fn update_writes_the_present_values() {
    let database = FakeDatabase::new(vec![db_store("vs_a")]);
    let mut setup = Setup::default();
    setup.database = Some(database.clone());
    setup.cache = Some(FakeCache::new(vec![db_store("vs_a")]));
    setup.registry = Some(Vec::new());
    let state = build(setup);
    let (status, _headers, body) = call_update(
        &state,
        &internal("u1", None),
        r#"{"vector_store_id":"vs_a","custom_llm_provider":"bedrock","vector_store_name":"renamed","vector_store_metadata":{"x":1}}"#,
    )
    .await;
    assert_eq!(StatusCode::OK, status);
    let changes = database.updates().pop().expect("the update ran").1;
    assert_eq!(
        FieldChange::Value("bedrock".to_owned()),
        changes.custom_llm_provider
    );
    assert_eq!(
        FieldChange::Value("renamed".to_owned()),
        changes.vector_store_name
    );
    assert_eq!(FieldChange::Omitted, changes.vector_store_description);
    let FieldChange::Value(metadata) = changes.vector_store_metadata else {
        panic!("the metadata is written as JSON text");
    };
    let parsed: Json = serde_json::from_str(&metadata).expect("the metadata is JSON");
    assert_eq!(
        Some(&Json::Number(Number::from(1_i64))),
        field(&parsed, "x")
    );
    let row = database.rows().pop().expect("the row stays");
    assert_eq!(Some("bedrock".to_owned()), row.custom_llm_provider);
    assert_eq!(
        Some(&Json::Number(Number::from(1_i64))),
        field(&stored_column(&row, Column::Metadata), "x")
    );
    let response_metadata = decoded(at(at(&body, "vector_store"), "vector_store_metadata"));
    assert_eq!(
        Some(&Json::Number(Number::from(1_i64))),
        field(&response_metadata, "x")
    );
    // an explicit null provider is written as null
    let (status, _headers, _body) = call_update(
        &state,
        &internal("u1", None),
        r#"{"vector_store_id":"vs_a","custom_llm_provider":null}"#,
    )
    .await;
    assert_eq!(StatusCode::OK, status);
    let changes = database.updates().pop().expect("the update ran").1;
    assert_eq!(FieldChange::Null, changes.custom_llm_provider);
    assert_eq!(
        None,
        database
            .rows()
            .pop()
            .expect("the row stays")
            .custom_llm_provider
    );
    // an empty metadata object stores the empty JSON object
    let (status, _headers, _body) = call_update(
        &state,
        &internal("u1", None),
        r#"{"vector_store_id":"vs_a","vector_store_metadata":{}}"#,
    )
    .await;
    assert_eq!(StatusCode::OK, status);
    let changes = database.updates().pop().expect("the update ran").1;
    assert_eq!(
        FieldChange::Value(String::from("{}")),
        changes.vector_store_metadata
    );
}

#[tokio::test]
async fn update_errors_before_mutation() {
    let owned_events = Events::default();
    let mut setup = Setup::default();
    setup.database = Some(FakeDatabase::new(vec![db_store("vs_c")]));
    setup.cache = Some(FakeCache::new(vec![config_store("vs_c")]));
    setup.registry = Some(vec![config_store("vs_c")]);
    setup.events = owned_events.clone();
    let owned = build(setup);
    let (status, _headers, body) = call_update(
        &owned,
        &internal("u1", None),
        r#"{"vector_store_id":"vs_c","vector_store_description":"x"}"#,
    )
    .await;
    assert_eq!(StatusCode::BAD_REQUEST, status);
    assert_eq!("vs_c", text(at(at(&body, "detail"), "vector_store_id")));
    assert!(owned_events.entries().is_empty());
    let missing_events = Events::default();
    let mut setup = Setup::default();
    setup.database = Some(FakeDatabase::new(Vec::new()));
    setup.cache = Some(FakeCache::new(Vec::new()));
    setup.registry = Some(Vec::new());
    setup.events = missing_events.clone();
    let missing = build(setup);
    let (status, _headers, body) = call_update(
        &missing,
        &internal("u1", None),
        r#"{"vector_store_id":"vs_a","vector_store_description":"x"}"#,
    )
    .await;
    assert_eq!(StatusCode::NOT_FOUND, status);
    assert_eq!(
        "Vector store with ID vs_a not found",
        text(at(&body, "detail"))
    );
    assert_eq!(vec!["db:find".to_owned()], missing_events.entries());
    let mut setup = Setup::default();
    setup.database = Some(FakeDatabase::new(vec![team_store("vs_a", "t2")]));
    setup.cache = Some(FakeCache::new(Vec::new()));
    setup.registry = Some(Vec::new());
    let denied = build(setup);
    let (status, _headers, body) = call_update(
        &denied,
        &internal("u1", Some("t1")),
        r#"{"vector_store_id":"vs_a","vector_store_description":"x"}"#,
    )
    .await;
    assert_eq!(StatusCode::FORBIDDEN, status);
    assert_eq!(
        "Access denied: You do not have permission to access this vector store",
        text(at(&body, "detail"))
    );
    // a row that vanished between the fetch and the update answers the same 404
    let vanished_events = Events::default();
    let mut setup = Setup::default();
    setup.database = Some(FakeDatabase::new(vec![db_store("vs_a")]).missing_update());
    setup.cache = Some(FakeCache::new(Vec::new()));
    setup.registry = Some(Vec::new());
    setup.events = vanished_events.clone();
    let vanished = build(setup);
    let (status, _headers, body) = call_update(
        &vanished,
        &internal("u1", None),
        r#"{"vector_store_id":"vs_a","vector_store_description":"x"}"#,
    )
    .await;
    assert_eq!(StatusCode::NOT_FOUND, status);
    assert_eq!(
        "Vector store with ID vs_a not found",
        text(at(&body, "detail"))
    );
    assert_eq!(
        vec!["db:find".to_owned(), "db:update".to_owned()],
        vanished_events.entries()
    );
}

#[tokio::test]
async fn update_registry_failure_preserves_database_change() {
    let events = Events::default();
    let database = FakeDatabase::new(vec![db_store("vs_a")]);
    let cache = FakeCache::new(vec![db_store("vs_a")]).failing(CacheFailure::Update);
    let mut setup = Setup::default();
    setup.database = Some(database.clone());
    setup.cache = Some(cache.clone());
    setup.registry = Some(Vec::new());
    setup.events = events.clone();
    let state = build(setup);
    let (logs, _guard) = capture_logs(tracing::Level::ERROR);
    let (status, _headers, body) = call_update(
        &state,
        &internal("u1", None),
        r#"{"vector_store_id":"vs_a","vector_store_description":"new"}"#,
    )
    .await;
    assert_eq!(StatusCode::INTERNAL_SERVER_ERROR, status);
    assert_eq!("simulated cache failure", text(at(&body, "detail")));
    assert_eq!(
        Some("new".to_owned()),
        database
            .rows()
            .pop()
            .expect("the row stays")
            .vector_store_description
    );
    assert_eq!(
        Some("db-vs_a".to_owned()),
        cache
            .stores()
            .pop()
            .expect("the registry row stays")
            .vector_store_name
    );
    assert_eq!(
        vec![
            "db:find".to_owned(),
            "db:update".to_owned(),
            "cache:update".to_owned(),
        ],
        events.entries()
    );
    assert!(
        logs.contents()
            .contains("Error updating vector store: simulated cache failure")
    );
}

#[tokio::test]
async fn update_redacts_existing_credentials() {
    let database = FakeDatabase::new(vec![db_store("vs_a")]);
    let mut setup = Setup::default();
    setup.database = Some(database.clone());
    setup.cache = None;
    setup.registry = Some(Vec::new());
    let state = build(setup);
    let (status, _headers, body) = call_update(
        &state,
        &internal("u1", None),
        r#"{"vector_store_id":"vs_a","vector_store_description":"changed"}"#,
    )
    .await;
    assert_eq!(StatusCode::OK, status);
    let updated = at(&body, "vector_store");
    let response_params = decoded(at(updated, "litellm_params"));
    assert_eq!(
        Some(&Json::Text(REDACTED_BY_LITELM.to_owned())),
        field(&response_params, "api_key")
    );
    let row = database.rows().pop().expect("the row stays");
    let params = stored_column(&row, Column::Params);
    assert_eq!(
        Some(&Json::Text("db-secret-vs_a".to_owned())),
        field(&params, "api_key")
    );
}

#[tokio::test]
async fn update_without_a_database_answers_500() {
    let mut setup = Setup::default();
    setup.cache = Some(FakeCache::new(Vec::new()));
    setup.registry = Some(Vec::new());
    let state = build(setup);
    let (status, _headers, body) = call_update(
        &state,
        &internal("u1", None),
        r#"{"vector_store_id":"vs_a","vector_store_description":"x"}"#,
    )
    .await;
    assert_eq!(StatusCode::INTERNAL_SERVER_ERROR, status);
    assert_eq!("Database not connected", text(at(&body, "detail")));
}

#[tokio::test]
async fn update_requires_a_vector_store_id() {
    let mut setup = Setup::default();
    setup.database = Some(FakeDatabase::new(Vec::new()));
    let state = build(setup);
    let (status, _headers, body) = call_update(
        &state,
        &internal("u1", None),
        r#"{"vector_store_description":"x"}"#,
    )
    .await;
    assert_eq!(StatusCode::UNPROCESSABLE_ENTITY, status);
    assert!(matches!(field(&body, "detail"), Some(Json::List(_))));
}
