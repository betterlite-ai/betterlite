#![cfg(test)]

use betterlite_error::error::ErrorClass;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::error::ContainerOpError;
use super::test_support::{
    CONTAINER_ID, Fixture, StubDecoder, StubHandler, StubLookup, decoding_to, deleted_result,
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

#[test]
fn encode_a_plain_id_comes_back_unchanged() {
    let fixture = Fixture::new();

    let object = fixture.retrieve().expect("the call runs");

    assert_eq!(object.id, CONTAINER_ID);
}

#[test]
fn encode_a_managed_id_is_re_encoded_with_the_decoded_provider_and_model() {
    let mut fixture = Fixture::new();
    fixture.container_id = build_container_id(Some("openai"), Some("old-model"), CONTAINER_ID);
    fixture.decoder =
        StubDecoder::answering(decoding_to(CONTAINER_ID, "azure", Some("deployment")));
    fixture.lookup = StubLookup::carrying("azure-config");

    let object = fixture.retrieve().expect("the call runs");

    assert_eq!(
        object.id,
        build_container_id(Some("azure"), Some("deployment"), CONTAINER_ID)
    );
}

#[test]
fn encode_a_metadata_model_info_id_encodes_even_a_plain_id() {
    let mut fixture = Fixture::new();
    fixture.litellm_metadata = Some(metadata_with_id("router-model"));

    let object = fixture.retrieve().expect("the call runs");

    assert_eq!(
        object.id,
        build_container_id(Some("openai"), Some("router-model"), CONTAINER_ID)
    );
}

#[test]
fn encode_the_decoded_model_id_replaces_a_metadata_id_for_a_managed_input() {
    let mut fixture = Fixture::new();
    fixture.container_id = build_container_id(Some("openai"), Some("old-model"), CONTAINER_ID);
    fixture.decoder = StubDecoder::answering(decoding_to(CONTAINER_ID, "openai", Some("decoded")));
    fixture.litellm_metadata = Some(metadata_with_id("caller-model"));

    let object = fixture.retrieve().expect("the call runs");

    assert_eq!(
        object.id,
        build_container_id(Some("openai"), Some("decoded"), CONTAINER_ID)
    );
}

// LiteLLM mutates the caller's kwargs dict; betterlite holds a shared reference, so the map cannot change
#[test]
fn encode_the_callers_metadata_is_left_untouched() {
    let mut fixture = Fixture::new();
    fixture.container_id = build_container_id(Some("openai"), Some("old-model"), CONTAINER_ID);
    fixture.decoder = StubDecoder::answering(decoding_to(CONTAINER_ID, "openai", Some("decoded")));
    fixture.litellm_metadata = Some(metadata_with_id("caller-model"));

    fixture.retrieve().expect("the call runs");

    assert_eq!(
        fixture.litellm_metadata,
        Some(metadata_with_id("caller-model"))
    );
}

#[test]
fn encode_a_decoded_model_id_is_not_written_for_a_plain_input() {
    let mut fixture = Fixture::new();
    fixture.decoder = StubDecoder::answering(decoding_to(CONTAINER_ID, "openai", Some("decoded")));

    let object = fixture.retrieve().expect("the call runs");

    assert_eq!(object.id, CONTAINER_ID);
}

#[test]
fn encode_a_null_model_info_is_created_for_the_injection() {
    let mut fixture = Fixture::new();
    fixture.container_id = build_container_id(Some("openai"), Some("old-model"), CONTAINER_ID);
    fixture.decoder = StubDecoder::answering(decoding_to(CONTAINER_ID, "openai", Some("decoded")));
    fixture.litellm_metadata = Some(IndexMap::from([("model_info".to_owned(), Json::Null)]));

    let object = fixture.retrieve().expect("the call runs");

    assert_eq!(
        object.id,
        build_container_id(Some("openai"), Some("decoded"), CONTAINER_ID)
    );
}

#[test]
fn encode_a_wrongly_typed_model_info_is_a_bad_request_naming_it() {
    let mut fixture = Fixture::new();
    fixture.container_id = build_container_id(Some("openai"), Some("old-model"), CONTAINER_ID);
    fixture.decoder = StubDecoder::answering(decoding_to(CONTAINER_ID, "openai", Some("decoded")));
    fixture.litellm_metadata = Some(IndexMap::from([(
        "model_info".to_owned(),
        Json::Text("text".to_owned()),
    )]));

    let failure = fixture
        .retrieve()
        .expect_err("the metadata cannot take the id");

    let ContainerOpError::Encode(field_error) = &failure.error else {
        panic!("expected an encode error, got {:?}", failure.error);
    };
    assert_eq!(field_error.field, "litellm_metadata.model_info");
    let ErrorClass::BadRequest(details) = &failure.class else {
        panic!("expected a bad request, got {:?}", failure.class);
    };
    assert_eq!(details.status.0, 400);
    assert_eq!(
        details.body_param.as_deref(),
        Some("litellm_metadata.model_info")
    );
}

#[test]
fn encode_extra_body_target_model_names_never_encode_here() {
    let mut fixture = Fixture::new();
    fixture.extra_body = Some(IndexMap::from([(
        "target_model_names".to_owned(),
        JsonValue::Text("gpt-4o".to_owned()),
    )]));

    let object = fixture.retrieve().expect("the call runs");

    assert_eq!(object.id, CONTAINER_ID);
}

#[test]
fn encode_the_delete_result_id_is_re_encoded_too() {
    let mut fixture = Fixture::new();
    fixture.handler = StubHandler::deleting(deleted_result());
    fixture.container_id = build_container_id(Some("openai"), Some("old-model"), CONTAINER_ID);
    fixture.decoder = StubDecoder::answering(decoding_to(CONTAINER_ID, "openai", Some("decoded")));

    let deleted = fixture.delete().expect("the call runs");

    assert_eq!(
        deleted.id,
        build_container_id(Some("openai"), Some("decoded"), CONTAINER_ID)
    );
}
