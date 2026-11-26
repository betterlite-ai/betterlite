#![cfg(test)]

mod support;

use axum::http::{HeaderMap, Method, StatusCode};
use betterlite_config::sections::management_rbac::Settings as FeatureSettings;
use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;

use self::support::{
    FakeDatabase, Setup, World, admin, build, call, capture_logs, config_store, db_store,
    flags_blocked, flags_blocked_team_admins, flags_open, internal, memory_store, parsed,
    registry_of, team_store, ui_session, viewer,
};
use super::{INFO_PATH, LIST_PATH, V1_LIST_PATH};
use crate::api::vector_management_boundaries::redaction::REDACTED_BY_LITELM;

fn at<'value>(value: &'value Json, key: &str) -> &'value Json {
    match value {
        Json::Object(fields) => fields
            .get(key)
            .unwrap_or_else(|| panic!("the object carries {key}")),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("expected an object carrying {key}, got {value:?}")
        }
    }
}

fn field<'value>(value: &'value Json, key: &str) -> Option<&'value Json> {
    match value {
        Json::Object(fields) => fields.get(key),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

fn field_count(value: &Json) -> usize {
    match value {
        Json::Object(fields) => fields.len(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("expected an object, got {value:?}")
        }
    }
}

fn text(value: &Json) -> &str {
    match value {
        Json::Text(inner) => inner,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            panic!("expected text, got {value:?}")
        }
    }
}

fn number(value: &Json) -> i64 {
    match value {
        Json::Number(inner) => inner.as_i64().expect("an integer"),
        Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => {
            panic!("expected a number, got {value:?}")
        }
    }
}

fn list(value: &Json) -> &[Json] {
    match value {
        Json::List(items) => items,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
            panic!("expected a list, got {value:?}")
        }
    }
}

fn ids(body: &Json) -> Vec<String> {
    list(at(body, "data"))
        .iter()
        .map(|item| text(at(item, "vector_store_id")).to_owned())
        .collect()
}

fn listed<'value>(body: &'value Json, id: &str) -> &'value Json {
    list(at(body, "data"))
        .iter()
        .find(|item| field(item, "vector_store_id").map(text) == Some(id))
        .unwrap_or_else(|| panic!("{id} is listed"))
}

fn loc(issue: &Json) -> Vec<String> {
    list(at(issue, "loc"))
        .iter()
        .map(|item| text(item).to_owned())
        .collect()
}

fn content_type(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("content-type")
        .and_then(|value| value.to_str().ok())
}

// a registry record carrying the metadata shape under test
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
async fn list_reconciles_config_and_database() {
    let database = FakeDatabase::new(vec![db_store("vs_db"), db_store("vs_shared")]);
    let setup = Setup {
        database: Some(database),
        registry: Some(vec![
            memory_store("vs_stale"),
            memory_store("vs_db"),
            config_store("vs_shared"),
            config_store("vs_config"),
        ]),
        world: World::default(),
        privileges: false,
        settings: FeatureSettings::default(),
    };
    let state = build(setup);
    let (status, _headers, body) = call(&state, &admin(), Method::GET, LIST_PATH, "").await;
    assert_eq!(StatusCode::OK, status);
    assert_eq!(
        vec![
            "vs_db".to_owned(),
            "vs_shared".to_owned(),
            "vs_config".to_owned()
        ],
        ids(&body)
    );
    let db = listed(&body, "vs_db");
    assert_eq!("db-vs_db", text(at(db, "vector_store_name")));
    assert_eq!(&Json::Bool(false), at(db, "is_config"));
    assert_eq!(&Json::Null, at(db, "team_id"));
    assert_eq!("us", text(at(at(db, "litellm_params"), "region")));
    assert_eq!(
        REDACTED_BY_LITELM,
        text(at(at(db, "litellm_params"), "api_key"))
    );
    let shared = listed(&body, "vs_shared");
    assert_eq!("config-vs_shared", text(at(shared, "vector_store_name")));
    assert_eq!(&Json::Bool(true), at(shared, "is_config"));
    let (alias_status, _alias_headers, alias_body) =
        call(&state, &admin(), Method::GET, V1_LIST_PATH, "").await;
    assert_eq!(StatusCode::OK, alias_status);
    assert_eq!(ids(&body), ids(&alias_body));
}

