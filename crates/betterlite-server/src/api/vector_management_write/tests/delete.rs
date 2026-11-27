#![cfg(test)]

use std::sync::Arc;

use axum::http::{HeaderMap, StatusCode};
use betterlite_models::decisions::Json;

use super::support::{
    CacheFailure, Events, Failure, FakeCache, FakeDatabase, Setup, build, capture_logs,
    config_store, db_store, internal, memory_store, team_store,
};
use super::{at, field, field_count, text};
use crate::api::vector_management_write::DELETE_PATH;
use crate::api::vector_management_write::state::VectorManagementWriteState;
use crate::auth::auth_identity::AuthContext;

type WriteState = Arc<VectorManagementWriteState<FakeDatabase>>;

const DELETE_BODY: &str = r#"{"vector_store_id":"vs_a"}"#;

async fn call_delete(
    state: &WriteState,
    caller: &AuthContext,
    payload: &str,
) -> (StatusCode, HeaderMap, Json) {
    super::support::call(state, caller, DELETE_PATH, payload).await
}

#[tokio::test]
async fn delete_database_before_cache() {
    let events = Events::default();
    let database = FakeDatabase::new(vec![db_store("vs_a")]);
    let cache = FakeCache::new(vec![db_store("vs_a")]);
    let mut setup = Setup::default();
    setup.database = Some(database.clone());
    setup.cache = Some(cache.clone());
    setup.registry = Some(Vec::new());
    setup.events = events.clone();
    let state = build(setup);
    let (status, headers, body) = call_delete(&state, &internal("u1", None), DELETE_BODY).await;
    assert_eq!(StatusCode::OK, status);
    assert_eq!(Some("application/json"), super::content_type(&headers));
    assert_eq!("success", text(at(&body, "status")));
    assert_eq!(
        "Vector store vs_a deleted successfully",
        text(at(&body, "message"))
    );
    assert_eq!(None, field(&body, "vector_store"));
    assert_eq!(
        vec![
            "db:find".to_owned(),
            "cache:find".to_owned(),
            "db:delete".to_owned(),
            "cache:remove".to_owned(),
        ],
        events.entries()
    );
    assert!(database.rows().is_empty());
    assert!(cache.stores().is_empty());
}

#[tokio::test]
async fn delete_removes_whichever_store_exists() {
    let cache_only_events = Events::default();
    let database = FakeDatabase::new(Vec::new());
    let cache = FakeCache::new(vec![memory_store("vs_m")]);
    let mut setup = Setup::default();
    setup.database = Some(database.clone());
    setup.cache = Some(cache.clone());
    setup.registry = Some(Vec::new());
    setup.events = cache_only_events.clone();
    let cache_only = build(setup);
    let (status, _headers, _body) = call_delete(
        &cache_only,
        &internal("u1", None),
        r#"{"vector_store_id":"vs_m"}"#,
    )
    .await;
    assert_eq!(StatusCode::OK, status);
    assert_eq!(
        vec![
            "db:find".to_owned(),
            "cache:find".to_owned(),
            "cache:remove".to_owned(),
        ],
        cache_only_events.entries()
    );
    assert!(cache.stores().is_empty());
    // only the database holds it: the registry is untouched and no cache call is made without one
    let database_only_events = Events::default();
    let database = FakeDatabase::new(vec![db_store("vs_b")]);
    let mut setup = Setup::default();
    setup.database = Some(database.clone());
    setup.cache = Some(FakeCache::new(Vec::new()));
    setup.registry = Some(Vec::new());
    setup.events = database_only_events.clone();
    let database_only = build(setup);
    let (status, _headers, _body) = call_delete(
        &database_only,
        &internal("u1", None),
        r#"{"vector_store_id":"vs_b"}"#,
    )
    .await;
    assert_eq!(StatusCode::OK, status);
    assert_eq!(
        vec![
            "db:find".to_owned(),
            "cache:find".to_owned(),
            "db:delete".to_owned(),
        ],
        database_only_events.entries()
    );
    assert!(database.rows().is_empty());
    let bare_events = Events::default();
    let mut setup = Setup::default();
    setup.database = Some(FakeDatabase::new(vec![db_store("vs_c")]));
    setup.registry = None;
    setup.events = bare_events.clone();
    let bare = build(setup);
    let (status, _headers, _body) = call_delete(
        &bare,
        &internal("u1", None),
        r#"{"vector_store_id":"vs_c"}"#,
    )
    .await;
    assert_eq!(StatusCode::OK, status);
    assert_eq!(
        vec!["db:find".to_owned(), "db:delete".to_owned()],
        bare_events.entries()
    );
}

