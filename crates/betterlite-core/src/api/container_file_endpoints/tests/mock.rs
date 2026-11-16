#![cfg(test)]

use std::sync::PoisonError;

use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json;
use betterlite_openai::containers::container_file_http::ContainerFileReply;
use serde_json::json;

use super::support::{
    DELETED_JSON, FILE_JSON, LIST_JSON, TIMEOUT_SECONDS, deleted_file, file_list, file_object,
    fixture, list_call, logging, retrieve_call, upload_call,
};

#[test]
fn mock_list_decodes_the_string_into_the_list_model() {
    let fixture = fixture();
    let endpoints = fixture.endpoints();
    let mut call = list_call();
    call.mock_response = Some(Json::Text(LIST_JSON.to_owned()));
    let reply = endpoints
        .container_file_call("list_container_files", &call, &logging(), TIMEOUT_SECONDS)
        .expect("the mock answers");
    assert_eq!(reply, ContainerFileReply::List(file_list()));
}

#[test]
fn mock_upload_decodes_an_object_into_the_file_model() {
    let fixture = fixture();
    let endpoints = fixture.endpoints();
    let mut call = upload_call();
    call.mock_response =
        Some(serde_json::from_str::<Json>(FILE_JSON).expect("the document parses"));
    let reply = endpoints
        .container_file_call("upload_container_file", &call, &logging(), TIMEOUT_SECONDS)
        .expect("the mock answers");
    assert_eq!(reply, ContainerFileReply::File(file_object()));
}

#[test]
fn mock_retrieve_decodes_the_string_into_the_file_model() {
    let fixture = fixture();
    let endpoints = fixture.endpoints();
    let mut call = retrieve_call();
    call.mock_response = Some(Json::Text(FILE_JSON.to_owned()));
    let reply = endpoints
        .container_file_call(
            "retrieve_container_file",
            &call,
            &logging(),
            TIMEOUT_SECONDS,
        )
        .expect("the mock answers");
    assert_eq!(reply, ContainerFileReply::File(file_object()));
}

#[test]
fn mock_delete_decodes_the_string_into_the_delete_model() {
    let fixture = fixture();
    let endpoints = fixture.endpoints();
    let mut call = retrieve_call();
    call.mock_response = Some(Json::Text(DELETED_JSON.to_owned()));
    let reply = endpoints
        .container_file_call("delete_container_file", &call, &logging(), TIMEOUT_SECONDS)
        .expect("the mock answers");
    assert_eq!(reply, ContainerFileReply::Deleted(deleted_file()));
}

#[test]
fn mock_content_returns_the_parsed_json_not_bytes() {
    let fixture = fixture();
    let endpoints = fixture.endpoints();
    let mut call = retrieve_call();
    call.mock_response = Some(Json::Text(
        r#"{"object":"file_content","text":"hello"}"#.to_owned(),
    ));
    let reply = endpoints
        .container_file_call(
            "retrieve_container_file_content",
            &call,
            &logging(),
            TIMEOUT_SECONDS,
        )
        .expect("the mock answers");
    assert_eq!(
        reply,
        ContainerFileReply::Raw(json!({"object": "file_content", "text": "hello"}))
    );
}

#[test]
fn mock_skips_the_provider_lookup_logging_and_the_handler() {
    let fixture = fixture();
    let endpoints = fixture.endpoints();
    let mut call = retrieve_call();
    call.custom_llm_provider = "bogus".to_owned();
    call.mock_response = Some(Json::Text(FILE_JSON.to_owned()));
    let logging = logging();
    let reply = endpoints
        .container_file_call("retrieve_container_file", &call, &logging, TIMEOUT_SECONDS)
        .expect("the mock answers without a provider");
    assert_eq!(reply, ContainerFileReply::File(file_object()));
    assert!(fixture.decoder.seen().is_empty());
    assert!(fixture.lookup.asked().is_empty());
    assert!(fixture.handler.seen().is_empty());
    let state = logging.lock().unwrap_or_else(PoisonError::into_inner);
    assert!(state.optional_params.is_empty());
    assert!(state.litellm_params.is_empty());
    assert_eq!(state.model, "container");
}

#[test]
fn mock_null_is_not_a_mock() {
    let fixture = fixture();
    let endpoints = fixture.endpoints();
    let mut call = retrieve_call();
    call.mock_response = Some(Json::Null);
    let reply = endpoints
        .container_file_call(
            "retrieve_container_file",
            &call,
            &logging(),
            TIMEOUT_SECONDS,
        )
        .expect("the handler answers");
    assert_eq!(reply, ContainerFileReply::File(file_object()));
    assert_eq!(fixture.handler.seen().len(), 1);
}

#[test]
fn mock_body_that_is_not_json_fails_before_the_handler() {
    let fixture = fixture();
    let endpoints = fixture.endpoints();
    let mut call = list_call();
    call.mock_response = Some(Json::Text("{not json".to_owned()));
    let failure = endpoints
        .container_file_call("list_container_files", &call, &logging(), TIMEOUT_SECONDS)
        .expect_err("the body is not JSON");
    assert_eq!(
        failure.error.to_string(),
        "the mock response is not valid JSON"
    );
    assert!(matches!(failure.class, ErrorClass::ApiConnection(_)));
    assert!(fixture.handler.seen().is_empty());
}

#[test]
fn mock_body_that_does_not_decode_names_the_response_model() {
    let fixture = fixture();
    let endpoints = fixture.endpoints();
    let mut call = list_call();
    call.mock_response = Some(Json::Text(r#"{"object":"list","data":[]}"#.to_owned()));
    let failure = endpoints
        .container_file_call("list_container_files", &call, &logging(), TIMEOUT_SECONDS)
        .expect_err("has_more is missing");
    assert_eq!(
        failure.error.to_string(),
        "the mock response does not match ContainerFileListResponse"
    );
    assert!(matches!(failure.class, ErrorClass::ApiConnection(_)));
    assert!(fixture.handler.seen().is_empty());
}
