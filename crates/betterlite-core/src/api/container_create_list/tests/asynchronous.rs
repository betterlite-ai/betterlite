#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_error::error::ErrorClass;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_openai::api::container_contracts::error::BaseLlmException;

use super::support::{
    CONTAINER_ID, CONTAINER_JSON, Fixture, HandlerForm, LIST_JSON, StubLookup, container_list,
    create_call, json, list_call, routing_metadata,
};
use crate::api::container_collection_http::ContainerCollectionError;
use crate::api::response_id_codecs::container::build_container_id;

fn provider_error(status_code: u16, message: &str) -> BaseLlmException {
    BaseLlmException {
        status_code,
        message: message.to_owned(),
        headers: BTreeMap::new(),
    }
}

#[tokio::test]
async fn async_create_returns_the_encoded_object_through_the_async_handler() {
    let fixture = Fixture::new();
    let mut call = create_call();
    call.name = "store".to_owned();
    call.litellm_metadata = Some(routing_metadata("gpt-4o"));

    let container = fixture.acreate(&call).await.expect("the handler answers");

    assert_eq!(
        container.id,
        build_container_id(Some("openai"), Some("gpt-4o"), CONTAINER_ID)
    );
    let creates = fixture.handler.creates();
    assert_eq!(creates.len(), 1);
    assert_eq!(creates[0].base.form, HandlerForm::Async);
    assert_eq!(creates[0].base.call_type, "create_container");
    assert_eq!(creates[0].name, "store");
}

#[tokio::test]
async fn async_create_answers_a_mock_before_the_async_handler() {
    let fixture = Fixture::new();
    let mut call = create_call();
    call.custom_llm_provider = "acme".to_owned();
    call.mock_response = Some(Json::Text(CONTAINER_JSON.to_owned()));

    let container = fixture.acreate(&call).await.expect("the mock answers");

    assert_eq!(container.id, CONTAINER_ID);
    assert!(fixture.lookup.asked().is_empty());
    assert!(fixture.handler.creates().is_empty());
}

#[tokio::test]
async fn async_create_encodes_after_the_async_handler_answers() {
    let fixture = Fixture::new();
    let mut call = create_call();
    let mut metadata = routing_metadata("gpt-4o");
    metadata.insert("model_info".to_owned(), JsonValue::Text("wrong".to_owned()));
    call.litellm_metadata = Some(metadata);

    let failure = fixture
        .acreate(&call)
        .await
        .expect_err("the metadata is not an object");

    let ErrorClass::BadRequest(details) = &failure.class else {
        panic!("expected a bad request, got {:?}", failure.class);
    };
    assert_eq!(details.status.0, 400);
    assert_eq!(
        details.body_param.as_deref(),
        Some("litellm_metadata.model_info")
    );
    // the encode step runs only after the async handler answered
    assert_eq!(fixture.handler.creates().len(), 1);
    assert_eq!(fixture.handler.creates()[0].base.form, HandlerForm::Async);
}

#[tokio::test]
async fn async_create_a_handler_failure_is_mapped() {
    let mut fixture = Fixture::new();
    fixture
        .handler
        .fail_with(ContainerCollectionError::Provider(provider_error(
            404,
            "No such container",
        )));

    let failure = fixture
        .acreate(&create_call())
        .await
        .expect_err("the handler failed the call");

    let ErrorClass::NotFound(details) = &failure.class else {
        panic!("expected a not found, got {:?}", failure.class);
    };
    assert_eq!(details.status.0, 404);
    assert_eq!(details.provider.as_deref(), Some("openai"));
}

#[tokio::test]
async fn async_list_returns_the_response_through_the_async_handler() {
    let fixture = Fixture::new();
    let mut call = list_call();
    call.after = Some("cntr_0".to_owned());
    call.litellm_metadata = Some(routing_metadata("gpt-4o"));
    call.extra_body = Some(BTreeMap::from([(
        "target_model_names".to_owned(),
        "gpt-4o-mini".to_owned(),
    )]));

    let response = fixture.alist(&call).await.expect("the handler answers");

    assert_eq!(response.data[0].id, CONTAINER_ID);
    assert_eq!(response.first_id.as_deref(), Some(CONTAINER_ID));
    let lists = fixture.handler.lists();
    assert_eq!(lists.len(), 1);
    assert_eq!(lists[0].base.form, HandlerForm::Async);
    assert_eq!(lists[0].base.call_type, "list_containers");
    assert_eq!(lists[0].after.as_deref(), Some("cntr_0"));
}

#[tokio::test]
async fn async_list_answers_a_mock_before_the_async_handler() {
    let mut fixture = Fixture::new();
    fixture.lookup = StubLookup::empty();
    let mut call = list_call();
    call.custom_llm_provider = "acme".to_owned();
    call.mock_response = Some(json(LIST_JSON));

    let response = fixture.alist(&call).await.expect("the mock answers");

    assert_eq!(response, container_list());
    assert!(fixture.lookup.asked().is_empty());
    assert!(fixture.handler.lists().is_empty());
}

#[tokio::test]
async fn async_list_a_handler_failure_is_mapped() {
    let mut fixture = Fixture::new();
    fixture
        .handler
        .fail_with(ContainerCollectionError::Provider(provider_error(
            500, "boom",
        )));

    let failure = fixture
        .alist(&list_call())
        .await
        .expect_err("the handler failed the call");

    let ErrorClass::InternalServer(context) = &failure.class else {
        panic!("expected an internal server, got {:?}", failure.class);
    };
    assert_eq!(context.provider.as_deref(), Some("openai"));
    assert_eq!(context.model.as_deref(), Some(""));
}

#[tokio::test]
async fn async_create_matches_the_blocking_result() {
    let blocking = Fixture::new();
    let asynchronous = Fixture::new();
    let mut call = create_call();
    call.name = "store".to_owned();
    call.litellm_metadata = Some(routing_metadata("gpt-4o"));

    let expected = blocking.create(&call).expect("the handler answers");
    let actual = asynchronous
        .acreate(&call)
        .await
        .expect("the handler answers");

    assert_eq!(actual, expected);
}

#[tokio::test]
async fn async_list_matches_the_blocking_result() {
    let blocking = Fixture::new();
    let asynchronous = Fixture::new();
    let mut call = list_call();
    call.limit = Some(5);

    let expected = blocking.list(&call).expect("the handler answers");
    let actual = asynchronous
        .alist(&call)
        .await
        .expect("the handler answers");

    assert_eq!(actual, expected);
}
