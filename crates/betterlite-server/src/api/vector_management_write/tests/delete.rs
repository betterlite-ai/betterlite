#![cfg(test)]

use std::sync::Arc;

use axum::http::{HeaderMap, StatusCode};
use betterlite_config::sections::management_rbac::Settings as FeatureSettings;
use betterlite_models::decisions::Json;

use super::support::{
    CacheFailure, Events, Failure, FakeCache, FakeDatabase, Setup, World, build, capture_logs,
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
    let setup = Setup {
        database: Some(database.clone()),
        cache: Some(cache.clone()),
        registry: Some(Vec::new()),
        world: World::default(),
        privileges: false,
        settings: FeatureSettings::default(),
        events: events.clone(),
    };
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
    assert_cache_only_delete().await;
    assert_database_only_delete().await;
    assert_bare_delete().await;
}

async fn assert_cache_only_delete() {
    let events = Events::default();
    let cache = FakeCache::new(vec![memory_store("vs_m")]);
    let setup = Setup {
        database: Some(FakeDatabase::new(Vec::new())),
        cache: Some(cache.clone()),
        registry: Some(Vec::new()),
        world: World::default(),
        privileges: false,
        settings: FeatureSettings::default(),
        events: events.clone(),
    };
    let state = build(setup);
    let (status, _, _) = call_delete(
        &state,
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
        events.entries()
    );
    assert!(cache.stores().is_empty());
}

async fn assert_database_only_delete() {
    let events = Events::default();
    let database = FakeDatabase::new(vec![db_store("vs_b")]);
    let setup = Setup {
        database: Some(database.clone()),
        cache: Some(FakeCache::new(Vec::new())),
        registry: Some(Vec::new()),
        world: World::default(),
        privileges: false,
        settings: FeatureSettings::default(),
        events: events.clone(),
    };
    let state = build(setup);
    let (status, _, _) = call_delete(
        &state,
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
        events.entries()
    );
    assert!(database.rows().is_empty());
}

async fn assert_bare_delete() {
    let events = Events::default();
    let database = FakeDatabase::new(vec![db_store("vs_c")]);
    let setup = Setup {
        database: Some(database.clone()),
        cache: None,
        registry: None,
        world: World::default(),
        privileges: false,
        settings: FeatureSettings::default(),
        events: events.clone(),
    };
    let state = build(setup);
    let (status, _, _) = call_delete(
        &state,
        &internal("u1", None),
        r#"{"vector_store_id":"vs_c"}"#,
    )
    .await;
    assert_eq!(StatusCode::OK, status);
    assert_eq!(
        vec!["db:find".to_owned(), "db:delete".to_owned()],
        events.entries()
    );
    assert!(database.rows().is_empty());
}

#[tokio::test]
async fn delete_not_found_and_config_owned() {
    assert_missing_store_is_404().await;
    assert_config_owned_in_both_stores_is_400().await;
    assert_config_owned_in_the_cache_is_400().await;
}

async fn assert_missing_store_is_404() {
    let events = Events::default();
    let setup = Setup {
        database: Some(FakeDatabase::new(Vec::new())),
        cache: Some(FakeCache::new(Vec::new())),
        registry: Some(Vec::new()),
        world: World::default(),
        privileges: false,
        settings: FeatureSettings::default(),
        events: events.clone(),
    };
    let state = build(setup);
    let (status, _headers, body) = call_delete(&state, &internal("u1", None), DELETE_BODY).await;
    assert_eq!(StatusCode::NOT_FOUND, status);
    assert_eq!(
        "Vector store with ID vs_a not found",
        text(at(&body, "detail"))
    );
    assert_eq!(
        vec!["db:find".to_owned(), "cache:find".to_owned()],
        events.entries()
    );
}

async fn assert_config_owned_in_both_stores_is_400() {
    let events = Events::default();
    let setup = Setup {
        database: Some(FakeDatabase::new(vec![db_store("vs_c")])),
        cache: Some(FakeCache::new(vec![config_store("vs_c")])),
        registry: Some(vec![config_store("vs_c")]),
        world: World::default(),
        privileges: false,
        settings: FeatureSettings::default(),
        events: events.clone(),
    };
    let state = build(setup);
    let (status, _headers, body) = call_delete(
        &state,
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
        events.entries()
    );
}

// a config-owned id that only the registry holds answers the same 400
async fn assert_config_owned_in_the_cache_is_400() {
    let setup = Setup {
        database: Some(FakeDatabase::new(Vec::new())),
        cache: Some(FakeCache::new(vec![config_store("vs_d")])),
        registry: Some(vec![config_store("vs_d")]),
        world: World::default(),
        privileges: false,
        settings: FeatureSettings::default(),
        events: Events::default(),
    };
    let state = build(setup);
    let (status, _, _) = call_delete(
        &state,
        &internal("u1", None),
        r#"{"vector_store_id":"vs_d"}"#,
    )
    .await;
    assert_eq!(StatusCode::BAD_REQUEST, status);
}

#[tokio::test]
async fn delete_denied_access_keeps_both_stores() {
    assert_denied_access_keeps_both().await;
    assert_row_decides_access_when_both_exist().await;
}

async fn assert_denied_access_keeps_both() {
    let events = Events::default();
    let database = FakeDatabase::new(vec![team_store("vs_t", "t2")]);
    let setup = Setup {
        database: Some(database.clone()),
        cache: Some(FakeCache::new(Vec::new())),
        registry: Some(Vec::new()),
        world: World::default(),
        privileges: false,
        settings: FeatureSettings::default(),
        events: events.clone(),
    };
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
}

// the row decides access when both stores exist: a teamless cache copy does not grant it
async fn assert_row_decides_access_when_both_exist() {
    let setup = Setup {
        database: Some(FakeDatabase::new(vec![team_store("vs_t", "t2")])),
        cache: Some(FakeCache::new(vec![memory_store("vs_t")])),
        registry: Some(Vec::new()),
        world: World::default(),
        privileges: false,
        settings: FeatureSettings::default(),
        events: Events::default(),
    };
    let state = build(setup);
    let (status, _, _) = call_delete(
        &state,
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
    let setup = Setup {
        database: Some(database.clone()),
        cache: Some(cache.clone()),
        registry: Some(Vec::new()),
        world: World::default(),
        privileges: false,
        settings: FeatureSettings::default(),
        events: events.clone(),
    };
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
    let setup = Setup {
        database: Some(FakeDatabase::new(vec![db_store("vs_a")]).failing(Failure::Delete)),
        cache: Some(cache.clone()),
        registry: Some(Vec::new()),
        world: World::default(),
        privileges: false,
        settings: FeatureSettings::default(),
        events: events.clone(),
    };
    let state = build(setup);
    let (status, _, _) = call_delete(&state, &internal("u1", None), DELETE_BODY).await;
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
    let setup = Setup {
        database: None,
        cache: Some(cache.clone()),
        registry: Some(Vec::new()),
        world: World::default(),
        privileges: false,
        settings: FeatureSettings::default(),
        events: events.clone(),
    };
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
    let setup = Setup {
        database: Some(FakeDatabase::new(Vec::new())),
        cache: None,
        registry: None,
        world: World::default(),
        privileges: false,
        settings: FeatureSettings::default(),
        events: events.clone(),
    };
    let state = build(setup);
    let (status, _headers, body) = call_delete(&state, &internal("u1", None), "{}").await;
    assert_eq!(StatusCode::UNPROCESSABLE_ENTITY, status);
    assert!(matches!(field(&body, "detail"), Some(Json::List(_))));
    assert!(events.entries().is_empty());
}
