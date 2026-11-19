use std::sync::Arc;

use axum::http::StatusCode;
use betterlite_auth::pass_through_registry::PassThroughRouteType;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::decisions::Json;
use rstest::rstest;

use super::support;

const CREATE: &str = "/config/pass_through_endpoint";

#[tokio::test]
async fn create_gives_a_missing_id_a_uuid_and_appends_the_entry() {
    let store = Arc::new(support::FakeStore::holding(JsonData::Null));
    let routes = support::registry();
    let body = r#"{"path": "/hook", "target": "https://target.example.com"}"#;
    let request = support::post(CREATE, body, Some(&support::admin()));
    let reply = support::call(support::app(&store, &routes, &[]), request).await;
    assert_eq!(reply.status, StatusCode::OK);
    let created = support::endpoint_of(&reply);
    let id = created.id.expect("a generated id");
    assert!(uuid::Uuid::parse_str(&id).is_ok());
    assert_eq!(created.path, "/hook");
    assert!(created.auth);
    assert_eq!(store.stored_ids(), vec![id.clone()]);
    let record = support::first_record(&store);
    assert_eq!(record.get("id"), Some(&JsonData::String(id.clone())));
    assert_eq!(record.get("is_from_config"), None);
    let registrations = support::registered(&routes);
    let registration = registrations.first().expect("one route");
    assert_eq!(registration.endpoint_id, id);
    assert_eq!(registration.path, "/hook");
    assert_eq!(registration.route_type, PassThroughRouteType::Exact);
}

#[tokio::test]
async fn create_starts_the_list_when_the_field_cannot_be_read() {
    let mut fake = support::FakeStore::holding(JsonData::Null);
    fake.read_fails = true;
    let store = Arc::new(fake);
    let routes = support::registry();
    let body = r#"{"id": "new", "path": "/hook", "target": "https://target.example.com"}"#;
    let request = support::post(CREATE, body, Some(&support::admin()));
    let reply = support::call(support::app(&store, &routes, &[]), request).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(store.stored_ids(), vec!["new".to_owned()]);
}

#[tokio::test]
async fn create_appends_to_the_stored_list_and_keeps_a_sent_id() {
    let saved = [
        support::stored("old", "/old"),
        support::stored("second", "/second"),
    ];
    let store = Arc::new(support::FakeStore::holding(support::entries(&saved)));
    let routes = support::registry();
    let body = r#"{"id": "new", "path": "/hook", "target": "https://target.example.com"}"#;
    let request = support::post(CREATE, body, Some(&support::admin()));
    let reply = support::call(support::app(&store, &routes, &[]), request).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        store.stored_ids(),
        vec!["old".to_owned(), "second".to_owned(), "new".to_owned()]
    );
    let created = support::endpoint_of(&reply);
    assert_eq!(created.id.as_deref(), Some("new"));
}

