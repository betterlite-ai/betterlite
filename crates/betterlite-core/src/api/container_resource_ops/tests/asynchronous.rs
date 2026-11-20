#![cfg(test)]

use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use super::test_support::{
    CONTAINER_ID, Fixture, Form, StubDecoder, StubHandler, StubLookup, decoding_to, deleted_result,
    provider_error,
};
use crate::api::response_id_codecs::container::build_container_id;

fn metadata_with_id(model: &str) -> IndexMap<String, Json> {
    IndexMap::from([(
        "model_info".to_owned(),
        Json::Object(IndexMap::from([(
            "id".to_owned(),
            Json::Text(model.to_owned()),
        )])),
    )])
}

#[tokio::test]
async fn async_retrieve_returns_the_raw_provider_id_without_re_encoding() {
    let mut fixture = Fixture::new();
    fixture.container_id = build_container_id(Some("openai"), Some("old-model"), CONTAINER_ID);
    fixture.decoder = StubDecoder::answering(decoding_to(CONTAINER_ID, "openai", Some("decoded")));
    fixture.litellm_metadata = Some(metadata_with_id("caller-model"));

    let object = fixture.aretrieve().await.expect("the call runs");

    assert_eq!(object.id, CONTAINER_ID);
    let calls = fixture.calls();
    let call = calls.first().expect("one call");
    assert_eq!(call.form, Form::Async);
    assert_eq!(call.container_id, CONTAINER_ID);
    assert_eq!(call.call_type, "retrieve_container");
}

#[tokio::test]
async fn async_delete_returns_the_raw_provider_id_without_re_encoding() {
    let mut fixture = Fixture::new();
    fixture.handler = StubHandler::deleting(deleted_result());
    fixture.container_id = build_container_id(Some("openai"), Some("old-model"), CONTAINER_ID);
    fixture.decoder = StubDecoder::answering(decoding_to(CONTAINER_ID, "openai", Some("decoded")));

    let deleted = fixture.adelete().await.expect("the call runs");

    assert_eq!(deleted.id, CONTAINER_ID);
    let calls = fixture.calls();
    let call = calls.first().expect("one call");
    assert_eq!(call.form, Form::Async);
    assert_eq!(call.call_type, "delete_container");
}

#[tokio::test]
async fn async_mock_returns_the_record_without_the_handler() {
    let mut fixture = Fixture::new();
    fixture.mock_response = Some(Json::Object(IndexMap::from([
        ("id".to_owned(), Json::Text("cntr_mock".to_owned())),
        ("object".to_owned(), Json::Text("container".to_owned())),
        ("created_at".to_owned(), Json::Number(Number::from(1_i64))),
        ("status".to_owned(), Json::Text("running".to_owned())),
    ])));

    let object = fixture.aretrieve().await.expect("the mock answers");

    assert_eq!(object.id, "cntr_mock");
    assert!(fixture.calls().is_empty());
    assert!(fixture.decoder.seen().is_empty());
}

#[tokio::test]
async fn async_errors_dispatch_with_the_callers_provider_name() {
    let mut fixture = Fixture::new();
    fixture.container_id = build_container_id(Some("azure"), Some("deployment"), CONTAINER_ID);
    fixture.decoder =
        StubDecoder::answering(decoding_to(CONTAINER_ID, "azure", Some("deployment")));
    fixture.lookup = StubLookup::carrying("azure-config");
    fixture.handler = StubHandler::failing(provider_error(401, "nope", false));

    let failure = fixture.aretrieve().await.expect_err("the handler fails");

    let ErrorClass::Authentication(details) = &failure.class else {
        panic!("expected an authentication error, got {:?}", failure.class);
    };
    assert_eq!(details.provider.as_deref(), Some("openai"));
    assert_eq!(details.model.as_deref(), Some(""));
    assert_eq!(details.status.0, 401);
}

#[tokio::test]
async fn async_delete_errors_dispatch_with_the_callers_provider_name() {
    let mut fixture = Fixture::new();
    fixture.container_id = build_container_id(Some("azure"), Some("deployment"), CONTAINER_ID);
    fixture.decoder =
        StubDecoder::answering(decoding_to(CONTAINER_ID, "azure", Some("deployment")));
    fixture.lookup = StubLookup::carrying("azure-config");
    fixture.handler = StubHandler::failing(provider_error(401, "nope", false));

    let failure = fixture.adelete().await.expect_err("the handler fails");

    let ErrorClass::Authentication(details) = &failure.class else {
        panic!("expected an authentication error, got {:?}", failure.class);
    };
    assert_eq!(details.provider.as_deref(), Some("openai"));
    assert_eq!(details.model.as_deref(), Some(""));
    assert_eq!(details.status.0, 401);
}

#[tokio::test]
async fn async_prepare_errors_keep_the_decoded_provider() {
    let mut fixture = Fixture::new();
    fixture.decoder = StubDecoder::answering(decoding_to(CONTAINER_ID, "azure", None));
    fixture.lookup = StubLookup::without_config();

    let failure = fixture
        .adelete()
        .await
        .expect_err("the lookup answers nothing");

    let ErrorClass::ApiConnection(context) = &failure.class else {
        panic!("expected a value error, got {:?}", failure.class);
    };
    assert_eq!(context.provider.as_deref(), Some("azure"));
}