#[tokio::test]
async fn list_reconciliation_purges_and_syncs_the_registry() {
    let database = FakeDatabase::new(vec![db_store("vs_db"), db_store("vs_shared")]);
    let setup = Setup {
        database: Some(database),
        registry: Some(vec![
            memory_store("vs_stale"),
            config_store("vs_shared"),
            config_store("vs_config"),
        ]),
        world: World::default(),
        privileges: false,
        settings: FeatureSettings::default(),
    };
    let state = build(setup);
    let (logs, _guard) = capture_logs(tracing::Level::DEBUG);
    let (status, _headers, _body) = call(&state, &admin(), Method::GET, LIST_PATH, "").await;
    assert_eq!(StatusCode::OK, status);
    {
        let registry = registry_of(&state);
        let stored: Vec<String> = registry
            .stores
            .iter()
            .filter_map(|store| store.vector_store_id.clone())
            .collect();
        assert_eq!(
            vec![
                "vs_shared".to_owned(),
                "vs_config".to_owned(),
                "vs_db".to_owned()
            ],
            stored
        );
        let row = registry
            .store_by_id("vs_db")
            .expect("the database row joined the registry");
        let params = row
            .litellm_params
            .as_ref()
            .expect("the row keeps its params");
        assert_eq!(
            Some("db-secret-vs_db"),
            params.get("api_key").and_then(|value| value.as_str())
        );
        let config = registry
            .store_by_id("vs_shared")
            .expect("the config record stays in the registry");
        assert_eq!(
            Some("config-secret-vs_shared"),
            config
                .litellm_params
                .as_ref()
                .and_then(|params| params.get("api_key"))
                .and_then(|value| value.as_str())
        );
    }
    let captured = logs.contents();
    assert!(captured.contains(
        "Vector store vs_stale exists in memory but not in database - marking for deletion from cache"
    ));
    assert!(captured.contains("Removed deleted vector store vs_stale from in-memory registry"));
}

#[tokio::test]
async fn list_pagination_is_metadata_only() {
    let database = FakeDatabase::new(vec![db_store("vs_1"), db_store("vs_2"), db_store("vs_3")]);
    let state = build(Setup {
        database: Some(database),
        registry: None,
        world: World::default(),
        privileges: false,
        settings: FeatureSettings::default(),
    });
    let (status, _headers, default_body) = call(&state, &admin(), Method::GET, LIST_PATH, "").await;
    assert_eq!(StatusCode::OK, status);
    assert_eq!(3, list(at(&default_body, "data")).len());
    assert_eq!(3, number(at(&default_body, "total_count")));
    assert_eq!(1, number(at(&default_body, "current_page")));
    assert_eq!(1, number(at(&default_body, "total_pages")));
    let (page_status, _page_headers, page_body) = call(
        &state,
        &admin(),
        Method::GET,
        "/vector_store/list?page=2&page_size=2",
        "",
    )
    .await;
    assert_eq!(StatusCode::OK, page_status);
    assert_eq!(3, list(at(&page_body, "data")).len());
    assert_eq!(2, number(at(&page_body, "current_page")));
    assert_eq!(2, number(at(&page_body, "total_pages")));
    let (_zero_status, _zero_headers, zero_body) = call(
        &state,
        &admin(),
        Method::GET,
        "/vector_store/list?page=0",
        "",
    )
    .await;
    assert_eq!(0, number(at(&zero_body, "current_page")));
    let (_two_status, _two_headers, two_body) = call(
        &state,
        &admin(),
        Method::GET,
        "/vector_store/list?page_size=-2",
        "",
    )
    .await;
    assert_eq!(0, number(at(&two_body, "total_pages")));
    let (_one_status, _one_headers, one_body) = call(
        &state,
        &admin(),
        Method::GET,
        "/vector_store/list?page_size=-1",
        "",
    )
    .await;
    assert_eq!(-1, number(at(&one_body, "total_pages")));
}

