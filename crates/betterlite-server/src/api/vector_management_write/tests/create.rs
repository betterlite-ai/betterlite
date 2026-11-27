#![cfg(test)]

use std::sync::Arc;

use axum::http::{HeaderMap, StatusCode};
use betterlite_models::decisions::Json;

use super::support::{
    CacheFailure, Events, Failure, FakeCache, FakeDatabase, Setup, build, capture_logs,
    config_store, db_store, internal,
};
use super::{Column, at, content_type, decoded, field, field_count, stored_column, text};
use crate::api::vector_management_boundaries::redaction::REDACTED_BY_LITELM;
use crate::api::vector_management_write::NEW_PATH;
use crate::api::vector_management_write::state::VectorManagementWriteState;

type WriteState = Arc<VectorManagementWriteState<FakeDatabase>>;

const REGISTER_BODY: &str = r#"{"vector_store_id":"vs_a","custom_llm_provider":"openai","litellm_params":{"litellm_embedding_model":"embed","api_key":"secret"},"team_id":"forged","user_id":"forged"}"#;

#[tokio::test]
async fn register_store_preserves_raw_embedding_reference() {
    let events = Events::default();
    let database = FakeDatabase::new(Vec::new());
    let cache = FakeCache::new(Vec::new());
    let mut setup = Setup::default();
    setup.database = Some(database.clone());
    setup.cache = Some(cache.clone());
    setup.registry = Some(vec![config_store("vs_owned")]);
    setup.events = events.clone();
    let state = build(setup);
    let (status, headers, body) = call_new(&state, REGISTER_BODY).await;
    assert_eq!(StatusCode::OK, status);
    assert_eq!(Some("application/json"), content_type(&headers));
    assert_eq!("success", text(at(&body, "status")));
    assert_eq!(
        "Vector store vs_a created successfully",
        text(at(&body, "message"))
    );
    let created = at(&body, "vector_store");
    assert_eq!("vs_a", text(at(created, "vector_store_id")));
    assert_eq!("openai", text(at(created, "custom_llm_provider")));
    assert_eq!(Some(&Json::Null), field(created, "vector_store_name"));
    assert_eq!(Some(&Json::Null), field(created, "litellm_credential_name"));
    // the body's team_id and user_id are ignored: the created row belongs to the authenticated caller
    assert_eq!("t", text(at(created, "team_id")));
    assert_eq!("u", text(at(created, "user_id")));
    // the response carries the raw embedding reference with every credential masked
    let response_params = decoded(at(created, "litellm_params"));
    assert_eq!(
        Some(&Json::Text(REDACTED_BY_LITELM.to_owned())),
        field(&response_params, "api_key")
    );
    assert_eq!(
        Some(&Json::Text("embed".to_owned())),
        field(&response_params, "litellm_embedding_model")
    );
    assert_eq!(
        Some(&Json::Bool(false)),
        field(&response_params, "use_litellm_proxy")
    );
    // the row stores the caller's identity and the raw reference, and no resolved credentials
    let record = database.creates().pop().expect("the insert ran");
    assert_eq!(Some("t".to_owned()), record.team_id);
    assert_eq!(Some("u".to_owned()), record.user_id);
    let stored: Json = serde_json::from_str(&record.litellm_params).expect("params are JSON text");
    assert_eq!(
        Some(&Json::Text("embed".to_owned())),
        field(&stored, "litellm_embedding_model")
    );
    assert_eq!(
        Some(&Json::Text("secret".to_owned())),
        field(&stored, "api_key")
    );
    assert_eq!(
        Some(&Json::Bool(false)),
        field(&stored, "use_litellm_proxy")
    );
    assert_eq!(
        Some(&Json::Bool(false)),
        field(&stored, "merge_reasoning_content_in_choices")
    );
    // the registry copy keeps the credentials the database holds
    let cached = cache.stores().pop().expect("the registry holds the store");
    assert_eq!(Some("t".to_owned()), cached.team_id);
    assert_eq!(
        Some(&Json::Text("secret".to_owned())),
        field(&stored_column(&cached, Column::Params), "api_key")
    );
    assert_eq!(
        vec![
            "db:find".to_owned(),
            "db:create".to_owned(),
            "cache:add".to_owned(),
        ],
        events.entries()
    );
}

