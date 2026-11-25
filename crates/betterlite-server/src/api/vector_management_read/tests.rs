#![cfg(test)]

mod support;

use axum::http::{HeaderMap, StatusCode};
use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;

use self::support::{
    FakeDatabase, Setup, World, admin, build, capture_logs, config_store, db_store, flags_blocked,
    flags_blocked_team_admins, flags_open, get, internal, memory_store, parsed, post, registry_of,
    team_store, ui_session, viewer,
};
use super::{LIST_PATH, V1_LIST_PATH};
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

#[tokio::test]
async fn list_reconciles_config_and_database() {
    let database = FakeDatabase::new(vec![db_store("vs_db"), db_store("vs_shared")]);
    let setup = Setup {
        database: Some(database),
        registry: Some(vec![
            memory_store("vs_stale"),
            config_store("vs_shared"),
            config_store("vs_config"),
        ]),
        ..Setup::default()
    };
    let state = build(setup);
    let (status, _headers, body) = get(&state, LIST_PATH, &admin()).await;
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
    let (alias_status, _alias_headers, alias_body) = get(&state, V1_LIST_PATH, &admin()).await;
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
        ..Setup::default()
    };
    let state = build(setup);
    let (logs, _guard) = capture_logs(tracing::Level::DEBUG);
    let (status, _headers, _body) = get(&state, LIST_PATH, &admin()).await;
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
        ..Setup::default()
    });
    let (status, _headers, default_body) = get(&state, LIST_PATH, &admin()).await;
    assert_eq!(StatusCode::OK, status);
    assert_eq!(3, list(at(&default_body, "data")).len());
    assert_eq!(3, number(at(&default_body, "total_count")));
    assert_eq!(1, number(at(&default_body, "current_page")));
    assert_eq!(1, number(at(&default_body, "total_pages")));
    let (page_status, _page_headers, page_body) =
        get(&state, "/vector_store/list?page=2&page_size=2", &admin()).await;
    assert_eq!(StatusCode::OK, page_status);
    assert_eq!(3, list(at(&page_body, "data")).len());
    assert_eq!(2, number(at(&page_body, "current_page")));
    assert_eq!(2, number(at(&page_body, "total_pages")));
    let (_zero_status, _zero_headers, zero_body) =
        get(&state, "/vector_store/list?page=0", &admin()).await;
    assert_eq!(0, number(at(&zero_body, "current_page")));
    let (_two_status, _two_headers, two_body) =
        get(&state, "/vector_store/list?page_size=-2", &admin()).await;
    assert_eq!(0, number(at(&two_body, "total_pages")));
    let (_one_status, _one_headers, one_body) =
        get(&state, "/vector_store/list?page_size=-1", &admin()).await;
    assert_eq!(-1, number(at(&one_body, "total_pages")));
}

#[tokio::test]
async fn list_with_a_zero_page_size_is_the_division_error() {
    let database = FakeDatabase::new(vec![db_store("vs_db")]);
    let state = build(Setup {
        database: Some(database),
        registry: Some(vec![memory_store("vs_stale")]),
        ..Setup::default()
    });
    let (logs, _guard) = capture_logs(tracing::Level::ERROR);
    let (status, _headers, body) = get(&state, "/vector_store/list?page_size=0", &admin()).await;
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
        ..Setup::default()
    });
    let (status, headers, body) = get(&state, "/vector_store/list?page=bad", &admin()).await;
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
        world,
        ..Setup::default()
    });
    let (logs, _guard) = capture_logs(tracing::Level::ERROR);
    let (status, _headers, body) = get(&state, LIST_PATH, &ui_session("u1")).await;
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
        ..Setup::default()
    });
    let (status, _headers, body) = get(&state, LIST_PATH, &admin()).await;
    assert_eq!(StatusCode::OK, status);
    assert_eq!(vec!["vs_db".to_owned()], ids(&body));
}

