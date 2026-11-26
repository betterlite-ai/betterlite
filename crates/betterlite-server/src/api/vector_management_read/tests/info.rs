#![cfg(test)]

use super::super::INFO_PATH;
use super::{
    FakeDatabase, REDACTED_BY_LITELM, Setup, World, admin, at, build, call, capture_logs, db_store,
    field_count, internal, list, loc, memory_store, number, parsed, team_store, text,
};
use axum::http::{Method, StatusCode};
use betterlite_config::sections::management_rbac::Settings as FeatureSettings;
use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;

fn cached(id: &str, metadata: &str) -> LiteLlmManagedVectorStore {
    parsed(&format!(
        r#"{{"vector_store_id":"{id}","custom_llm_provider":"openai","vector_store_metadata":{metadata},"litellm_params":{{"api_key":"k"}}}}"#
    ))
}

// the metadata a cached record's value shows as, from the info route's answer
async fn cached_metadata_value(metadata: &str) -> Json {
    let state = build(Setup {
        database: Some(FakeDatabase::new(Vec::new())),
        registry: Some(vec![cached("vs_a", metadata)]),
        world: World::default(),
        privileges: false,
        settings: FeatureSettings::default(),
    });
    let (_status, _headers, body) = call(
        &state,
        &admin(),
        Method::POST,
        INFO_PATH,
        r#"{"vector_store_id":"vs_a"}"#,
    )
    .await;
    at(at(&body, "vector_store"), "vector_store_metadata").clone()
}

#[tokio::test]
async fn info_prefers_cached_record() {
    let stored = parsed(
        r#"{"vector_store_id":"vs_a","custom_llm_provider":"openai","vector_store_name":"","vector_store_description":"d","vector_store_metadata":"{\"tag\":\"x\"}","created_at":"","updated_at":"2024-01-02T03:04:05Z","litellm_credential_name":"cred-a","litellm_params":{"api_key":"config-secret-a"},"team_id":"","user_id":"u1","is_config":true}"#,
    );
    let database = FakeDatabase::new(vec![db_store("vs_a")]);
    let state = build(Setup {
        database: Some(database.clone()),
        registry: Some(vec![stored]),
        world: World::default(),
        privileges: false,
        settings: FeatureSettings::default(),
    });
    let (status, _headers, body) = call(
        &state,
        &admin(),
        Method::POST,
        INFO_PATH,
        r#"{"vector_store_id":"vs_a"}"#,
    )
    .await;
    assert_eq!(StatusCode::OK, status);
    let table = at(&body, "vector_store");
    assert_eq!(12, field_count(table));
    assert_eq!("vs_a", text(at(table, "vector_store_id")));
    assert_eq!("openai", text(at(table, "custom_llm_provider")));
    assert_eq!(&Json::Null, at(table, "vector_store_name"));
    assert_eq!("d", text(at(table, "vector_store_description")));
    assert_eq!(&Json::Null, at(table, "created_at"));
    assert_eq!("2024-01-02T03:04:05Z", text(at(table, "updated_at")));
    assert_eq!("cred-a", text(at(table, "litellm_credential_name")));
    assert_eq!(&Json::Null, at(table, "team_id"));
    assert_eq!("u1", text(at(table, "user_id")));
    assert_eq!(&Json::Bool(true), at(table, "is_config"));
    assert_eq!("x", text(at(at(table, "vector_store_metadata"), "tag")));
    assert_eq!(
        REDACTED_BY_LITELM,
        text(at(at(table, "litellm_params"), "api_key"))
    );
    assert!(database.asked().is_empty());
}

#[tokio::test]
async fn info_requires_a_database_even_for_a_cached_record() {
    let state = build(Setup {
        database: None,
        registry: Some(vec![memory_store("vs_a")]),
        world: World::default(),
        privileges: false,
        settings: FeatureSettings::default(),
    });
    let (status, _headers, body) = call(
        &state,
        &admin(),
        Method::POST,
        INFO_PATH,
        r#"{"vector_store_id":"vs_a"}"#,
    )
    .await;
    assert_eq!(StatusCode::INTERNAL_SERVER_ERROR, status);
    assert_eq!("Database not connected", text(at(&body, "detail")));
}