#[tokio::test]
async fn list_with_a_zero_page_size_is_the_division_error() {
    let database = FakeDatabase::new(vec![db_store("vs_db")]);
    let state = build(Setup {
        database: Some(database),
        registry: Some(vec![memory_store("vs_stale")]),
        world: World::default(),
        privileges: false,
        settings: FeatureSettings::default(),
    });
    let (logs, _guard) = capture_logs(tracing::Level::ERROR);
    let (status, _headers, body) = call(
        &state,
        &admin(),
        Method::GET,
        "/vector_store/list?page_size=0",
        "",
    )
    .await;
    assert_eq!(StatusCode::INTERNAL_SERVER_ERROR, status);
    assert_eq!(
        "integer division or modulo by zero",
        text(at(&body, "detail"))
    );
    assert!(registry_of(&state).store_by_id("vs_stale").is_none());
    assert!(
        logs.contents()
            .contains("Error listing vector stores: integer division or modulo by zero")
    );
}

#[tokio::test]
async fn list_query_validation_precedes_the_handler() {
    let database = FakeDatabase::new(vec![db_store("vs_db")]);
    let state = build(Setup {
        database: Some(database.clone()),
        registry: Some(vec![memory_store("vs_stale")]),
        world: World::default(),
        privileges: false,
        settings: FeatureSettings::default(),
    });
    let (status, headers, body) = call(
        &state,
        &admin(),
        Method::GET,
        "/vector_store/list?page=bad",
        "",
    )
    .await;
    assert_eq!(StatusCode::UNPROCESSABLE_ENTITY, status);
    assert_eq!(Some("application/json"), content_type(&headers));
    let issue = list(at(&body, "detail")).first().expect("one issue");
    assert_eq!("int_parsing", text(at(issue, "type")));
    assert_eq!(vec!["query".to_owned(), "page".to_owned()], loc(issue));
    assert_eq!(
        "Input should be a valid integer, unable to parse string as an integer",
        text(at(issue, "msg"))
    );
    assert_eq!(0, database.loads());
    assert!(registry_of(&state).store_by_id("vs_stale").is_some());
    // the deviation: an i64 cannot hold the page, so it is the same 422 at query.page
    let (huge_status, _huge_headers, huge_body) = call(
        &state,
        &admin(),
        Method::GET,
        "/vector_store/list?page=9223372036854775808",
        "",
    )
    .await;
    assert_eq!(StatusCode::UNPROCESSABLE_ENTITY, huge_status);
    let huge_issue = list(at(&huge_body, "detail")).first().expect("one issue");
    assert_eq!(vec!["query".to_owned(), "page".to_owned()], loc(huge_issue));
    assert_eq!(0, database.loads());
}

#[tokio::test]
async fn list_reports_an_access_helper_failure_as_the_catch_all() {
    let database = FakeDatabase::new(vec![db_store("vs_db")]);
    let world = World {
        user_teams: vec!["t1".to_owned()],
        database: true,
    };
    let state = build(Setup {
        database: Some(database),
        registry: None,
        world,
        privileges: false,
        settings: FeatureSettings::default(),
    });
    let (logs, _guard) = capture_logs(tracing::Level::ERROR);
    let (status, _headers, body) =
        call(&state, &ui_session("u1"), Method::GET, LIST_PATH, "").await;
    assert_eq!(StatusCode::INTERNAL_SERVER_ERROR, status);
    assert_eq!(
        "Team doesn't exist in db. Team=t1. Create team via `/team/new` call.",
        text(at(&body, "detail"))
    );
    assert!(
        logs.contents()
            .contains("Error listing vector stores: Team doesn't exist in db.")
    );
}

#[tokio::test]
async fn list_without_a_registry_lists_the_database() {
    let database = FakeDatabase::new(vec![db_store("vs_db")]);
    let state = build(Setup {
        database: Some(database),
        registry: None,
        world: World::default(),
        privileges: false,
        settings: FeatureSettings::default(),
    });
    let (status, _headers, body) = call(&state, &admin(), Method::GET, LIST_PATH, "").await;
    assert_eq!(StatusCode::OK, status);
    assert_eq!(vec!["vs_db".to_owned()], ids(&body));
}