#[rstest]
#[case::omitted("", true)]
#[case::disabled(r#", "auth": false"#, false)]
#[case::enabled(r#", "auth": true"#, true)]
#[tokio::test]
async fn create_sets_the_auth_default_and_keeps_a_sent_value(
    #[case] extra: &str,
    #[case] expected: bool,
) {
    let store = Arc::new(support::FakeStore::holding(JsonData::Null));
    let routes = support::registry();
    let body = format!(r#"{{"path": "/hook", "target": "https://target.example.com"{extra}}}"#);
    let request = support::post(CREATE, &body, Some(&support::admin()));
    let reply = support::call(support::app(&store, &routes, &[]), request).await;
    assert_eq!(reply.status, StatusCode::OK);
    let created = support::endpoint_of(&reply);
    assert_eq!(created.auth, expected);
    let registrations = support::registered(&routes);
    assert_eq!(
        registrations.first().expect("one route").auth,
        Some(expected)
    );
}

#[tokio::test]
async fn create_registers_a_subpath_route_when_asked() {
    let store = Arc::new(support::FakeStore::holding(JsonData::Null));
    let routes = support::registry();
    let body = r#"{"id": "new", "path": "/hook", "target": "https://target.example.com", "include_subpath": true}"#;
    let request = support::post(CREATE, body, Some(&support::admin()));
    let reply = support::call(support::app(&store, &routes, &[]), request).await;
    assert_eq!(reply.status, StatusCode::OK);
    let registrations = support::registered(&routes);
    let registration = registrations.first().expect("one route");
    assert_eq!(registration.route_type, PassThroughRouteType::Subpath);
    assert_eq!(registration.path, "/hook");
}

#[tokio::test]
async fn create_stores_the_raw_headers_and_installs_the_resolved_ones() {
    let store = Arc::new(support::FakeStore::holding(JsonData::Null));
    let routes = support::registry();
    let body = r#"{"id": "new", "path": "/hook", "target": "https://target.example.com", "headers": {"X-Api-Key": "os.environ/UPSTREAM"}}"#;
    let request = support::post(CREATE, body, Some(&support::admin()));
    let reply = support::call(
        support::app(&store, &routes, &[("UPSTREAM", "secret")]),
        request,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let params = support::params_of(&routes, "new");
    let headers = params.custom_headers.expect("custom headers");
    assert_eq!(
        headers.get("X-Api-Key"),
        Some(&Json::Text("secret".to_owned()))
    );
    let created = support::endpoint_of(&reply);
    assert_eq!(
        created.headers.get("X-Api-Key"),
        Some(&support::json(r#""os.environ/UPSTREAM""#))
    );
}

#[tokio::test]
async fn create_carries_the_optional_fields_into_the_route() {
    let store = Arc::new(support::FakeStore::holding(JsonData::Null));
    let routes = support::registry();
    let body = r#"{"id": "new", "path": "/hook", "target": "https://target.example.com", "methods": ["POST"], "cost_per_request": 1.5, "timeout": 20, "default_query_params": {"api-version": "2024-01"}, "guardrails": {"pii": {"request_fields": ["query"]}}}"#;
    let request = support::post(CREATE, body, Some(&support::admin()));
    let reply = support::call(support::app(&store, &routes, &[]), request).await;
    assert_eq!(reply.status, StatusCode::OK);
    let params = support::params_of(&routes, "new");
    assert_eq!(params.timeout, Some(20.0));
    assert_eq!(
        params.cost_per_request,
        Some(rust_decimal::Decimal::new(15, 1))
    );
    let query = params.default_query_params.expect("query params");
    assert_eq!(
        query.get("api-version"),
        Some(&Json::Text("2024-01".to_owned()))
    );
    let guardrails = params.guardrails.expect("guardrails");
    let Some(Json::Object(pii)) = guardrails.get("pii") else {
        panic!("the pii entry");
    };
    assert_eq!(
        pii.get("request_fields"),
        Some(&Json::List(vec![Json::Text("query".to_owned())]))
    );
    assert_eq!(pii.get("response_fields"), Some(&Json::Null));
    let registrations = support::registered(&routes);
    assert_eq!(
        registrations.first().expect("one route").methods,
        vec!["POST".to_owned()]
    );
}

#[tokio::test]
async fn create_answers_401_without_a_caller() {
    let store = Arc::new(support::FakeStore::holding(JsonData::Null));
    let routes = support::registry();
    let body = r#"{"path": "/hook", "target": "https://target.example.com"}"#;
    let request = support::post(CREATE, body, None);
    let reply = support::call(support::app(&store, &routes, &[]), request).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert!(store.writes().is_empty());
    assert!(support::registered(&routes).is_empty());
}

#[tokio::test]
async fn create_refuses_a_stored_value_that_is_not_a_list() {
    let store = Arc::new(support::FakeStore::holding(support::json(
        r#"{"id": "ep1"}"#,
    )));
    let routes = support::registry();
    let body = r#"{"path": "/hook", "target": "https://target.example.com"}"#;
    let request = support::post(CREATE, body, Some(&support::admin()));
    let reply = support::call(support::app(&store, &routes, &[]), request).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(store.writes().is_empty());
    assert!(support::registered(&routes).is_empty());
}

#[tokio::test]
async fn create_installs_no_route_when_the_store_refuses() {
    let mut fake = support::FakeStore::holding(JsonData::Null);
    fake.write_fails = true;
    let store = Arc::new(fake);
    let routes = support::registry();
    let body = r#"{"path": "/hook", "target": "https://target.example.com"}"#;
    let request = support::post(CREATE, body, Some(&support::admin()));
    let reply = support::call(support::app(&store, &routes, &[]), request).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(reply.detail_error(), support::DB_NOT_CONNECTED);
    assert!(support::registered(&routes).is_empty());
}