#[tokio::test]
async fn create_requires_id_and_provider() {
    let events = Events::default();
    let mut setup = Setup::default();
    setup.database = Some(FakeDatabase::new(Vec::new()));
    setup.events = events.clone();
    let state = build(setup);
    assert_required(&state, r#"{"custom_llm_provider":"openai"}"#).await;
    assert_required(
        &state,
        r#"{"vector_store_id":"","custom_llm_provider":"openai"}"#,
    )
    .await;
    assert_required(&state, r#"{"vector_store_id":"vs_a"}"#).await;
    assert_required(
        &state,
        r#"{"vector_store_id":"vs_a","custom_llm_provider":""}"#,
    )
    .await;
    assert!(events.entries().is_empty());
}

async fn assert_required(state: &WriteState, payload: &str) {
    let (status, _headers, body) = call_new(state, payload).await;
    assert_eq!(StatusCode::BAD_REQUEST, status);
    assert_eq!(
        "vector_store_id and custom_llm_provider are required",
        text(at(&body, "detail"))
    );
    assert_eq!(None, field(&body, "type"));
}

#[tokio::test]
async fn create_refuses_config_owned_id_before_the_database() {
    let events = Events::default();
    let database = FakeDatabase::new(Vec::new());
    let mut setup = Setup::default();
    setup.database = Some(database.clone());
    setup.registry = Some(vec![config_store("vs_c")]);
    setup.events = events.clone();
    let state = build(setup);
    let (status, _headers, body) = call_new(
        &state,
        r#"{"vector_store_id":"vs_c","custom_llm_provider":"openai"}"#,
    )
    .await;
    assert_eq!(StatusCode::BAD_REQUEST, status);
    let detail = at(&body, "detail");
    assert_eq!(2, field_count(detail));
    assert_eq!("vs_c", text(at(detail, "vector_store_id")));
    assert!(text(at(detail, "error")).contains("is defined in the config file"));
    assert!(events.entries().is_empty());
    assert!(database.creates().is_empty());
}

#[tokio::test]
async fn create_refuses_duplicate_id() {
    let events = Events::default();
    let database = FakeDatabase::new(vec![db_store("vs_a")]);
    let mut setup = Setup::default();
    setup.database = Some(database.clone());
    setup.registry = Some(Vec::new());
    setup.events = events.clone();
    let state = build(setup);
    let (status, _headers, body) = call_new(
        &state,
        r#"{"vector_store_id":"vs_a","custom_llm_provider":"openai"}"#,
    )
    .await;
    assert_eq!(StatusCode::BAD_REQUEST, status);
    assert_eq!(
        "Vector store with ID vs_a already exists",
        text(at(&body, "detail"))
    );
    assert_eq!(vec!["db:find".to_owned()], events.entries());
    assert!(database.creates().is_empty());
}

#[tokio::test]
async fn create_optional_database_values() {
    let events = Events::default();
    let database = FakeDatabase::new(Vec::new());
    let cache = FakeCache::new(Vec::new());
    let mut setup = Setup::default();
    setup.database = Some(database.clone());
    setup.cache = Some(cache.clone());
    setup.registry = Some(Vec::new());
    setup.events = events;
    let state = build(setup);
    let (status, _headers, body) = call_new(
        &state,
        r#"{"vector_store_id":"vs_a","custom_llm_provider":"openai","vector_store_name":"","vector_store_description":"","vector_store_metadata":{},"litellm_params":null,"litellm_credential_name":""}"#,
    )
    .await;
    assert_eq!(StatusCode::OK, status);
    let record = database.creates().pop().expect("the insert ran");
    assert_eq!(Some(String::new()), record.vector_store_name);
    assert_eq!(Some(String::new()), record.vector_store_description);
    assert_eq!(Some(String::from("{}")), record.vector_store_metadata);
    assert_eq!(Some(String::new()), record.litellm_credential_name);
    // null params still store the empty dict
    assert_eq!("{}", record.litellm_params);
    assert_eq!(None, record.team_id);
    assert_eq!(Some("u1".to_owned()), record.user_id);
    let document = record.document();
    assert_eq!(
        vec![
            "custom_llm_provider",
            "litellm_credential_name",
            "litellm_params",
            "user_id",
            "vector_store_description",
            "vector_store_id",
            "vector_store_metadata",
            "vector_store_name",
        ],
        document.keys().map(String::as_str).collect::<Vec<_>>()
    );
    assert_eq!(
        Some(&Json::Text(String::new())),
        field(at(&body, "vector_store"), "vector_store_name")
    );
    // omitted optional fields never reach the create data, while the params column still exists
    let (status, _headers, _body) = call_new(
        &state,
        r#"{"vector_store_id":"vs_b","custom_llm_provider":"openai"}"#,
    )
    .await;
    assert_eq!(StatusCode::OK, status);
    let record = database.creates().pop().expect("the insert ran");
    assert_eq!(
        vec![
            "custom_llm_provider",
            "litellm_params",
            "user_id",
            "vector_store_id",
        ],
        record
            .document()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>()
    );
    // a non-dict metadata is dropped after the record accepts it
    let (status, _headers, _body) = call_new(
        &state,
        r#"{"vector_store_id":"vs_c","custom_llm_provider":"openai","vector_store_metadata":"text"}"#,
    )
    .await;
    assert_eq!(StatusCode::OK, status);
    let record = database.creates().pop().expect("the insert ran");
    assert_eq!(None, record.vector_store_metadata);
    assert!(!record.document().contains_key("vector_store_metadata"));
    // an empty params object also stores the empty dict
    let (status, _headers, _body) = call_new(
        &state,
        r#"{"vector_store_id":"vs_d","custom_llm_provider":"openai","litellm_params":{}}"#,
    )
    .await;
    assert_eq!(StatusCode::OK, status);
    let record = database.creates().pop().expect("the insert ran");
    assert_eq!("{}", record.litellm_params);
}

#[tokio::test]
async fn create_without_a_database_answers_500() {
    let events = Events::default();
    let mut setup = Setup::default();
    setup.cache = Some(FakeCache::new(Vec::new()));
    setup.registry = Some(Vec::new());
    setup.events = events.clone();
    let state = build(setup);
    let (status, _headers, body) = call_new(&state, REGISTER_BODY).await;
    assert_eq!(StatusCode::INTERNAL_SERVER_ERROR, status);
    assert_eq!("Database not connected", text(at(&body, "detail")));
    assert!(events.entries().is_empty());
}

#[tokio::test]
async fn create_database_failure_writes_nothing_to_the_cache() {
    let events = Events::default();
    let cache = FakeCache::new(Vec::new());
    let mut setup = Setup::default();
    setup.database = Some(FakeDatabase::new(Vec::new()).failing(Failure::Create));
    setup.cache = Some(cache.clone());
    setup.registry = Some(Vec::new());
    setup.events = events.clone();
    let state = build(setup);
    let (status, _headers, _body) = call_new(&state, REGISTER_BODY).await;
    assert_eq!(StatusCode::INTERNAL_SERVER_ERROR, status);
    assert_eq!(
        vec!["db:find".to_owned(), "db:create".to_owned()],
        events.entries()
    );
    assert!(cache.stores().is_empty());
}

#[tokio::test]
async fn create_cache_failure_keeps_the_database_row() {
    let events = Events::default();
    let database = FakeDatabase::new(Vec::new());
    let cache = FakeCache::new(Vec::new()).failing(CacheFailure::Add);
    let mut setup = Setup::default();
    setup.database = Some(database.clone());
    setup.cache = Some(cache);
    setup.registry = Some(Vec::new());
    setup.events = events.clone();
    let state = build(setup);
    let (logs, _guard) = capture_logs(tracing::Level::ERROR);
    let (status, _headers, body) = call_new(&state, REGISTER_BODY).await;
    assert_eq!(StatusCode::INTERNAL_SERVER_ERROR, status);
    assert_eq!("simulated cache failure", text(at(&body, "detail")));
    assert_eq!(1, database.rows().len());
    assert!(
        logs.contents()
            .contains("Error creating vector store: simulated cache failure")
    );
}

#[tokio::test]
async fn create_logs_the_created_id() {
    let mut setup = Setup::default();
    setup.database = Some(FakeDatabase::new(Vec::new()));
    setup.cache = Some(FakeCache::new(Vec::new()));
    setup.registry = Some(Vec::new());
    let state = build(setup);
    let (logs, _guard) = capture_logs(tracing::Level::INFO);
    let (status, _headers, _body) = call_new(&state, REGISTER_BODY).await;
    assert_eq!(StatusCode::OK, status);
    assert!(
        logs.contents()
            .contains("Vector store vs_a created in database successfully")
    );
    assert!(!logs.contents().contains("secret"));
}

// a create call from the DoD's caller: user u under team t
async fn call_new(state: &WriteState, payload: &str) -> (StatusCode, HeaderMap, Json) {
    super::support::call(state, &internal("u", Some("t")), NEW_PATH, payload).await
}
