#![cfg(test)]

use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::json;

use super::error::ContainerOpError;
use super::test_support::Fixture;
use super::test_support::{
    CONTAINER_ID, CONTAINER_JSON, INITIAL_CALL_TYPE, StubHandler, container_object, deleted_result,
};

#[test]
fn mock_retrieve_returns_the_record_before_any_lookup() {
    let mut fixture = Fixture::new();
    fixture.container_id = "cntr_managed_abc".to_owned();
    fixture.litellm_metadata = Some(IndexMap::from([(
        "model_info".to_owned(),
        Json::Object(IndexMap::from([(
            "id".to_owned(),
            Json::Text("routed".to_owned()),
        )])),
    )]));
    fixture.mock_response = Some(json!({
        "id": "cntr_mock",
        "object": "container",
        "created_at": 1_i64,
        "status": "running"
    }));

    let object = fixture.retrieve().expect("the mock answers");

    assert_eq!(object.id, "cntr_mock");
    assert!(fixture.calls().is_empty());
    assert!(fixture.decoder.seen().is_empty());
    assert!(fixture.lookup.asked().is_empty());
    let state = fixture.logging.lock().unwrap();
    assert_eq!(state.call_type, INITIAL_CALL_TYPE);
    assert_eq!(state.model, "container");
}

#[test]
fn mock_delete_returns_the_result_before_any_lookup() {
    let mut fixture = Fixture::new();
    fixture.handler = StubHandler::deleting(deleted_result());
    fixture.litellm_metadata = Some(IndexMap::from([(
        "model_info".to_owned(),
        Json::Object(IndexMap::from([(
            "id".to_owned(),
            Json::Text("routed".to_owned()),
        )])),
    )]));
    fixture.mock_response = Some(Json::Text(
        r#"{"id":"cntr_mock","object":"container.deleted","deleted":true}"#.to_owned(),
    ));

    let deleted = fixture.delete().expect("the mock answers");

    assert_eq!(deleted.id, "cntr_mock");
    assert!(deleted.deleted);
    assert!(fixture.calls().is_empty());
    assert!(fixture.decoder.seen().is_empty());
    assert!(fixture.lookup.asked().is_empty());
}

#[test]
fn mock_a_json_string_is_parsed_into_the_record() {
    let mut fixture = Fixture::new();
    fixture.mock_response = Some(Json::Text(CONTAINER_JSON.to_owned()));

    let object = fixture.retrieve().expect("the string parses");

    assert_eq!(object, container_object());
    assert!(fixture.calls().is_empty());
}

#[test]
fn mock_an_explicit_null_response_falls_through_to_the_call() {
    let mut fixture = Fixture::new();
    fixture.mock_response = Some(Json::Null);

    let object = fixture.retrieve().expect("the call runs");

    assert_eq!(object.id, CONTAINER_ID);
    assert_eq!(fixture.calls().len(), 1);
    assert_eq!(fixture.decoder.seen().len(), 1);
}

#[test]
fn mock_a_response_that_is_not_json_is_a_value_error() {
    let mut fixture = Fixture::new();
    fixture.mock_response = Some(Json::Text("{ ".to_owned()));

    let failure = fixture.retrieve().expect_err("the mock does not parse");

    assert!(matches!(&failure.error, ContainerOpError::MockJson { .. }));
    assert!(failure.error.to_string().contains("not valid JSON"));
    assert!(matches!(&failure.class, ErrorClass::ApiConnection(_)));
    assert!(fixture.calls().is_empty());
    assert!(fixture.decoder.seen().is_empty());
}

// the mock error leaves through the sync dispatch before any decode, so the provider is still the caller's
#[test]
fn mock_a_bad_body_dispatches_with_the_callers_provider() {
    let mut fixture = Fixture::new();
    fixture.provider = Some("azure".to_owned());
    fixture.mock_response = Some(Json::Text("{ ".to_owned()));

    let failure = fixture.retrieve().expect_err("the mock does not parse");

    let ErrorClass::ApiConnection(context) = &failure.class else {
        panic!("expected a value error, got {:?}", failure.class);
    };
    assert_eq!(context.provider.as_deref(), Some("azure"));
}

#[test]
fn mock_a_body_that_does_not_match_the_record_names_the_retrieve_model() {
    let mut fixture = Fixture::new();
    fixture.mock_response = Some(json!({"id": 5}));

    let failure = fixture.retrieve().expect_err("the body does not decode");

    let ContainerOpError::MockModel { response_type, .. } = &failure.error else {
        panic!("expected a mock model error, got {:?}", failure.error);
    };
    assert_eq!(response_type.as_str(), "ContainerObject");
    assert!(matches!(&failure.class, ErrorClass::ApiConnection(_)));
    assert!(fixture.calls().is_empty());
}

#[test]
fn mock_a_delete_body_that_does_not_match_names_the_delete_model() {
    let mut fixture = Fixture::new();
    fixture.handler = StubHandler::deleting(deleted_result());
    fixture.mock_response = Some(json!({"id": "cntr_1"}));

    let failure = fixture.delete().expect_err("the body does not decode");

    let ContainerOpError::MockModel { response_type, .. } = &failure.error else {
        panic!("expected a mock model error, got {:?}", failure.error);
    };
    assert_eq!(response_type.as_str(), "DeleteContainerResult");
    assert!(fixture.calls().is_empty());
}