#[tokio::test]
async fn delete_not_found_and_config_owned() {
    let missing_events = Events::default();
    let mut setup = Setup::default();
    setup.database = Some(FakeDatabase::new(Vec::new()));
    setup.cache = Some(FakeCache::new(Vec::new()));
    setup.registry = Some(Vec::new());
    setup.events = missing_events.clone();
    let missing = build(setup);
    let (status, _headers, body) = call_delete(&missing, &internal("u1", None), DELETE_BODY).await;
    assert_eq!(StatusCode::NOT_FOUND, status);
    assert_eq!(
        "Vector store with ID vs_a not found",
        text(at(&body, "detail"))
    );
    assert_eq!(
        vec!["db:find".to_owned(), "cache:find".to_owned()],
        missing_events.entries()
    );
    let owned_events = Events::default();
    let mut setup = Setup::default();
    setup.database = Some(FakeDatabase::new(vec![db_store("vs_c")]));
    setup.cache = Some(FakeCache::new(vec![config_store("vs_c")]));
    setup.registry = Some(vec![config_store("vs_c")]);
    setup.events = owned_events.clone();
    let owned = build(setup);
    let (status, _headers, body) = call_delete(
        &owned,
        &internal("u1", None),
        r#"{"vector_store_id":"vs_c"}"#,
    )
    .await;
    assert_eq!(StatusCode::BAD_REQUEST, status);
    let detail = at(&body, "detail");
    assert_eq!(2, field_count(detail));
    assert_eq!("vs_c", text(at(detail, "vector_store_id")));
    assert_eq!(
        vec!["db:find".to_owned(), "cache:find".to_owned()],
        owned_events.entries()
    );
    // a config-owned id that only the registry holds answers the same 400
    let mut setup = Setup::default();
    setup.database = Some(FakeDatabase::new(Vec::new()));
    setup.cache = Some(FakeCache::new(vec![config_store("vs_d")]));
    setup.registry = Some(vec![config_store("vs_d")]);
    let memory_owned = build(setup);
    let (status, _headers, _body) = call_delete(
        &memory_owned,
        &internal("u1", None),
        r#"{"vector_store_id":"vs_d"}"#,
    )
    .await;
    assert_eq!(StatusCode::BAD_REQUEST, status);
}

#[tokio::test]
async fn delete_denied_access_keeps_both_stores() {
    let events = Events::default();
    let database = FakeDatabase::new(vec![team_store("vs_t", "t2")]);
    let mut setup = Setup::default();
    setup.database = Some(database.clone());
    setup.cache = Some(FakeCache::new(Vec::new()));
    setup.registry = Some(Vec::new());
    setup.events = events.clone();
    let state = build(setup);
    let (status, _headers, body) = call_delete(
        &state,
        &internal("u1", Some("t1")),
        r#"{"vector_store_id":"vs_t"}"#,
    )
    .await;
    assert_eq!(StatusCode::FORBIDDEN, status);
    assert_eq!(
        "Access denied: You do not have permission to delete this vector store",
        text(at(&body, "detail"))
    );
    assert_eq!(
        vec!["db:find".to_owned(), "cache:find".to_owned()],
        events.entries()
    );
    assert_eq!(1, database.rows().len());
    // the row decides access when both stores exist: a teamless cache copy does not grant it
    let mut setup = Setup::default();
    setup.database = Some(FakeDatabase::new(vec![team_store("vs_t", "t2")]));
    setup.cache = Some(FakeCache::new(vec![memory_store("vs_t")]));
    setup.registry = Some(Vec::new());
    let both = build(setup);
    let (status, _headers, _body) = call_delete(
        &both,
        &internal("u1", Some("t1")),
        r#"{"vector_store_id":"vs_t"}"#,
    )
    .await;
    assert_eq!(StatusCode::FORBIDDEN, status);
}

