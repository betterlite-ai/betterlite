#![cfg(test)]

use std::sync::Arc;

use axum::http::{HeaderMap, StatusCode};
use betterlite_config::sections::management_rbac::Settings as FeatureSettings;
use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;
use rstest::rstest;
use serde_json::Number;

use super::support::{
    CacheFailure, Events, FakeCache, FakeDatabase, Setup, UPDATED_AT, World, build, capture_logs,
    config_store, db_store, internal, team_store,
};
use super::{Column, at, decoded, field, stored_column, text};
use crate::api::vector_management_boundaries::redaction::REDACTED_BY_LITELM;
use crate::api::vector_management_write::UPDATE_PATH;
use crate::api::vector_management_write::rows::VectorStoreChanges;
use crate::api::vector_management_write::state::VectorManagementWriteState;
use crate::auth::auth_identity::AuthContext;

type WriteState = Arc<VectorManagementWriteState<FakeDatabase>>;

const NULL_UPDATE: &str = r#"{"vector_store_id":"vs_a","vector_store_description":null,"litellm_params":null,"vector_store_metadata":null}"#;
const PRESENT_UPDATE: &str = r#"{"vector_store_id":"vs_a","custom_llm_provider":"bedrock","vector_store_name":"renamed","vector_store_metadata":{"x":1}}"#;

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
    let setup = Setup {
        database: Some(database.clone()),
        cache: Some(FakeCache::new(vec![db_store("vs_a")])),
        registry: Some(Vec::new()),
        world: World::default(),
        privileges: false,
        settings: FeatureSettings::default(),
        events: events.clone(),
    };
    let state = build(setup);
    let (status, _headers, body) = call_update(&state, &internal("u1", None), NULL_UPDATE).await;
    assert_eq!(StatusCode::OK, status);
    assert_eq!(
        "Vector store vs_a updated successfully",
        text(at(&body, "message"))
    );
    let changes = database.updates().pop().expect("the update ran").1;
    assert_eq!(Some(None), changes.vector_store_description);
    assert_eq!(Some(None), changes.vector_store_metadata);
    assert_eq!(None, changes.vector_store_name);
    assert_eq!(None, changes.custom_llm_provider);
    // the id alone never reaches the written columns
    assert_eq!(
        vec!["vector_store_description", "vector_store_metadata"],
        written_names(&changes)
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
        .filter(|(_, change)| change.is_some())
        .map(|(name, _)| name)
        .collect()
}

#[tokio::test]
async fn update_keeps_the_stored_params_and_registry_row() {
    let database = FakeDatabase::new(vec![db_store("vs_a")]);
    let cache = FakeCache::new(vec![db_store("vs_a")]);
    let setup = Setup {
        database: Some(database.clone()),
        cache: Some(cache.clone()),
        registry: Some(Vec::new()),
        world: World::default(),
        privileges: false,
        settings: FeatureSettings::default(),
        events: Events::default(),
    };
    let state = build(setup);
    let (status, _, _) = call_update(&state, &internal("u1", None), NULL_UPDATE).await;
    assert_eq!(StatusCode::OK, status);
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
}

#[tokio::test]
async fn update_returns_the_stored_row() {
    let setup = Setup {
        database: Some(FakeDatabase::new(vec![db_store("vs_a")])),
        cache: Some(FakeCache::new(vec![db_store("vs_a")])),
        registry: Some(Vec::new()),
        world: World::default(),
        privileges: false,
        settings: FeatureSettings::default(),
        events: Events::default(),
    };
    let state = build(setup);
    let (status, _headers, body) = call_update(&state, &internal("u1", None), NULL_UPDATE).await;
    assert_eq!(StatusCode::OK, status);
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
}

#[tokio::test]
async fn update_writes_the_present_values() {
    let database = FakeDatabase::new(vec![db_store("vs_a")]);
    let setup = Setup {
        database: Some(database.clone()),
        cache: Some(FakeCache::new(vec![db_store("vs_a")])),
        registry: Some(Vec::new()),
        world: World::default(),
        privileges: false,
        settings: FeatureSettings::default(),
        events: Events::default(),
    };
    let state = build(setup);
    let (status, _, _) = call_update(&state, &internal("u1", None), PRESENT_UPDATE).await;
    assert_eq!(StatusCode::OK, status);
    let changes = database.updates().pop().expect("the update ran").1;
    assert_eq!(
        Some(Some("bedrock".to_owned())),
        changes.custom_llm_provider
    );
    assert_eq!(Some(Some("renamed".to_owned())), changes.vector_store_name);
    assert_eq!(None, changes.vector_store_description);
    let Some(Some(metadata)) = changes.vector_store_metadata else {
        panic!("the metadata is written as JSON text");
    };
    let parsed: Json = serde_json::from_str(&metadata).expect("the metadata is JSON");
    assert_eq!(
        Some(&Json::Number(Number::from(1_i64))),
        field(&parsed, "x")
    );
}

