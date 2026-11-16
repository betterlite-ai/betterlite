#![cfg(test)]

use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json;

use super::support::{
    StubDecode, StubReply, TIMEOUT_SECONDS, decoding_to, fixture, list_call, logging, retrieve_call,
};

#[test]
fn error_before_decoding_uses_the_callers_provider() {
    let fixture = fixture();
    let endpoints = fixture.endpoints();
    let mut call = list_call();
    call.custom_llm_provider = "bogus".to_owned();
    call.mock_response = Some(Json::Text("{not json".to_owned()));
    let failure = endpoints
        .container_file_call("list_container_files", &call, &logging(), TIMEOUT_SECONDS)
        .expect_err("the mock body is not JSON");
    let ErrorClass::ApiConnection(context) = &failure.class else {
        panic!("a ValueError maps to a connection error");
    };
    assert_eq!(context.provider.as_deref(), Some("bogus"));
    assert_eq!(context.model.as_deref(), Some(""));
    assert!(
        context
            .message
            .contains("the mock response is not valid JSON")
    );
    assert!(fixture.decoder.seen().is_empty());
}

#[test]
fn error_after_decoding_uses_the_resolved_provider() {
    let mut fixture = fixture();
    let answer = StubDecode::Decoded {
        container_id: "cntr_original".to_owned(),
        provider: "bogus".to_owned(),
        model_id: None,
    };
    fixture.decoder.answer = answer;
    let endpoints = fixture.endpoints();
    let failure = endpoints
        .container_file_call(
            "list_container_files",
            &list_call(),
            &logging(),
            TIMEOUT_SECONDS,
        )
        .expect_err("the decoded provider name is unknown");
    assert_eq!(
        failure.error.to_string(),
        "\"bogus\" is not a provider betterlite knows"
    );
    let ErrorClass::ApiConnection(context) = &failure.class else {
        panic!("a ValueError maps to a connection error");
    };
    assert_eq!(context.provider.as_deref(), Some("bogus"));
    assert_eq!(context.model.as_deref(), Some(""));
    assert!(
        context
            .message
            .contains("is not a provider betterlite knows")
    );
}

#[test]
fn error_handler_status_maps_to_the_status_class() {
    let mut fixture = fixture();
    fixture.handler.reply = StubReply::Status {
        status_code: 404,
        message: "no such file".to_owned(),
    };
    let endpoints = fixture.endpoints();
    let failure = endpoints
        .container_file_call(
            "retrieve_container_file",
            &retrieve_call(),
            &logging(),
            TIMEOUT_SECONDS,
        )
        .expect_err("the provider fails");
    assert_eq!(failure.error.to_string(), "no such file");
    let ErrorClass::NotFound(details) = &failure.class else {
        panic!("a 404 maps to not found");
    };
    assert_eq!(details.provider.as_deref(), Some("openai"));
    assert_eq!(details.model.as_deref(), Some(""));
    assert!(details.message.contains("no such file"));
}

#[test]
fn error_handler_status_uses_the_decoded_provider() {
    let mut fixture = fixture();
    fixture.decoder.answer = decoding_to("azure", None);
    fixture.handler.reply = StubReply::Status {
        status_code: 404,
        message: "no such file".to_owned(),
    };
    let endpoints = fixture.endpoints();
    let failure = endpoints
        .container_file_call(
            "retrieve_container_file",
            &retrieve_call(),
            &logging(),
            TIMEOUT_SECONDS,
        )
        .expect_err("the provider fails");
    let ErrorClass::NotFound(details) = &failure.class else {
        panic!("a 404 maps to not found");
    };
    assert_eq!(details.provider.as_deref(), Some("azure"));
    assert_eq!(details.model.as_deref(), Some(""));
    assert!(details.message.contains("no such file"));
}

#[test]
fn error_handler_authentication_failure_maps_to_authentication() {
    let mut fixture = fixture();
    fixture.handler.reply = StubReply::Status {
        status_code: 401,
        message: "invalid api key".to_owned(),
    };
    let endpoints = fixture.endpoints();
    let failure = endpoints
        .container_file_call(
            "retrieve_container_file",
            &retrieve_call(),
            &logging(),
            TIMEOUT_SECONDS,
        )
        .expect_err("the provider fails");
    assert!(matches!(failure.class, ErrorClass::Authentication(_)));
    assert_eq!(failure.error.to_string(), "invalid api key");
}

#[test]
fn error_handler_config_upstream_failure_maps_to_internal_server() {
    let mut fixture = fixture();
    fixture.handler.reply = StubReply::ConfigStatus {
        status_code: 500,
        message: "upstream exploded".to_owned(),
    };
    let endpoints = fixture.endpoints();
    let failure = endpoints
        .container_file_call(
            "retrieve_container_file",
            &retrieve_call(),
            &logging(),
            TIMEOUT_SECONDS,
        )
        .expect_err("the config fails the request");
    assert!(matches!(failure.class, ErrorClass::InternalServer(_)));
    assert_eq!(failure.error.to_string(), "upstream exploded");
}

#[test]
fn error_decode_failure_uses_the_callers_provider() {
    let mut fixture = fixture();
    fixture.decoder.answer = StubDecode::Fails;
    let endpoints = fixture.endpoints();
    let failure = endpoints
        .container_file_call(
            "retrieve_container_file",
            &retrieve_call(),
            &logging(),
            TIMEOUT_SECONDS,
        )
        .expect_err("the managed id does not decode");
    assert_eq!(
        failure.error.to_string(),
        "the model_id field has the wrong type"
    );
    let ErrorClass::ApiConnection(context) = &failure.class else {
        panic!("a ValueError maps to a connection error");
    };
    assert_eq!(context.provider.as_deref(), Some("openai"));
    assert_eq!(context.model.as_deref(), Some(""));
    assert!(fixture.handler.seen().is_empty());
}

#[test]
fn error_default_timeout_that_is_not_a_duration_fails() {
    let fixture = fixture();
    let endpoints = fixture.endpoints();
    let mut call = retrieve_call();
    call.timeout = None;
    let failure = endpoints
        .container_file_call("retrieve_container_file", &call, &logging(), -1.0)
        .expect_err("the default is not a usable duration");
    assert_eq!(
        failure.error.to_string(),
        "the default request timeout of -1 seconds is not a usable duration"
    );
    assert!(matches!(failure.class, ErrorClass::ApiConnection(_)));
    assert!(fixture.handler.seen().is_empty());
}