#[tokio::test]
async fn delete_cache_failure_keeps_the_database_deletion() {
    let events = Events::default();
    let database = FakeDatabase::new(vec![db_store("vs_a")]);
    let cache = FakeCache::new(vec![db_store("vs_a")]).failing(CacheFailure::Remove);
    let mut setup = Setup::default();
    setup.database = Some(database.clone());
    setup.cache = Some(cache.clone());
    setup.registry = Some(Vec::new());
    setup.events = events.clone();
    let state = build(setup);
    let (logs, _guard) = capture_logs(tracing::Level::ERROR);
    let (status, _headers, body) = call_delete(&state, &internal("u1", None), DELETE_BODY).await;
    assert_eq!(StatusCode::INTERNAL_SERVER_ERROR, status);
    assert_eq!("simulated cache failure", text(at(&body, "detail")));
    assert!(database.rows().is_empty());
    assert_eq!(1, cache.stores().len());
    assert_eq!(
        vec![
            "db:find".to_owned(),
            "cache:find".to_owned(),
            "db:delete".to_owned(),
            "cache:remove".to_owned(),
        ],
        events.entries()
    );
    assert!(
        logs.contents()
            .contains("Error deleting vector store: simulated cache failure")
    );
}

#[tokio::test]
async fn delete_database_failure_keeps_the_cache_copy() {
    let events = Events::default();
    let cache = FakeCache::new(vec![db_store("vs_a")]);
    let mut setup = Setup::default();
    setup.database = Some(FakeDatabase::new(vec![db_store("vs_a")]).failing(Failure::Delete));
    setup.cache = Some(cache.clone());
    setup.registry = Some(Vec::new());
    setup.events = events.clone();
    let state = build(setup);
    let (status, _headers, _body) = call_delete(&state, &internal("u1", None), DELETE_BODY).await;
    assert_eq!(StatusCode::INTERNAL_SERVER_ERROR, status);
    assert_eq!(1, cache.stores().len());
    assert_eq!(
        vec![
            "db:find".to_owned(),
            "cache:find".to_owned(),
            "db:delete".to_owned(),
        ],
        events.entries()
    );
}

#[tokio::test]
async fn delete_without_a_database_rejects_a_cache_only_store() {
    let events = Events::default();
    let cache = FakeCache::new(vec![memory_store("vs_m")]);
    let mut setup = Setup::default();
    setup.cache = Some(cache.clone());
    setup.registry = Some(Vec::new());
    setup.events = events.clone();
    let state = build(setup);
    let (status, _headers, body) = call_delete(
        &state,
        &internal("u1", None),
        r#"{"vector_store_id":"vs_m"}"#,
    )
    .await;
    assert_eq!(StatusCode::INTERNAL_SERVER_ERROR, status);
    assert_eq!("Database not connected", text(at(&body, "detail")));
    assert!(events.entries().is_empty());
    assert_eq!(1, cache.stores().len());
}

#[tokio::test]
async fn delete_requires_a_vector_store_id() {
    let events = Events::default();
    let mut setup = Setup::default();
    setup.database = Some(FakeDatabase::new(Vec::new()));
    setup.events = events.clone();
    let state = build(setup);
    let (status, _headers, body) = call_delete(&state, &internal("u1", None), "{}").await;
    assert_eq!(StatusCode::UNPROCESSABLE_ENTITY, status);
    assert!(matches!(field(&body, "detail"), Some(Json::List(_))));
    assert!(events.entries().is_empty());
}