#[tokio::test]
async fn list_without_a_database_purges_uncached_records() {
    let state = build(Setup {
        database: None,
        registry: Some(vec![memory_store("vs_stale"), config_store("vs_config")]),
        world: World::default(),
        privileges: false,
        settings: FeatureSettings::default(),
    });
    let (status, _headers, body) = call(&state, &admin(), Method::GET, LIST_PATH, "").await;
    assert_eq!(StatusCode::OK, status);
    assert_eq!(vec!["vs_config".to_owned()], ids(&body));
    let registry = registry_of(&state);
    let stored: Vec<&str> = registry
        .stores
        .iter()
        .filter_map(|store| store.vector_store_id.as_deref())
        .collect();
    assert_eq!(vec!["vs_config"], stored);
}

// reconciliation comes first: a stale team record would pass the filter, and is gone before it runs
#[tokio::test]
async fn list_filters_after_reconciling_the_registry() {
    let database = FakeDatabase::new(vec![db_store("vs_db"), team_store("vs_team", "t1")]);
    let state = build(Setup {
        database: Some(database),
        registry: Some(vec![team_store("vs_stale", "t1")]),
        world: World::default(),
        privileges: false,
        settings: FeatureSettings::default(),
    });
    let (status, _headers, body) = call(
        &state,
        &internal("u1", Some("t1")),
        Method::GET,
        LIST_PATH,
        "",
    )
    .await;
    assert_eq!(StatusCode::OK, status);
    assert_eq!(vec!["vs_team".to_owned()], ids(&body));
    assert!(registry_of(&state).store_by_id("vs_stale").is_none());
}

#[tokio::test]
async fn list_skips_records_without_an_id_and_drops_undeclared_fields() {
    let database = FakeDatabase::new(vec![
        parsed(
            r#"{"custom_llm_provider":"openai","vector_store_name":"nameless","litellm_params":{}}"#,
        ),
        parsed(
            r#"{"vector_store_id":"vs_extra","custom_llm_provider":"openai","litellm_params":{},"surprise":"x"}"#,
        ),
    ]);
    let state = build(Setup {
        database: Some(database),
        registry: Some(vec![parsed(
            r#"{"vector_store_id":"","custom_llm_provider":"openai"}"#,
        )]),
        world: World::default(),
        privileges: false,
        settings: FeatureSettings::default(),
    });
    let (status, _headers, body) = call(&state, &admin(), Method::GET, LIST_PATH, "").await;
    assert_eq!(StatusCode::OK, status);
    assert_eq!(vec!["vs_extra".to_owned()], ids(&body));
    assert_eq!(1, number(at(&body, "total_count")));
    let extra = listed(&body, "vs_extra");
    assert_eq!(None, field(extra, "surprise"));
    assert_eq!(None, field(extra, "vector_store_metadata"));
}

