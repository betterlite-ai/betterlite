use std::sync::Arc;

use axum::http::StatusCode;
use betterlite_db::budget_permission_rows::JsonData;
use rstest::rstest;

use super::support;

const DELETE: &str = "/config/pass_through_endpoint?endpoint_id=ep1";

#[tokio::test]
async fn delete_removes_the_entry_and_its_routes() {
    let saved = [
        support::stored("ep1", "/one"),
        support::stored("ep2", "/two"),
    ];
    let store = Arc::new(support::FakeStore::holding(support::entries(&saved)));
    let routes = support::registry();
    support::seed(&routes, &[("ep1", "/one"), ("ep2", "/two")]);
    let request = support::delete(DELETE, Some(&support::admin()));
    let reply = support::call(support::app(&store, &routes, &[]), request).await;
    assert_eq!(reply.status, StatusCode::OK);
    let deleted = support::endpoint_of(&reply);
    assert_eq!(deleted.id.as_deref(), Some("ep1"));
    assert_eq!(deleted.path, "/one");
    assert_eq!(
        support::endpoint_field(&reply, "is_from_config"),
        JsonData::Bool(false)
    );
    assert_eq!(store.stored_ids(), vec!["ep2".to_owned()]);
    assert_eq!(support::registered_ids(&routes), vec!["ep2".to_owned()]);
    assert_eq!(support::registered_paths(&routes), vec!["/two".to_owned()]);
}

#[rstest]
#[case::read_fails(true)]
#[case::null_field(false)]
#[tokio::test]
async fn delete_answers_400_when_no_endpoints_are_saved(#[case] read_fails: bool) {
    let mut fake = support::FakeStore::holding(JsonData::Null);
    fake.read_fails = read_fails;
    let store = Arc::new(fake);
    let routes = support::registry();
    let request = support::delete(DELETE, Some(&support::admin()));
    let reply = support::call(support::app(&store, &routes, &[]), request).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        reply.detail_error(),
        "There are no pass-through endpoints setup."
    );
    assert!(store.writes().is_empty());
}

#[tokio::test]
async fn delete_answers_400_for_an_unknown_endpoint_id() {
    let store = Arc::new(support::FakeStore::holding(support::entries(&[
        support::stored("other", "/other"),
    ])));
    let routes = support::registry();
    let request = support::delete(DELETE, Some(&support::admin()));
    let reply = support::call(support::app(&store, &routes, &[]), request).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        reply.detail_error(),
        "Endpoint with ID 'ep1' was not found in pass-through endpoint list."
    );
    assert!(store.writes().is_empty());
}

#[tokio::test]
async fn delete_answers_422_when_endpoint_id_is_missing() {
    let store = Arc::new(support::FakeStore::holding(support::entries(&[
        support::stored("ep1", "/one"),
    ])));
    let routes = support::registry();
    let request = support::delete("/config/pass_through_endpoint", Some(&support::admin()));
    let reply = support::call(support::app(&store, &routes, &[]), request).await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let issue = reply.issues().into_iter().next().expect("an issue");
    assert_eq!(issue.kind, "missing");
    assert_eq!(
        issue.loc,
        vec![
            JsonData::String("query".to_owned()),
            JsonData::String("endpoint_id".to_owned())
        ]
    );
    assert!(store.writes().is_empty());
}

#[tokio::test]
async fn delete_answers_401_before_the_query_is_read() {
    let store = Arc::new(support::FakeStore::holding(support::entries(&[
        support::stored("ep1", "/one"),
    ])));
    let routes = support::registry();
    let request = support::delete("/config/pass_through_endpoint", None);
    let reply = support::call(support::app(&store, &routes, &[]), request).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert!(store.writes().is_empty());
}

#[tokio::test]
async fn delete_keeps_the_entry_and_the_route_when_the_store_refuses() {
    let mut fake = support::FakeStore::holding(support::entries(&[support::stored("ep1", "/one")]));
    fake.write_fails = true;
    let store = Arc::new(fake);
    let routes = support::registry();
    support::seed(&routes, &[("ep1", "/one")]);
    let request = support::delete(DELETE, Some(&support::admin()));
    let reply = support::call(support::app(&store, &routes, &[]), request).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(reply.detail_error(), support::DB_NOT_CONNECTED);
    assert_eq!(support::registered_paths(&routes), vec!["/one".to_owned()]);
    assert_eq!(store.stored_ids(), vec!["ep1".to_owned()]);
}

#[tokio::test]
async fn delete_removes_a_subpath_route_and_its_wildcard() {
    let store = Arc::new(support::FakeStore::holding(support::entries(&[
        support::stored("ep1", "/one"),
    ])));
    let routes = support::registry();
    support::seed_subpath(&routes, "ep1", "/one");
    assert!(support::openai_routes(&routes).contains(&"/one/*".to_owned()));
    let request = support::delete(DELETE, Some(&support::admin()));
    let reply = support::call(support::app(&store, &routes, &[]), request).await;
    assert_eq!(reply.status, StatusCode::OK);
    let protected = support::openai_routes(&routes);
    assert!(!protected.contains(&"/one".to_owned()));
    assert!(!protected.contains(&"/one/*".to_owned()));
    assert!(support::registered(&routes).is_empty());
}

#[tokio::test]
async fn delete_answers_500_when_the_stored_value_is_not_a_list() {
    let store = Arc::new(support::FakeStore::holding(support::json(
        r#"{"id": "ep1"}"#,
    )));
    let routes = support::registry();
    let request = support::delete(DELETE, Some(&support::admin()));
    let reply = support::call(support::app(&store, &routes, &[]), request).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(store.writes().is_empty());
}

#[tokio::test]
async fn delete_answers_500_when_the_second_lookup_cannot_read_an_entry() {
    let mut saved = vec![support::json("42")];
    saved.push(support::json(&support::stored("ep1", "/one")));
    let store = Arc::new(support::FakeStore::holding(JsonData::Array(saved)));
    let routes = support::registry();
    let request = support::delete(DELETE, Some(&support::admin()));
    let reply = support::call(support::app(&store, &routes, &[]), request).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(store.writes().is_empty());
}