#[tokio::test]
async fn update_persists_the_present_values() {
    let database = FakeDatabase::new(vec![db_store("vs_a")]);
    let setup = Setup {
        database: Some(database.clone()),
        cache: Some(FakeCache::new(vec![db_store("vs_a")])),
        registry: Some(Vec::new()),
        world: World::default(),
        privileges: false,
        settings: FeatureSettings::default(),
        events: Events::default(),
    };
    let state = build(setup);
    let (status, _, body) = call_update(&state, &internal("u1", None), PRESENT_UPDATE).await;
    assert_eq!(StatusCode::OK, status);
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
}

// an explicit null provider is written as null
#[tokio::test]
async fn update_writes_an_explicit_null_provider() {
    let database = FakeDatabase::new(vec![db_store("vs_a")]);
    let setup = Setup {
        database: Some(database.clone()),
        cache: Some(FakeCache::new(vec![db_store("vs_a")])),
        registry: Some(Vec::new()),
        world: World::default(),
        privileges: false,
        settings: FeatureSettings::default(),
        events: Events::default(),
    };
    let state = build(setup);
    let (status, _, _) = call_update(
        &state,
        &internal("u1", None),
        r#"{"vector_store_id":"vs_a","custom_llm_provider":null}"#,
    )
    .await;
    assert_eq!(StatusCode::OK, status);
    let changes = database.updates().pop().expect("the update ran").1;
    assert_eq!(Some(None), changes.custom_llm_provider);
    // a key the body leaves out is not written at all, so the stored metadata survives
    assert_eq!(None, changes.vector_store_metadata);
    assert_eq!(None, changes.vector_store_name);
    assert_eq!(None, changes.vector_store_description);
    let row = database.rows().pop().expect("the row stays");
    assert_eq!(None, row.custom_llm_provider);
    assert_eq!(
        Some(&Json::Text("prod".to_owned())),
        field(&stored_column(&row, Column::Metadata), "env")
    );
}

// an empty metadata object stores the empty JSON object
#[tokio::test]
async fn update_stores_empty_metadata_as_an_empty_object() {
    let database = FakeDatabase::new(vec![db_store("vs_a")]);
    let setup = Setup {
        database: Some(database.clone()),
        cache: Some(FakeCache::new(vec![db_store("vs_a")])),
        registry: Some(Vec::new()),
        world: World::default(),
        privileges: false,
        settings: FeatureSettings::default(),
        events: Events::default(),
    };
    let state = build(setup);
    let (status, _, _) = call_update(
        &state,
        &internal("u1", None),
        r#"{"vector_store_id":"vs_a","vector_store_metadata":{}}"#,
    )
    .await;
    assert_eq!(StatusCode::OK, status);
    let changes = database.updates().pop().expect("the update ran").1;
    assert_eq!(
        Some(Some(String::from("{}"))),
        changes.vector_store_metadata
    );
}