#[tokio::test]
async fn list_without_a_database_purges_uncached_records() {
    let state = build(Setup {
        registry: Some(vec![memory_store("vs_stale"), config_store("vs_config")]),
        ..Setup::default()
    });
    let (status, _headers, body) = get(&state, LIST_PATH, &admin()).await;
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
        ..Setup::default()
    });
    let (status, _headers, body) = get(&state, LIST_PATH, &admin()).await;
    assert_eq!(StatusCode::OK, status);
    assert_eq!(vec!["vs_extra".to_owned()], ids(&body));
    assert_eq!(1, number(at(&body, "total_count")));
    let extra = listed(&body, "vs_extra");
    assert_eq!(None, field(extra, "surprise"));
    assert_eq!(None, field(extra, "vector_store_metadata"));
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
        ..Setup::default()
    });
    let (status, _headers, body) = post(&state, r#"{"vector_store_id":"vs_a"}"#, &admin()).await;
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
        registry: Some(vec![memory_store("vs_a")]),
        ..Setup::default()
    });
    let (status, _headers, body) = post(&state, r#"{"vector_store_id":"vs_a"}"#, &admin()).await;
    assert_eq!(StatusCode::INTERNAL_SERVER_ERROR, status);
    assert_eq!("Database not connected", text(at(&body, "detail")));
}

