#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json;

use super::support::{
    CALL_ID, CONTAINER_ID, CONTAINER_JSON, Fixture, INITIAL_CALL_TYPE, LIST_JSON, StubLookup,
    container_list, container_object, create_call, json, list_call, routing_metadata,
};

#[test]
fn mock_create_a_json_string_answers_without_a_provider_or_a_handler() {
    let mut fixture = Fixture::new();
    fixture.lookup = StubLookup::empty();
    let mut call = create_call();
    call.custom_llm_provider = "acme".to_owned();
    call.mock_response = Some(Json::Text(CONTAINER_JSON.to_owned()));

    let container = fixture.create(&call).expect("the mock answers");

    assert_eq!(container, container_object());
    assert!(fixture.lookup.asked().is_empty());
    assert!(fixture.handler.creates().is_empty());
    let state = fixture.state();
    assert_eq!(state.call_type, INITIAL_CALL_TYPE);
    assert!(state.optional_params.is_empty());
    assert!(state.litellm_params.is_empty());
}

#[test]
fn mock_create_a_json_object_answers_like_a_json_string() {
    let fixture = Fixture::new();
    let mut call = create_call();
    call.mock_response = Some(json(CONTAINER_JSON));

    let container = fixture.create(&call).expect("the mock answers");

    assert_eq!(container, container_object());
    assert!(fixture.lookup.asked().is_empty());
    assert!(fixture.handler.creates().is_empty());
}

#[test]
fn mock_create_skips_the_id_encoding() {
    let fixture = Fixture::new();
    let mut call = create_call();
    call.mock_response = Some(json(CONTAINER_JSON));
    call.litellm_metadata = Some(routing_metadata("gpt-4o"));
    call.extra_body = Some(BTreeMap::from([(
        "target_model_names".to_owned(),
        "gpt-4o-mini".to_owned(),
    )]));

    let container = fixture.create(&call).expect("the mock answers");

    assert_eq!(container.id, CONTAINER_ID);
}

#[test]
fn mock_create_an_unparsable_string_is_a_value_error() {
    let fixture = Fixture::new();
    let mut call = create_call();
    call.mock_response = Some(Json::Text("not json".to_owned()));

    let failure = fixture.create(&call).expect_err("the mock does not parse");

    let ErrorClass::ApiConnection(context) = &failure.class else {
        panic!("expected an api connection, got {:?}", failure.class);
    };
    assert_eq!(context.provider.as_deref(), Some("openai"));
    assert_eq!(context.model.as_deref(), Some(""));
    assert!(context.message.contains("not valid JSON"));
    assert!(fixture.handler.creates().is_empty());
}

#[test]
fn mock_create_a_wrong_shape_is_a_value_error() {
    let fixture = Fixture::new();
    let mut call = create_call();
    call.mock_response = Some(Json::Text(r#"{"object":"list"}"#.to_owned()));

    let failure = fixture.create(&call).expect_err("the mock misses its id");

    let ErrorClass::ApiConnection(context) = &failure.class else {
        panic!("expected an api connection, got {:?}", failure.class);
    };
    assert!(
        context
            .message
            .contains("does not match a container object")
    );
    assert!(fixture.lookup.asked().is_empty());
    assert!(fixture.handler.creates().is_empty());
}

#[test]
fn mock_create_a_null_mock_is_no_mock() {
    let fixture = Fixture::new();
    let mut call = create_call();
    call.mock_response = Some(Json::Null);

    fixture.create(&call).expect("the handler answers");

    assert_eq!(fixture.lookup.asked().len(), 1);
    assert_eq!(fixture.handler.creates().len(), 1);
    assert_eq!(fixture.state().call_type, "create_container");
}

#[test]
fn mock_list_a_json_string_answers_without_a_provider_or_a_handler() {
    let mut fixture = Fixture::new();
    fixture.lookup = StubLookup::empty();
    let mut call = list_call();
    call.custom_llm_provider = "anthropic".to_owned();
    call.mock_response = Some(Json::Text(LIST_JSON.to_owned()));

    let response = fixture.list(&call).expect("the mock answers");

    assert_eq!(response, container_list());
    assert!(fixture.lookup.asked().is_empty());
    assert!(fixture.handler.lists().is_empty());
    let state = fixture.state();
    assert_eq!(state.call_type, INITIAL_CALL_TYPE);
    assert!(state.litellm_params.is_empty());
}

#[test]
fn mock_list_a_json_object_answers_like_a_json_string() {
    let fixture = Fixture::new();
    let mut call = list_call();
    call.mock_response = Some(json(LIST_JSON));

    let response = fixture.list(&call).expect("the mock answers");

    assert_eq!(response, container_list());
    assert!(fixture.lookup.asked().is_empty());
    assert!(fixture.handler.lists().is_empty());
}

#[test]
fn mock_list_skips_the_id_encoding() {
    let fixture = Fixture::new();
    let mut call = list_call();
    call.mock_response = Some(json(LIST_JSON));
    call.litellm_metadata = Some(routing_metadata("gpt-4o"));
    call.litellm_call_id = Some(CALL_ID.to_owned());

    let response = fixture.list(&call).expect("the mock answers");

    assert_eq!(response.data[0].id, CONTAINER_ID);
}

#[test]
fn mock_list_an_unparsable_string_is_a_value_error() {
    let fixture = Fixture::new();
    let mut call = list_call();
    call.mock_response = Some(Json::Text("{ ".to_owned()));

    let failure = fixture.list(&call).expect_err("the mock does not parse");

    let ErrorClass::ApiConnection(context) = &failure.class else {
        panic!("expected an api connection, got {:?}", failure.class);
    };
    assert!(context.message.contains("not valid JSON"));
    assert!(fixture.handler.lists().is_empty());
}

#[test]
fn mock_list_a_wrong_shape_is_a_value_error() {
    let fixture = Fixture::new();
    let mut call = list_call();
    call.mock_response = Some(Json::Text(r#"{"object":"list"}"#.to_owned()));

    let failure = fixture.list(&call).expect_err("the mock misses its data");

    let ErrorClass::ApiConnection(context) = &failure.class else {
        panic!("expected an api connection, got {:?}", failure.class);
    };
    assert!(context.message.contains("does not match a container list"));
    assert!(fixture.lookup.asked().is_empty());
    assert!(fixture.handler.lists().is_empty());
}

#[test]
fn mock_list_a_null_mock_is_no_mock() {
    let fixture = Fixture::new();
    let mut call = list_call();
    call.mock_response = Some(Json::Null);

    fixture.list(&call).expect("the handler answers");

    assert_eq!(fixture.lookup.asked().len(), 1);
    assert_eq!(fixture.handler.lists().len(), 1);
    assert_eq!(fixture.state().call_type, "list_containers");
}