#[tokio::test]
async fn info_cached_metadata_shapes() {
    let object = cached_metadata_value(r#"{"env":"prod"}"#).await;
    assert_eq!("prod", text(at(&object, "env")));
    assert_eq!(&Json::Null, &cached_metadata_value("5").await);
    assert_eq!(&Json::Null, &cached_metadata_value("null").await);
    assert_eq!(&Json::Null, &cached_metadata_value("[1]").await);
    assert_eq!(&Json::Null, &cached_metadata_value("true").await);
}

#[tokio::test]
async fn info_cached_metadata_with_invalid_json_is_the_catch_all() {
    let state = build(Setup {
        database: Some(FakeDatabase::new(Vec::new())),
        registry: Some(vec![cached("vs_a", "\"not json\"")]),
        world: World::default(),
        privileges: false,
        settings: FeatureSettings::default(),
    });
    let (logs, _guard) = capture_logs(tracing::Level::ERROR);
    let (status, _headers, body) = call(
        &state,
        &admin(),
        Method::POST,
        INFO_PATH,
        r#"{"vector_store_id":"vs_a"}"#,
    )
    .await;
    assert_eq!(StatusCode::INTERNAL_SERVER_ERROR, status);
    assert_eq!(
        "stored JSON text is not valid JSON",
        text(at(&body, "detail"))
    );
    assert!(
        logs.contents()
            .contains("Error getting vector store info: stored JSON text is not valid JSON")
    );
}

// the deviation: text that parses to a non-object is our own 500, not the response model's validation error
#[tokio::test]
async fn info_cached_metadata_text_that_is_not_an_object_is_the_catch_all() {
    let state = build(Setup {
        database: Some(FakeDatabase::new(Vec::new())),
        registry: Some(vec![cached("vs_a", "\"5\"")]),
        world: World::default(),
        privileges: false,
        settings: FeatureSettings::default(),
    });
    let (logs, _guard) = capture_logs(tracing::Level::ERROR);
    let (status, _headers, body) = call(
        &state,
        &admin(),
        Method::POST,
        INFO_PATH,
        r#"{"vector_store_id":"vs_a"}"#,
    )
    .await;
    assert_eq!(StatusCode::INTERNAL_SERVER_ERROR, status);
    assert_eq!(
        "the stored vector store metadata is not a JSON object",
        text(at(&body, "detail"))
    );
    assert!(logs.contents().contains(
        "Error getting vector store info: the stored vector store metadata is not a JSON object"
    ));
}

#[tokio::test]
async fn info_cached_blank_and_absent_fields_follow_the_model() {
    let stored = parsed(
        r#"{"vector_store_id":"","vector_store_name":"","vector_store_description":"","created_at":"","updated_at":"","litellm_credential_name":"","litellm_params":{"api_key":"k"},"team_id":"","user_id":""}"#,
    );
    let state = build(Setup {
        database: Some(FakeDatabase::new(Vec::new())),
        registry: Some(vec![stored]),
        world: World::default(),
        privileges: false,
        settings: FeatureSettings::default(),
    });
    let (status, _headers, body) = call(
        &state,
        &admin(),
        Method::POST,
        INFO_PATH,
        r#"{"vector_store_id":""}"#,
    )
    .await;
    assert_eq!(StatusCode::OK, status);
    let table = at(&body, "vector_store");
    assert_eq!("", text(at(table, "vector_store_id")));
    assert_eq!("", text(at(table, "custom_llm_provider")));
    assert_eq!(&Json::Bool(false), at(table, "is_config"));
    assert_eq!(&Json::Null, at(table, "vector_store_metadata"));
    assert_eq!(&Json::Null, at(table, "vector_store_name"));
    assert_eq!(&Json::Null, at(table, "vector_store_description"));
    assert_eq!(&Json::Null, at(table, "created_at"));
    assert_eq!(&Json::Null, at(table, "updated_at"));
    assert_eq!(&Json::Null, at(table, "team_id"));
    assert_eq!(&Json::Null, at(table, "user_id"));
    // the one field LiteLLM reads without collapsing a blank to null
    assert_eq!("", text(at(table, "litellm_credential_name")));
}

// no cache hit: the row is fetched, kept whole and redacted, and its own 404 and 403 survive the catch-all
#[tokio::test]
async fn info_database_access_errors() {
    let database = FakeDatabase::new(vec![db_store("vs_a")]);
    let state = build(Setup {
        database: Some(database.clone()),
        registry: None,
        world: World::default(),
        privileges: false,
        settings: FeatureSettings::default(),
    });
    let (status, _headers, body) = call(
        &state,
        &admin(),
        Method::POST,
        INFO_PATH,
        r#"{"vector_store_id":"vs_a"}"#,
    )
    .await;
    assert_eq!(StatusCode::OK, status);
    let table = at(&body, "vector_store");
    assert_eq!("db-vs_a", text(at(table, "vector_store_name")));
    assert_eq!("2024-01-02T03:04:05Z", text(at(table, "created_at")));
    assert_eq!("prod", text(at(at(table, "vector_store_metadata"), "env")));
    assert_eq!(
        REDACTED_BY_LITELM,
        text(at(at(table, "litellm_params"), "api_key"))
    );
    assert_eq!(vec!["vs_a".to_owned()], database.asked());
}

#[tokio::test]
async fn info_missing_database_row_is_404() {
    let state = build(Setup {
        database: Some(FakeDatabase::new(Vec::new())),
        registry: None,
        world: World::default(),
        privileges: false,
        settings: FeatureSettings::default(),
    });
    let (status, _, body) = call(
        &state,
        &admin(),
        Method::POST,
        INFO_PATH,
        r#"{"vector_store_id":"vs_missing"}"#,
    )
    .await;
    assert_eq!(StatusCode::NOT_FOUND, status);
    assert_eq!(
        "Vector store with ID vs_missing not found",
        text(at(&body, "detail"))
    );
}

#[tokio::test]
async fn info_database_row_access_denial_is_403() {
    let database = FakeDatabase::new(vec![team_store("vs_team", "t9")]);
    let state = build(Setup {
        database: Some(database.clone()),
        registry: None,
        world: World::default(),
        privileges: false,
        settings: FeatureSettings::default(),
    });
    let (status, _, body) = call(
        &state,
        &internal("u1", Some("t1")),
        Method::POST,
        INFO_PATH,
        r#"{"vector_store_id":"vs_team"}"#,
    )
    .await;
    assert_eq!(StatusCode::FORBIDDEN, status);
    assert_eq!(
        "Access denied: You do not have permission to access this vector store",
        text(at(&body, "detail"))
    );
    assert_eq!(vec!["vs_team".to_owned()], database.asked());
}

#[tokio::test]
async fn info_database_row_without_params_gets_defaults() {
    let bare = parsed(r#"{"vector_store_id":"vs_b","custom_llm_provider":"openai"}"#);
    let state = build(Setup {
        database: Some(FakeDatabase::new(vec![bare])),
        registry: None,
        world: World::default(),
        privileges: false,
        settings: FeatureSettings::default(),
    });
    let (status, _headers, body) = call(
        &state,
        &admin(),
        Method::POST,
        INFO_PATH,
        r#"{"vector_store_id":"vs_b"}"#,
    )
    .await;
    assert_eq!(StatusCode::OK, status);
    let table = at(&body, "vector_store");
    assert_eq!(&Json::Null, at(table, "litellm_params"));
    assert_eq!(&Json::Null, at(table, "vector_store_metadata"));
    assert_eq!(&Json::Bool(false), at(table, "is_config"));
}

// the database row is returned as stored, so a blank string stays blank instead of collapsing to null
#[tokio::test]
async fn info_database_row_keeps_blank_fields() {
    let blank = parsed(
        r#"{"vector_store_id":"vs_blank","custom_llm_provider":"openai","vector_store_name":"","vector_store_description":"","created_at":"","updated_at":"","team_id":"","user_id":""}"#,
    );
    let state = build(Setup {
        database: Some(FakeDatabase::new(vec![blank])),
        registry: None,
        world: World::default(),
        privileges: false,
        settings: FeatureSettings::default(),
    });
    let (status, _headers, body) = call(
        &state,
        &admin(),
        Method::POST,
        INFO_PATH,
        r#"{"vector_store_id":"vs_blank"}"#,
    )
    .await;
    assert_eq!(StatusCode::OK, status);
    let table = at(&body, "vector_store");
    assert_eq!("", text(at(table, "vector_store_name")));
    assert_eq!("", text(at(table, "vector_store_description")));
    assert_eq!("", text(at(table, "created_at")));
    assert_eq!("", text(at(table, "updated_at")));
    assert_eq!("", text(at(table, "team_id")));
    assert_eq!("", text(at(table, "user_id")));
}

// the deviation: a stored non-object params is served after redaction, not refused by the response model
#[tokio::test]
async fn info_database_params_that_are_not_an_object_are_served_as_stored() {
    let odd = parsed(
        r#"{"vector_store_id":"vs_params","custom_llm_provider":"openai","litellm_params":7}"#,
    );
    let state = build(Setup {
        database: Some(FakeDatabase::new(vec![odd])),
        registry: None,
        world: World::default(),
        privileges: false,
        settings: FeatureSettings::default(),
    });
    let (status, _headers, body) = call(
        &state,
        &admin(),
        Method::POST,
        INFO_PATH,
        r#"{"vector_store_id":"vs_params"}"#,
    )
    .await;
    assert_eq!(StatusCode::OK, status);
    assert_eq!(7, number(at(at(&body, "vector_store"), "litellm_params")));
}

// the deviation: a database row's non-object metadata is served as stored, not refused by the response model
#[tokio::test]
async fn info_database_metadata_that_is_not_an_object_is_served_as_stored() {
    let odd = parsed(
        r#"{"vector_store_id":"vs_odd","custom_llm_provider":"openai","vector_store_metadata":[1]}"#,
    );
    let state = build(Setup {
        database: Some(FakeDatabase::new(vec![odd])),
        registry: None,
        world: World::default(),
        privileges: false,
        settings: FeatureSettings::default(),
    });
    let (status, _headers, body) = call(
        &state,
        &admin(),
        Method::POST,
        INFO_PATH,
        r#"{"vector_store_id":"vs_odd"}"#,
    )
    .await;
    assert_eq!(StatusCode::OK, status);
    let table = at(&body, "vector_store");
    let metadata = list(at(table, "vector_store_metadata"));
    assert_eq!(1, metadata.len());
    assert_eq!(1, number(metadata.first().expect("one item")));
}

#[tokio::test]
async fn info_denied_cached_record_is_403() {
    let database = FakeDatabase::new(vec![db_store("vs_team")]);
    let state = build(Setup {
        database: Some(database.clone()),
        registry: Some(vec![team_store("vs_team", "t9")]),
        world: World::default(),
        privileges: false,
        settings: FeatureSettings::default(),
    });
    let (logs, _guard) = capture_logs(tracing::Level::ERROR);
    let (status, _headers, body) = call(
        &state,
        &internal("u1", Some("t1")),
        Method::POST,
        INFO_PATH,
        r#"{"vector_store_id":"vs_team"}"#,
    )
    .await;
    assert_eq!(StatusCode::FORBIDDEN, status);
    assert_eq!(
        "Access denied: You do not have permission to access this vector store",
        text(at(&body, "detail"))
    );
    assert!(database.asked().is_empty());
    assert!(!logs.contents().contains("Error getting vector store info"));
}

#[tokio::test]
async fn info_body_without_an_id_is_fastapi_validation() {
    let state = build(Setup::default());
    let (status, _headers, body) = call(&state, &admin(), Method::POST, INFO_PATH, "{}").await;
    assert_eq!(StatusCode::UNPROCESSABLE_ENTITY, status);
    let issue = list(at(&body, "detail")).first().expect("one issue");
    assert_eq!("missing", text(at(issue, "type")));
    assert_eq!(
        vec!["body".to_owned(), "vector_store_id".to_owned()],
        loc(issue)
    );
    assert_eq!("Field required", text(at(issue, "msg")));
}