#[tokio::test]
async fn info_cached_metadata_shapes() {
    let state = build(Setup {
        database: Some(FakeDatabase::new(Vec::new())),
        registry: Some(vec![cached("vs_a", r#"{"env":"prod"}"#)]),
        ..Setup::default()
    });
    let (_object_status, _object_headers, object_body) =
        post(&state, r#"{"vector_store_id":"vs_a"}"#, &admin()).await;
    assert_eq!(
        "prod",
        text(at(
            at(at(&object_body, "vector_store"), "vector_store_metadata"),
            "env"
        ))
    );
    for metadata in ["5", "null", "[1]", "true"] {
        let registry = vec![cached("vs_a", metadata)];
        let case = build(Setup {
            database: Some(FakeDatabase::new(Vec::new())),
            registry: Some(registry),
            ..Setup::default()
        });
        let (_scalar_status, _scalar_headers, scalar_body) =
            post(&case, r#"{"vector_store_id":"vs_a"}"#, &admin()).await;
        assert_eq!(
            &Json::Null,
            at(at(&scalar_body, "vector_store"), "vector_store_metadata")
        );
    }
}

#[tokio::test]
async fn info_cached_metadata_with_invalid_json_is_the_catch_all() {
    let state = build(Setup {
        database: Some(FakeDatabase::new(Vec::new())),
        registry: Some(vec![cached("vs_a", "\"not json\"")]),
        ..Setup::default()
    });
    let (logs, _guard) = capture_logs(tracing::Level::ERROR);
    let (status, _headers, body) = post(&state, r#"{"vector_store_id":"vs_a"}"#, &admin()).await;
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

#[tokio::test]
async fn info_cached_blank_and_absent_fields_follow_the_model() {
    let stored = parsed(r#"{"vector_store_id":"","litellm_params":{"api_key":"k"}}"#);
    let state = build(Setup {
        database: Some(FakeDatabase::new(Vec::new())),
        registry: Some(vec![stored]),
        ..Setup::default()
    });
    let (status, _headers, body) = post(&state, r#"{"vector_store_id":""}"#, &admin()).await;
    assert_eq!(StatusCode::OK, status);
    let table = at(&body, "vector_store");
    assert_eq!("", text(at(table, "vector_store_id")));
    assert_eq!("", text(at(table, "custom_llm_provider")));
    assert_eq!(&Json::Bool(false), at(table, "is_config"));
    assert_eq!(&Json::Null, at(table, "vector_store_metadata"));
    assert_eq!(&Json::Null, at(table, "vector_store_name"));
}

// no cache hit: the row is fetched, kept whole and redacted, and its own 404 and 403 survive the catch-all
#[tokio::test]
async fn info_database_access_errors() {
    let database = FakeDatabase::new(vec![db_store("vs_a")]);
    let state = build(Setup {
        database: Some(database.clone()),
        ..Setup::default()
    });
    let (status, _headers, body) = post(&state, r#"{"vector_store_id":"vs_a"}"#, &admin()).await;
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
        ..Setup::default()
    });
    let (missing_status, _missing_headers, missing_body) = post(
        &missing_state,
        r#"{"vector_store_id":"vs_missing"}"#,
        &admin(),
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
        ..Setup::default()
    });
    let caller = internal("u1", Some("t1"));
    let (row_status, _row_headers, row_body) =
        post(&row_state, r#"{"vector_store_id":"vs_team"}"#, &caller).await;
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
        ..Setup::default()
    });
    let (status, _headers, body) = post(&state, r#"{"vector_store_id":"vs_b"}"#, &admin()).await;
    assert_eq!(StatusCode::OK, status);
    let table = at(&body, "vector_store");
    assert_eq!(&Json::Null, at(table, "litellm_params"));
    assert_eq!(&Json::Null, at(table, "vector_store_metadata"));
    assert_eq!(&Json::Bool(false), at(table, "is_config"));
}

#[tokio::test]
async fn info_denied_cached_record_is_403() {
    let database = FakeDatabase::new(vec![db_store("vs_team")]);
    let state = build(Setup {
        database: Some(database.clone()),
        registry: Some(vec![team_store("vs_team", "t9")]),
        ..Setup::default()
    });
    let (status, _headers, body) = post(
        &state,
        r#"{"vector_store_id":"vs_team"}"#,
        &internal("u1", Some("t1")),
    )
    .await;
    assert_eq!(StatusCode::FORBIDDEN, status);
    assert_eq!(
        "Access denied: You do not have permission to access this vector store",
        text(at(&body, "detail"))
    );
    assert!(database.asked().is_empty());
}

#[tokio::test]
async fn info_body_without_an_id_is_fastapi_validation() {
    let state = build(Setup::default());
    let (status, _headers, body) = post(&state, "{}", &admin()).await;
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
        settings: flags_blocked(),
        ..Setup::default()
    });
    let caller = internal("u1", None);
    for uri in [LIST_PATH, V1_LIST_PATH] {
        let (status, headers, body) = get(&state, uri, &caller).await;
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
    }
    let (blocked_status, _blocked_headers, _blocked_body) =
        post(&state, r#"{"vector_store_id":"vs_db"}"#, &caller).await;
    assert_eq!(StatusCode::FORBIDDEN, blocked_status);
    assert_eq!(StatusCode::OK, get(&state, LIST_PATH, &admin()).await.0);
    assert_eq!(StatusCode::OK, get(&state, LIST_PATH, &viewer()).await.0);

    let open = build(Setup {
        database: Some(FakeDatabase::new(vec![
            db_store("vs_free"),
            team_store("vs_team", "t1"),
        ])),
        settings: flags_open(),
        ..Setup::default()
    });
    let (status, _headers, body) = get(&open, LIST_PATH, &internal("u1", Some("t1"))).await;
    assert_eq!(StatusCode::OK, status);
    assert_eq!(vec!["vs_team".to_owned()], ids(&body));
}

#[tokio::test]
async fn team_admin_allow_flag_exempts_only_a_verified_admin() {
    let caller = internal("u1", None);
    let allowed = build(Setup {
        database: Some(FakeDatabase::new(vec![db_store("vs_db")])),
        privileges: true,
        settings: flags_blocked_team_admins(),
        ..Setup::default()
    });
    assert_eq!(StatusCode::OK, get(&allowed, LIST_PATH, &caller).await.0);
    let unverified = build(Setup {
        database: Some(FakeDatabase::new(vec![db_store("vs_db")])),
        settings: flags_blocked_team_admins(),
        ..Setup::default()
    });
    assert_eq!(
        StatusCode::FORBIDDEN,
        get(&unverified, LIST_PATH, &caller).await.0
    );
}