// the four refusals an update answers before it changes anything: what the table holds, what the body says and who asks
#[rstest]
#[case::config_owned(Refusal {
    database: FakeDatabase::new(vec![db_store("vs_c")]),
    cache: Some(FakeCache::new(vec![config_store("vs_c")])),
    registry: vec![config_store("vs_c")],
    id: "vs_c",
    outcome: Outcome::ConfigOwned,
})]
#[case::missing_row(Refusal {
    database: FakeDatabase::new(Vec::new()),
    cache: Some(FakeCache::new(Vec::new())),
    registry: Vec::new(),
    id: "vs_a",
    outcome: Outcome::Missing,
})]
#[case::denied_access(Refusal {
    database: FakeDatabase::new(vec![team_store("vs_a", "t2")]),
    cache: Some(FakeCache::new(Vec::new())),
    registry: Vec::new(),
    id: "vs_a",
    outcome: Outcome::Denied,
})]
#[case::vanished_row(Refusal {
    database: FakeDatabase::new(vec![db_store("vs_a")]).missing_update(),
    cache: Some(FakeCache::new(Vec::new())),
    registry: Vec::new(),
    id: "vs_a",
    outcome: Outcome::Vanished,
})]
#[tokio::test]
async fn update_errors_before_mutation(#[case] refusal: Refusal) {
    let events = Events::default();
    let setup = Setup {
        database: Some(refusal.database),
        cache: refusal.cache,
        registry: Some(refusal.registry),
        world: World::default(),
        privileges: false,
        settings: FeatureSettings::default(),
        events: events.clone(),
    };
    let id = refusal.id;
    let payload = format!(r#"{{"vector_store_id":"{id}","vector_store_description":"x"}}"#);
    let (status, _headers, body) = call_update(
        &build(setup),
        &internal("u1", refusal.outcome.team()),
        &payload,
    )
    .await;
    assert_eq!(refusal.outcome.status(), status);
    refusal.outcome.detail().check(&body, id);
    let expected: Vec<String> = refusal
        .outcome
        .events()
        .iter()
        .map(|entry| (*entry).to_owned())
        .collect();
    assert_eq!(expected, events.entries());
}

// one refusal: the stores, the body's id and the outcome the fakes must produce
struct Refusal {
    database: FakeDatabase,
    cache: Option<FakeCache>,
    registry: Vec<LiteLlmManagedVectorStore>,
    id: &'static str,
    outcome: Outcome,
}

enum Outcome {
    ConfigOwned,
    Missing,
    Denied,
    Vanished,
}

impl Outcome {
    fn status(&self) -> StatusCode {
        match self {
            Self::ConfigOwned => StatusCode::BAD_REQUEST,
            Self::Missing | Self::Vanished => StatusCode::NOT_FOUND,
            Self::Denied => StatusCode::FORBIDDEN,
        }
    }

    fn detail(&self) -> Detail {
        match self {
            Self::ConfigOwned => Detail::ConfigOwned,
            Self::Missing | Self::Vanished => Detail::NotFound,
            Self::Denied => Detail::AccessDenied,
        }
    }

    fn events(&self) -> &'static [&'static str] {
        match self {
            Self::ConfigOwned => &[],
            Self::Missing | Self::Denied => &["db:find"],
            Self::Vanished => &["db:find", "db:update"],
        }
    }

    fn team(&self) -> Option<&'static str> {
        match self {
            Self::Denied => Some("t1"),
            Self::ConfigOwned | Self::Missing | Self::Vanished => None,
        }
    }
}

enum Detail {
    ConfigOwned,
    NotFound,
    AccessDenied,
}

impl Detail {
    fn check(&self, body: &Json, id: &str) {
        match self {
            Self::ConfigOwned => assert_eq!(id, text(at(at(body, "detail"), "vector_store_id"))),
            Self::NotFound => assert_eq!(
                format!("Vector store with ID {id} not found"),
                text(at(body, "detail"))
            ),
            Self::AccessDenied => assert_eq!(
                "Access denied: You do not have permission to access this vector store",
                text(at(body, "detail"))
            ),
        }
    }
}

#[tokio::test]
async fn update_registry_failure_preserves_database_change() {
    let events = Events::default();
    let database = FakeDatabase::new(vec![db_store("vs_a")]);
    let cache = FakeCache::new(vec![db_store("vs_a")]).failing(CacheFailure::Update);
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
    let (status, _headers, body) = call_update(
        &state,
        &internal("u1", None),
        r#"{"vector_store_id":"vs_a","vector_store_description":"new"}"#,
    )
    .await;
    assert_eq!(StatusCode::INTERNAL_SERVER_ERROR, status);
    assert_eq!("simulated cache failure", text(at(&body, "detail")));
    assert_committed_change(&database, &cache, &events);
    assert!(
        logs.contents()
            .contains("Error updating vector store: simulated cache failure")
    );
}

fn assert_committed_change(database: &FakeDatabase, cache: &FakeCache, events: &Events) {
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
}

#[tokio::test]
async fn update_redacts_existing_credentials() {
    let database = FakeDatabase::new(vec![db_store("vs_a")]);
    let setup = Setup {
        database: Some(database.clone()),
        cache: None,
        registry: Some(Vec::new()),
        world: World::default(),
        privileges: false,
        settings: FeatureSettings::default(),
        events: Events::default(),
    };
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
    let setup = Setup {
        database: None,
        cache: Some(FakeCache::new(Vec::new())),
        registry: Some(Vec::new()),
        world: World::default(),
        privileges: false,
        settings: FeatureSettings::default(),
        events: Events::default(),
    };
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
    let setup = Setup {
        database: Some(FakeDatabase::new(Vec::new())),
        cache: None,
        registry: None,
        world: World::default(),
        privileges: false,
        settings: FeatureSettings::default(),
        events: Events::default(),
    };
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
