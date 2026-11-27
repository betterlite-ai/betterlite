use super::super::service::Operation;
use super::support::{
    Fixture, REQUEST_TIMEOUT, Spy, Which, attempt, call, json, listing, object, params,
};
use crate::api::vector_store_http_write::service::ClientKind;

#[tokio::test]
async fn list_attachment_query() {
    let mut fixture = Fixture::new();
    fixture.after = Some(String::from("file_a"));
    fixture.before = Some(String::from("file_z"));
    fixture.filter = Some(String::from("failed"));
    fixture.limit = Some(0);
    fixture.order = Some(String::from("asc"));
    fixture.extra_query = Some(object(r#"{"limit":2,"trace":"x"}"#));
    fixture.extra_body = Some(object(r#"{"ignored":"y"}"#));
    let spy = Spy::replying(listing());
    let seen = attempt(Which::List, &spy, &fixture, None, REQUEST_TIMEOUT)
        .await
        .expect("the attachment call succeeds");
    assert_eq!(seen.operation, Operation::List);
    assert_eq!(seen.mode, ClientKind::Sync);
    assert_eq!(
        seen.fields,
        params(r#"{"after":"file_a","before":"file_z","filter":"failed","limit":0,"order":"asc"}"#)
    );
    assert_eq!(seen.extra_query, Some(object(r#"{"limit":2,"trace":"x"}"#)));
    assert_eq!(seen.extra_body, None);
    let answer = call(Which::List, &spy, &fixture, None, REQUEST_TIMEOUT)
        .await
        .expect("the attachment call answers");
    assert_eq!(answer, listing());
}

#[tokio::test]
async fn list_attachment_async_query() {
    let mut fixture = Fixture::new();
    fixture.after = Some(String::from("file_a"));
    fixture.before = Some(String::from("file_z"));
    fixture.filter = Some(String::from("failed"));
    fixture.limit = Some(0);
    fixture.order = Some(String::from("asc"));
    fixture.extra_query = Some(object(r#"{"limit":2,"trace":"x"}"#));
    fixture.extra_body = Some(object(r#"{"ignored":"y"}"#));
    let spy = Spy::replying(listing());
    let seen = attempt(Which::Alist, &spy, &fixture, None, REQUEST_TIMEOUT)
        .await
        .expect("the attachment call succeeds");
    assert_eq!(seen.operation, Operation::List);
    assert_eq!(seen.mode, ClientKind::Asynchronous);
    assert_eq!(
        seen.fields,
        params(r#"{"after":"file_a","before":"file_z","filter":"failed","limit":0,"order":"asc"}"#)
    );
    assert_eq!(seen.extra_query, Some(object(r#"{"limit":2,"trace":"x"}"#)));
    assert_eq!(seen.extra_body, None);
    assert!(!seen.litellm_params.contains_key("alist"));
}

#[tokio::test]
async fn list_attachment_defaults() {
    let fixture = Fixture::new();
    let spy = Spy::replying(listing());
    let seen = attempt(Which::List, &spy, &fixture, None, REQUEST_TIMEOUT)
        .await
        .expect("the attachment call succeeds");
    assert_eq!(seen.fields, params("{}"));
    assert_eq!(seen.extra_query, None);
    assert_eq!(seen.extra_body, None);
}

#[tokio::test]
async fn list_attachment_empty_after_survives() {
    let mut fixture = Fixture::new();
    fixture.after = Some(String::new());
    let spy = Spy::replying(listing());
    let seen = attempt(Which::List, &spy, &fixture, None, REQUEST_TIMEOUT)
        .await
        .expect("the attachment call succeeds");
    assert_eq!(seen.fields, params(r#"{"after":""}"#));
}

#[tokio::test]
async fn list_attachment_field_types() {
    let mut fixture = Fixture::new();
    fixture.kwargs.insert(String::from("api_base"), json("42"));
    let spy = Spy::replying(listing());
    let failure = attempt(Which::List, &spy, &fixture, None, REQUEST_TIMEOUT)
        .await
        .expect_err("the attachment call fails");
    assert_eq!(failure.class, "APIConnectionError");
    assert_eq!(failure.status, 500);
    assert_eq!(failure.message, "the api_base field must be a string");
    assert_eq!(spy.call_count(), 0);
}
