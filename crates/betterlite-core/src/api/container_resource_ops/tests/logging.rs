#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::test_support::{
    CALL_ID, Fixture, StubDecoder, StubHandler, decoding_to, deleted_result,
};

#[test]
fn logging_the_retrieve_update_writes_the_call_type_and_empty_optional_params() {
    let fixture = Fixture::new();

    fixture.retrieve().expect("the call runs");

    let state = fixture.logging.lock().unwrap();
    assert_eq!(state.call_type, "retrieve_container");
    assert_eq!(state.model, "");
    assert!(state.optional_params.is_empty());
    assert_eq!(state.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(
        state.litellm_params,
        IndexMap::from([("litellm_call_id".to_owned(), Json::Text(CALL_ID.to_owned()))])
    );
}

#[test]
fn logging_the_delete_call_type_is_set_before_the_handler_runs() {
    let mut fixture = Fixture::new();
    fixture.handler = StubHandler::deleting(deleted_result());

    fixture.delete().expect("the call runs");

    let calls = fixture.calls();
    let call = calls.first().expect("one call");
    assert_eq!(call.call_type, "delete_container");
    assert_eq!(call.model, "");
    assert!(call.optional_params.is_empty());
    assert_eq!(call.provider.as_deref(), Some("openai"));
    assert_eq!(
        call.litellm_params.get("litellm_call_id"),
        Some(&Json::Text(CALL_ID.to_owned()))
    );
    assert_eq!(
        fixture.logging.lock().unwrap().call_type,
        "delete_container"
    );
}

#[test]
fn logging_the_resolved_provider_is_recorded_not_the_callers_default() {
    let mut fixture = Fixture::new();
    fixture.decoder = StubDecoder::answering(decoding_to("cntr_1", "azure", None));

    fixture.retrieve().expect("the call runs");

    assert_eq!(
        fixture
            .calls()
            .first()
            .expect("one call")
            .provider
            .as_deref(),
        Some("azure")
    );
    assert_eq!(
        fixture
            .logging
            .lock()
            .unwrap()
            .custom_llm_provider
            .as_deref(),
        Some("azure")
    );
}

// LiteLLM hands update_from_kwargs the same kwargs dict, so an extra metadata entry reaches litellm_params
#[test]
fn logging_an_extra_metadata_reaches_the_state() {
    let mut fixture = Fixture::new();
    fixture.extras = Some(IndexMap::from([(
        "metadata".to_owned(),
        Json::Object(IndexMap::from([(
            "team".to_owned(),
            Json::Text("core".to_owned()),
        )])),
    )]));

    fixture.retrieve().expect("the call runs");

    let state = fixture.logging.lock().unwrap();
    let metadata = state.litellm_params.get("metadata").cloned();
    drop(state);
    assert!(matches!(metadata, Some(Json::Object(_))));
}

#[test]
fn logging_an_absent_call_id_is_recorded_as_null() {
    let mut fixture = Fixture::new();
    fixture.litellm_call_id = None;

    fixture.retrieve().expect("the call runs");

    assert_eq!(
        fixture
            .logging
            .lock()
            .unwrap()
            .litellm_params
            .get("litellm_call_id"),
        Some(&Json::Null)
    );
}

#[test]
fn logging_the_callers_litellm_metadata_reaches_the_state() {
    let mut fixture = Fixture::new();
    fixture.litellm_metadata = Some(IndexMap::from([(
        "model_info".to_owned(),
        Json::Object(IndexMap::from([(
            "id".to_owned(),
            Json::Text("routed".to_owned()),
        )])),
    )]));

    fixture.retrieve().expect("the call runs");

    let state = fixture.logging.lock().unwrap();
    let metadata = state.litellm_params.get("litellm_metadata").cloned();
    drop(state);
    assert!(matches!(metadata, Some(Json::Object(_))));
}