// the deviations reach the list too: non-object metadata and params are served as stored
#[tokio::test]
async fn list_serves_values_the_response_model_would_refuse() {
    let odd = parsed(
        r#"{"vector_store_id":"vs_odd","custom_llm_provider":"openai","vector_store_metadata":[1],"litellm_params":7}"#,
    );
    let state = build(Setup {
        database: Some(FakeDatabase::new(vec![odd])),
        registry: None,
        world: World::default(),
        privileges: false,
        settings: FeatureSettings::default(),
    });
    let (status, _headers, body) = call(&state, &admin(), Method::GET, LIST_PATH, "").await;
    assert_eq!(StatusCode::OK, status);
    let item = listed(&body, "vs_odd");
    assert_eq!(7, number(at(item, "litellm_params")));
    let metadata = list(at(item, "vector_store_metadata"));
    assert_eq!(1, number(metadata.first().expect("one item")));
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

    let missing_state = build(Setup {
        database: Some(FakeDatabase::new(Vec::new())),
        registry: None,
        world: World::default(),
        privileges: false,
        settings: FeatureSettings::default(),
    });
    let (missing_status, _missing_headers, missing_body) = call(
        &missing_state,
        &admin(),
        Method::POST,
        INFO_PATH,
        r#"{"vector_store_id":"vs_missing"}"#,
    )
    .await;
    assert_eq!(StatusCode::NOT_FOUND, missing_status);
    assert_eq!(
        "Vector store with ID vs_missing not found",
        text(at(&missing_body, "detail"))
    );

    let row_database = FakeDatabase::new(vec![team_store("vs_team", "t9")]);
    let row_state = build(Setup {
        database: Some(row_database.clone()),
        registry: None,
        world: World::default(),
        privileges: false,
        settings: FeatureSettings::default(),
    });
    let caller = internal("u1", Some("t1"));
    let (row_status, _row_headers, row_body) = call(
        &row_state,
        &caller,
        Method::POST,
        INFO_PATH,
        r#"{"vector_store_id":"vs_team"}"#,
    )
    .await;
    assert_eq!(StatusCode::FORBIDDEN, row_status);
    assert_eq!(
        "Access denied: You do not have permission to access this vector store",
        text(at(&row_body, "detail"))
    );
    assert_eq!(vec!["vs_team".to_owned()], row_database.asked());
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

#[tokio::test]
async fn management_read_auth_and_flags() {
    let state = build(Setup {
        database: Some(FakeDatabase::new(vec![db_store("vs_db")])),
        registry: None,
        world: World::default(),
        privileges: false,
        settings: flags_blocked(),
    });
    let caller = internal("u1", None);
    let (status, headers, body) = call(&state, &caller, Method::GET, LIST_PATH, "").await;
    assert_eq!(StatusCode::FORBIDDEN, status);
    assert_eq!(Some("application/json"), content_type(&headers));
    assert_eq!(1, field_count(&body));
    let detail = at(&body, "detail");
    assert_eq!(1, field_count(detail));
    assert_eq!(
        "Access to vector_stores is disabled for your role. Contact your proxy admin.",
        text(at(detail, "error"))
    );
    assert_eq!(None, field(&body, "type"));
    assert_eq!(None, field(&body, "param"));
    assert_eq!(None, field(&body, "code"));
    let (alias_status, _alias_headers, alias_body) =
        call(&state, &caller, Method::GET, V1_LIST_PATH, "").await;
    assert_eq!(StatusCode::FORBIDDEN, alias_status);
    assert_eq!(body, alias_body);
    let (blocked_status, _blocked_headers, _blocked_body) = call(
        &state,
        &caller,
        Method::POST,
        INFO_PATH,
        r#"{"vector_store_id":"vs_db"}"#,
    )
    .await;
    assert_eq!(StatusCode::FORBIDDEN, blocked_status);
    assert_eq!(
        StatusCode::OK,
        call(&state, &admin(), Method::GET, LIST_PATH, "").await.0
    );
    assert_eq!(
        StatusCode::OK,
        call(&state, &viewer(), Method::GET, LIST_PATH, "").await.0
    );

    let open = build(Setup {
        database: Some(FakeDatabase::new(vec![
            db_store("vs_free"),
            team_store("vs_team", "t1"),
        ])),
        registry: None,
        world: World::default(),
        privileges: false,
        settings: flags_open(),
    });
    let (open_status, _open_headers, open_body) = call(
        &open,
        &internal("u1", Some("t1")),
        Method::GET,
        LIST_PATH,
        "",
    )
    .await;
    assert_eq!(StatusCode::OK, open_status);
    assert_eq!(vec!["vs_team".to_owned()], ids(&open_body));
}

#[tokio::test]
async fn team_admin_allow_flag_exempts_only_a_verified_admin() {
    let caller = internal("u1", None);
    let allowed = build(Setup {
        database: Some(FakeDatabase::new(vec![db_store("vs_db")])),
        registry: None,
        world: World::default(),
        privileges: true,
        settings: flags_blocked_team_admins(),
    });
    assert_eq!(
        StatusCode::OK,
        call(&allowed, &caller, Method::GET, LIST_PATH, "").await.0
    );
    let unverified = build(Setup {
        database: Some(FakeDatabase::new(vec![db_store("vs_db")])),
        registry: None,
        world: World::default(),
        privileges: false,
        settings: flags_blocked_team_admins(),
    });
    assert_eq!(
        StatusCode::FORBIDDEN,
        call(&unverified, &caller, Method::GET, LIST_PATH, "")
            .await
            .0
    );
}
