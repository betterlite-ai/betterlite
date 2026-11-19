use std::sync::Arc;

use axum::http::StatusCode;
use betterlite_auth::pass_through_registry::PassThroughRouteType;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::decisions::Json;
use rstest::rstest;

use super::support;

const UPDATE: &str = "/config/pass_through_endpoint/ep1";
const BODY: &str = r#"{"path": "/two", "target": "https://new.example.com"}"#;

#[tokio::test]
async fn update_merges_only_the_sent_fields_over_the_stored_entry() {
    let entry = support::stored_with(
        "ep1",
        "/one",
        r#", "auth": false, "headers": {"x-key": "v"}, "methods": ["GET"], "timeout": 15"#,
    );
    let store = Arc::new(support::FakeStore::holding(support::entries(&[entry])));
    let routes = support::registry();
    support::seed(&routes, &[("ep1", "/one")]);
    let request = support::post(UPDATE, BODY, Some(&support::admin()));
    let reply = support::call(support::app(&store, &routes, &[]), request).await;
    assert_eq!(reply.status, StatusCode::OK);
    let updated = support::endpoint_of(&reply);
    assert_eq!(updated.id.as_deref(), Some("ep1"));
    assert_eq!(updated.path, "/two");
    assert_eq!(updated.target, "https://new.example.com");
    assert!(!updated.auth);
    assert_eq!(updated.timeout, Some(15.0_f64));
    assert_eq!(updated.methods, Some(vec!["GET".to_owned()]));
    assert_eq!(updated.headers.get("x-key"), Some(&support::json(r#""v""#)));
    assert_eq!(
        support::endpoint_field(&reply, "is_from_config"),
        JsonData::Bool(false)
    );
    let record = support::first_record(&store);
    assert_eq!(record.get("is_from_config"), None);
    assert_eq!(record.get("auth"), Some(&JsonData::Bool(false)));
    assert_eq!(store.stored_ids(), vec!["ep1".to_owned()]);
}

#[tokio::test]
async fn update_replaces_a_sent_field_and_a_null_keeps_the_stored_one() {
    let entry = support::stored_with(
        "ep1",
        "/one",
        r#", "headers": {"x-key": "old"}, "timeout": 15"#,
    );
    let store = Arc::new(support::FakeStore::holding(support::entries(&[entry])));
    let routes = support::registry();
    let body = r#"{"path": "/one", "target": "https://t.example.com", "headers": {"y-key": "new"}, "timeout": null, "auth": true}"#;
    let request = support::post(UPDATE, body, Some(&support::admin()));
    let reply = support::call(support::app(&store, &routes, &[]), request).await;
    assert_eq!(reply.status, StatusCode::OK);
    let updated = support::endpoint_of(&reply);
    assert!(updated.auth);
    assert!(updated.headers.contains_key("y-key"));
    assert!(!updated.headers.contains_key("x-key"));
    assert_eq!(updated.timeout, Some(15.0_f64));
}

#[rstest]
#[case::read_fails(true)]
#[case::null_field(false)]
#[tokio::test]
async fn update_answers_404_when_no_endpoints_are_saved(#[case] read_fails: bool) {
    let mut fake = support::FakeStore::holding(JsonData::Null);
    fake.read_fails = read_fails;
    let store = Arc::new(fake);
    let routes = support::registry();
    let request = support::post(UPDATE, BODY, Some(&support::admin()));
    let reply = support::call(support::app(&store, &routes, &[]), request).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.detail_error(), "No pass-through endpoints found");
    assert!(store.writes().is_empty());
}

#[tokio::test]
async fn update_answers_404_for_an_unknown_endpoint_id() {
    let other = support::stored("other", "/other");
    let store = Arc::new(support::FakeStore::holding(support::entries(&[other])));
    let routes = support::registry();
    let request = support::post(UPDATE, BODY, Some(&support::admin()));
    let reply = support::call(support::app(&store, &routes, &[]), request).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.detail_error(), "Endpoint with ID 'ep1' not found");
    assert!(store.writes().is_empty());
}

#[tokio::test]
async fn update_answers_500_when_the_stored_value_is_not_a_list() {
    let store = Arc::new(support::FakeStore::holding(support::json(
        r#"{"id": "ep1"}"#,
    )));
    let routes = support::registry();
    let request = support::post(UPDATE, BODY, Some(&support::admin()));
    let reply = support::call(support::app(&store, &routes, &[]), request).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(reply.error().message, "Internal server error");
    assert_eq!(reply.error().kind, "internal_server_error");
}

#[tokio::test]
async fn update_answers_500_for_a_malformed_stored_entry() {
    let broken = r#"{"id": "ep1", "path": "/one"}"#.to_owned();
    let store = Arc::new(support::FakeStore::holding(support::entries(&[broken])));
    let routes = support::registry();
    let request = support::post(UPDATE, BODY, Some(&support::admin()));
    let reply = support::call(support::app(&store, &routes, &[]), request).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(store.writes().is_empty());
}

#[tokio::test]
async fn update_answers_422_when_the_body_misses_a_field() {
    let store = Arc::new(support::FakeStore::holding(support::entries(&[
        support::stored("ep1", "/one"),
    ])));
    let routes = support::registry();
    let request = support::post(UPDATE, r#"{"path": "/two"}"#, Some(&support::admin()));
    let reply = support::call(support::app(&store, &routes, &[]), request).await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let issue = reply.issues().into_iter().next().expect("an issue");
    assert_eq!(issue.kind, "missing");
    assert_eq!(issue.msg, "Field required");
    assert_eq!(
        issue.loc,
        vec![
            JsonData::String("body".to_owned()),
            JsonData::String("target".to_owned())
        ]
    );
}

#[rstest]
#[case::malformed("{not json", "JSON decode error")]
#[case::trailing(r#"{"path": "/two", "target": "https://t"}{}"#, "JSON decode error")]
#[case::empty("", "Field required")]
#[tokio::test]
async fn update_answers_422_for_a_body_that_is_not_one_json_object(
    #[case] body: &str,
    #[case] msg: &str,
) {
    let store = Arc::new(support::FakeStore::holding(JsonData::Null));
    let routes = support::registry();
    let request = support::post(UPDATE, body, Some(&support::admin()));
    let reply = support::call(support::app(&store, &routes, &[]), request).await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let issue = reply.issues().into_iter().next().expect("an issue");
    assert_eq!(issue.msg, msg);
    assert_eq!(issue.loc, vec![JsonData::String("body".to_owned())]);
}

#[tokio::test]
async fn update_answers_401_without_a_caller() {
    let store = Arc::new(support::FakeStore::holding(JsonData::Null));
    let routes = support::registry();
    let request = support::post(UPDATE, BODY, None);
    let reply = support::call(support::app(&store, &routes, &[]), request).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert_eq!(reply.error().kind, "auth_error");
    assert!(store.writes().is_empty());
}

#[tokio::test]
async fn update_keeps_the_store_and_the_old_route_when_the_store_refuses() {
    let mut fake = support::FakeStore::holding(support::entries(&[support::stored("ep1", "/one")]));
    fake.write_fails = true;
    let store = Arc::new(fake);
    let routes = support::registry();
    support::seed(&routes, &[("ep1", "/one")]);
    let request = support::post(UPDATE, BODY, Some(&support::admin()));
    let reply = support::call(support::app(&store, &routes, &[]), request).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(reply.detail_error(), support::DB_NOT_CONNECTED);
    assert_eq!(support::registered_paths(&routes), vec!["/one".to_owned()]);
    assert!(store.writes().is_empty());
}

#[tokio::test]
async fn update_keeps_the_store_and_the_old_route_when_a_header_cannot_resolve() {
    let entry = support::stored_with(
        "ep1",
        "/one",
        r#", "headers": {"LANGFUSE_PUBLIC_KEY": "public"}"#,
    );
    let store = Arc::new(support::FakeStore::holding(support::entries(&[entry])));
    let routes = support::registry();
    support::seed(&routes, &[("ep1", "/one")]);
    let request = support::post(UPDATE, BODY, Some(&support::admin()));
    let reply = support::call(support::app(&store, &routes, &[]), request).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(support::registered_paths(&routes), vec!["/one".to_owned()]);
    assert!(store.writes().is_empty());
}

#[tokio::test]
async fn update_reinstalls_the_route_and_drops_the_old_one() {
    let entry = support::stored_with("ep1", "/one", r#", "auth": true"#);
    let store = Arc::new(support::FakeStore::holding(support::entries(&[entry])));
    let routes = support::registry();
    support::seed(&routes, &[("ep1", "/one")]);
    let request = support::post(UPDATE, BODY, Some(&support::admin()));
    let reply = support::call(support::app(&store, &routes, &[]), request).await;
    assert_eq!(reply.status, StatusCode::OK);
    let registrations = support::registered(&routes);
    assert_eq!(registrations.len(), 1);
    let registration = registrations.first().expect("one route");
    assert_eq!(registration.path, "/two");
    assert_eq!(registration.endpoint_id, "ep1");
    assert_eq!(registration.route_type, PassThroughRouteType::Exact);
    assert_eq!(registration.auth, Some(true));
    assert!(!support::openai_routes(&routes).contains(&"/one".to_owned()));
    let params = support::params_of(&routes, "ep1");
    assert_eq!(params.target, "https://new.example.com");
    assert_eq!(params.forward_headers, None);
    assert_eq!(params.merge_query_params, None);
    assert_eq!(params.config_file_path, None);
}

#[tokio::test]
async fn update_installs_a_subpath_route_when_the_entry_asks_for_it() {
    let entry = support::stored_with("ep1", "/one", r#", "include_subpath": true"#);
    let store = Arc::new(support::FakeStore::holding(support::entries(&[entry])));
    let routes = support::registry();
    support::seed(&routes, &[("ep1", "/one")]);
    let request = support::post(UPDATE, BODY, Some(&support::admin()));
    let reply = support::call(support::app(&store, &routes, &[]), request).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert!(support::endpoint_of(&reply).include_subpath);
    let registrations = support::registered(&routes);
    let registration = registrations.first().expect("one route");
    assert_eq!(registration.route_type, PassThroughRouteType::Subpath);
    assert_eq!(registration.path, "/two");
}

#[tokio::test]
async fn update_installs_the_route_under_the_id_the_body_sends() {
    let store = Arc::new(support::FakeStore::holding(support::entries(&[
        support::stored("ep1", "/one"),
    ])));
    let routes = support::registry();
    support::seed(&routes, &[("ep1", "/one")]);
    let body = r#"{"id": "renamed", "path": "/two", "target": "https://new.example.com"}"#;
    let request = support::post(UPDATE, body, Some(&support::admin()));
    let reply = support::call(support::app(&store, &routes, &[]), request).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(support::registered_ids(&routes), vec!["renamed".to_owned()]);
    assert_eq!(store.stored_ids(), vec!["renamed".to_owned()]);
}

#[tokio::test]
async fn update_resolves_an_os_environ_header_before_installing_the_route() {
    let entry = support::stored_with(
        "ep1",
        "/one",
        r#", "headers": {"Authorization": "os.environ/UPSTREAM_KEY"}"#,
    );
    let store = Arc::new(support::FakeStore::holding(support::entries(&[entry])));
    let routes = support::registry();
    let request = support::post(UPDATE, BODY, Some(&support::admin()));
    let reply = support::call(
        support::app(&store, &routes, &[("UPSTREAM_KEY", "secret")]),
        request,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let params = support::params_of(&routes, "ep1");
    let headers = params.custom_headers.expect("custom headers");
    assert_eq!(
        headers.get("Authorization"),
        Some(&Json::Text("secret".to_owned()))
    );
}

#[tokio::test]
async fn update_registers_the_optional_fields_in_the_route() {
    let entry = support::stored("ep1", "/one");
    let store = Arc::new(support::FakeStore::holding(support::entries(&[entry])));
    let routes = support::registry();
    let body = r#"{"path": "/two", "target": "https://new.example.com", "methods": ["POST"], "cost_per_request": 0.25, "timeout": 30, "default_query_params": {"api-version": "2024-01"}}"#;
    let request = support::post(UPDATE, body, Some(&support::admin()));
    let reply = support::call(support::app(&store, &routes, &[]), request).await;
    assert_eq!(reply.status, StatusCode::OK);
    let params = support::params_of(&routes, "ep1");
    assert_eq!(params.timeout, Some(30.0_f64));
    assert_eq!(
        params.cost_per_request,
        Some(rust_decimal::Decimal::new(25, 2))
    );
    let query = params.default_query_params.expect("query params");
    assert_eq!(
        query.get("api-version"),
        Some(&Json::Text("2024-01".to_owned()))
    );
    assert_eq!(
        support::endpoint_field(&reply, "cost_per_request"),
        support::json("0.25")
    );
}
