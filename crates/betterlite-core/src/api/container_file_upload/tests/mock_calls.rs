#![cfg(test)]

use std::sync::PoisonError;

use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json;

use super::test_support::{Fixture, INITIAL_CALL_TYPE, OBJECT_BODY, StubExecutor, object};
use super::upload_container_file;

#[test]
fn mock_upload_returns_the_object_a_json_string_describes() {
    let mut fixture = Fixture::new();
    fixture.kwargs.insert(
        "mock_response".to_owned(),
        Json::Text(OBJECT_BODY.to_owned()),
    );
    let request = fixture.request();
    let executor = StubExecutor::replying(object("unused"));

    let reply =
        upload_container_file(&executor, fixture.deps(), &request).expect("the mock succeeds");

    assert_eq!(reply, object("file_1"));
    assert!(!executor.was_called());
}

#[test]
fn mock_upload_takes_an_object_as_well_as_a_string() {
    let mut fixture = Fixture::new();
    let parsed = serde_json::from_str::<Json>(OBJECT_BODY).expect("the fixture parses as JSON");
    fixture.kwargs.insert("mock_response".to_owned(), parsed);
    let request = fixture.request();
    let executor = StubExecutor::replying(object("unused"));

    let reply =
        upload_container_file(&executor, fixture.deps(), &request).expect("the mock succeeds");

    assert_eq!(reply, object("file_1"));
    assert!(!executor.was_called());
}

#[test]
fn mock_upload_skips_the_provider_lookup_and_the_upload_logging() {
    let mut fixture = Fixture::new();
    fixture.provider = Some("bogus".to_owned());
    fixture.kwargs.insert(
        "mock_response".to_owned(),
        Json::Text(OBJECT_BODY.to_owned()),
    );
    let request = fixture.request();
    let executor = StubExecutor::replying(object("unused"));

    let reply = upload_container_file(&executor, fixture.deps(), &request)
        .expect("a mock needs no provider config");

    assert_eq!(reply, object("file_1"));
    assert!(!executor.was_called());
    let state = fixture
        .logging
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    assert_eq!(state.call_type, INITIAL_CALL_TYPE);
    assert_eq!(state.model, "gpt-4o");
    assert!(state.optional_params.is_empty());
    assert!(!state.litellm_params.contains_key("litellm_call_id"));
}

#[test]
fn mock_upload_with_a_null_response_is_not_a_mock() {
    let mut fixture = Fixture::new();
    fixture
        .kwargs
        .insert("mock_response".to_owned(), Json::Null);
    let request = fixture.request();
    let executor = StubExecutor::replying(object("file_1"));

    let reply =
        upload_container_file(&executor, fixture.deps(), &request).expect("the upload runs");

    assert_eq!(reply, object("file_1"));
    assert!(executor.was_called());
}

#[test]
fn mock_upload_with_invalid_json_is_a_mapped_failure() {
    let mut fixture = Fixture::new();
    fixture.kwargs.insert(
        "mock_response".to_owned(),
        Json::Text("not json".to_owned()),
    );
    let request = fixture.request();
    let executor = StubExecutor::replying(object("unused"));

    let error = upload_container_file(&executor, fixture.deps(), &request)
        .expect_err("the mock is not JSON");

    assert!(matches!(error.class, ErrorClass::ApiConnection(_)));
    assert!(!executor.was_called());
}

#[test]
fn mock_upload_that_is_not_a_container_file_is_a_mapped_failure() {
    let mut fixture = Fixture::new();
    fixture.kwargs.insert(
        "mock_response".to_owned(),
        Json::Text(r#"{"id": "file_1"}"#.to_owned()),
    );
    let request = fixture.request();
    let executor = StubExecutor::replying(object("unused"));

    let error = upload_container_file(&executor, fixture.deps(), &request)
        .expect_err("the mock misses required fields");

    assert!(matches!(error.class, ErrorClass::ApiConnection(_)));
    assert!(!executor.was_called());
}
