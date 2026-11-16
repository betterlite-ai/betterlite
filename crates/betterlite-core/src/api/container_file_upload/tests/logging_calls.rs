#![cfg(test)]

use std::sync::PoisonError;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;

use super::test_support::{
    API_BASE, API_KEY, API_VERSION, CALL_ID, CONTAINER_ID, Fixture, INITIAL_CALL_TYPE,
    StubExecutor, object,
};
use super::upload_container_file;
use crate::api::response_id_codecs::container::build_container_id;

#[test]
fn upload_logs_the_input_container_id_and_the_call_id() {
    let mut fixture = Fixture::new();
    let managed = build_container_id(Some("openai"), None, CONTAINER_ID);
    fixture.container_id = managed.clone();
    let request = fixture.request();
    let executor = StubExecutor::replying(object("file_1"));

    upload_container_file(&executor, fixture.deps(), &request).expect("the upload succeeds");

    let state = fixture
        .logging
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    assert_eq!(
        state.optional_params.get("container_id"),
        Some(&Json::Text(managed.clone()))
    );
    assert_eq!(
        state.litellm_params.get("litellm_call_id"),
        Some(&Json::Text(CALL_ID.to_owned()))
    );
    assert_eq!(state.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(state.model, "");
    assert_eq!(state.call_type, "upload_container_file");
    assert_eq!(
        state.model_call_details.get("container_id"),
        Some(&Json::Text(managed))
    );
    // LiteLLM sets the call type after the logging update, so the details keep the one @client derived
    assert_eq!(
        state.model_call_details.get("call_type"),
        Some(&Json::Text(INITIAL_CALL_TYPE.to_owned()))
    );
}

#[test]
fn upload_logs_before_the_executor_runs() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let executor = StubExecutor::replying(object("file_1"));

    upload_container_file(&executor, fixture.deps(), &request).expect("the upload succeeds");

    let seen = executor.seen().expect("the executor ran");
    assert_eq!(seen.call_type, "upload_container_file");
    assert_eq!(seen.provider.as_deref(), Some("openai"));
    assert_eq!(seen.model, "");
    assert_eq!(
        seen.optional_params.get("container_id"),
        Some(&Json::Text(CONTAINER_ID.to_owned()))
    );
    assert_eq!(
        seen.logging_params.get("litellm_call_id"),
        Some(&Json::Text(CALL_ID.to_owned()))
    );
    assert_eq!(
        seen.details.get("call_type"),
        Some(&Json::Text(INITIAL_CALL_TYPE.to_owned()))
    );
}

#[test]
fn upload_logs_the_provider_the_managed_id_resolves_to() {
    let mut fixture = Fixture::new();
    fixture.container_id = build_container_id(Some("azure"), None, CONTAINER_ID);
    let request = fixture.request();
    let executor = StubExecutor::replying(object("file_1"));

    upload_container_file(&executor, fixture.deps(), &request).expect("the upload succeeds");

    let seen = executor.seen().expect("the executor ran");
    assert_eq!(seen.provider.as_deref(), Some("azure"));
    assert_eq!(
        seen.details.get("custom_llm_provider"),
        Some(&Json::Text("azure".to_owned()))
    );
}

#[test]
fn upload_does_not_log_the_named_credentials() {
    let mut fixture = Fixture::new();
    fixture.api_key = Some(SecretString::from(API_KEY));
    fixture.api_base = Some(API_BASE.to_owned());
    fixture.api_version = Some(API_VERSION.to_owned());
    let request = fixture.request();
    let executor = StubExecutor::replying(object("file_1"));

    upload_container_file(&executor, fixture.deps(), &request).expect("the upload succeeds");

    let state = fixture
        .logging
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    assert!(!state.litellm_params.contains_key("api_key"));
    assert!(!state.litellm_params.contains_key("api_base"));
    let Some(Json::Object(details)) = state.model_call_details.get("litellm_params") else {
        panic!("the details hold the litellm params");
    };
    assert!(!details.contains_key("api_key"));
    assert!(!details.contains_key("api_base"));
}

#[test]
fn upload_merges_kwargs_metadata_into_the_logging_state() {
    let mut fixture = Fixture::new();
    fixture.kwargs.insert(
        "metadata".to_owned(),
        Json::Object(IndexMap::from([(
            "team".to_owned(),
            Json::Text("core".to_owned()),
        )])),
    );
    let request = fixture.request();
    let executor = StubExecutor::replying(object("file_1"));

    upload_container_file(&executor, fixture.deps(), &request).expect("the upload succeeds");

    let state = fixture
        .logging
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let Some(Json::Object(metadata)) = state.litellm_params.get("metadata") else {
        panic!("the metadata is merged into the logging params");
    };
    assert_eq!(metadata.get("team"), Some(&Json::Text("core".to_owned())));
}
