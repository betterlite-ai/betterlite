#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_error::error::ErrorClass;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_openai::api::container_contracts::error::{BaseLlmException, ContainerError};
use betterlite_openai::containers::container_file_http::ContainerFileHttpError;

use super::error::{ContainerOpError, ContainerOpFailure, failure};
use super::test_support::{
    CONTAINER_ID, Fixture, StubDecode, StubDecoder, StubHandler, StubLookup, container_refusal,
    decoding_to, provider_error, redactor,
};
use crate::api::container_resource_http::ContainerResourceHttpError;
use crate::api::response_id_codecs::container::build_container_id;

fn handler_failure(error: ContainerResourceHttpError) -> ContainerOpFailure {
    failure(ContainerOpError::Handler(error), "openai", &redactor())
}

fn upstream(status_code: u16, message: &str) -> BaseLlmException {
    BaseLlmException {
        status_code,
        message: message.to_owned(),
        headers: BTreeMap::new(),
    }
}

#[test]
fn error_a_provider_rate_limit_keeps_its_status_and_text() {
    let failure = handler_failure(provider_error(429, "slow down", false));

    let ErrorClass::RateLimit(details) = &failure.class else {
        panic!("expected a rate limit, got {:?}", failure.class);
    };
    assert_eq!(details.context.provider.as_deref(), Some("openai"));
    assert_eq!(details.context.model.as_deref(), Some(""));
    assert!(details.context.message.contains("slow down"));
}

#[test]
fn error_a_synthesized_status_lets_the_message_choose_the_family() {
    let failure = handler_failure(provider_error(
        500,
        "invalid_request_error model_not_found",
        true,
    ));

    let ErrorClass::NotFound(details) = &failure.class else {
        panic!("expected a not found, got {:?}", failure.class);
    };
    assert_eq!(details.status.0, 404);
    assert!(details.message.contains("model_not_found"));
}

#[test]
fn error_a_wrong_param_type_from_the_handler_names_the_field() {
    let failure = handler_failure(ContainerResourceHttpError::Call(
        ContainerFileHttpError::ParamType {
            field: "ssl_verify",
            expected: "a boolean or a CA bundle path",
        },
    ));

    let ErrorClass::BadRequest(details) = &failure.class else {
        panic!("expected a bad request, got {:?}", failure.class);
    };
    assert_eq!(details.status.0, 400);
    assert_eq!(details.body_param.as_deref(), Some("ssl_verify"));
}

#[test]
fn error_a_suspended_handler_call_is_a_value_error() {
    let failure = handler_failure(container_refusal());

    let ErrorClass::ApiConnection(context) = &failure.class else {
        panic!("expected a value error, got {:?}", failure.class);
    };
    assert_eq!(context.model.as_deref(), Some(""));
}

#[test]
fn error_an_upstream_config_error_keeps_its_status() {
    let failure = handler_failure(ContainerResourceHttpError::Call(
        ContainerFileHttpError::Config(ContainerError::Upstream(upstream(401, "nope"))),
    ));

    let ErrorClass::Authentication(details) = &failure.class else {
        panic!("expected an authentication error, got {:?}", failure.class);
    };
    assert_eq!(details.status.0, 401);
    assert_eq!(details.provider.as_deref(), Some("openai"));
}

#[test]
fn error_a_decode_failure_is_a_bad_request_naming_the_field() {
    let mut fixture = Fixture::new();
    fixture.decoder = StubDecoder::answering(StubDecode::Fails);

    let failure = fixture.retrieve().expect_err("the decoder refuses");

    assert!(matches!(&failure.error, ContainerOpError::Decode(_)));
    let ErrorClass::BadRequest(details) = &failure.class else {
        panic!("expected a bad request, got {:?}", failure.class);
    };
    assert_eq!(details.body_param.as_deref(), Some("model_id"));
    assert_eq!(details.model.as_deref(), Some(""));
    assert!(fixture.calls().is_empty());
}

#[test]
fn error_a_logging_failure_is_a_value_error() {
    let failure = failure(
        ContainerOpError::Logging(LoggingStateError::FieldType { field: "user" }),
        "openai",
        &redactor(),
    );

    let ErrorClass::ApiConnection(context) = &failure.class else {
        panic!("expected a value error, got {:?}", failure.class);
    };
    assert_eq!(context.provider.as_deref(), Some("openai"));
    assert_eq!(context.model.as_deref(), Some(""));
}

#[test]
fn error_the_sync_delete_failure_uses_the_decoded_provider() {
    let mut fixture = Fixture::new();
    fixture.container_id = build_container_id(Some("azure"), Some("deployment"), CONTAINER_ID);
    fixture.decoder =
        StubDecoder::answering(decoding_to(CONTAINER_ID, "azure", Some("deployment")));
    fixture.lookup = StubLookup::carrying("azure-config");
    fixture.handler = StubHandler::failing(provider_error(401, "nope", false));

    let failure = fixture.delete().expect_err("the handler fails");

    let ErrorClass::Authentication(details) = &failure.class else {
        panic!("expected an authentication error, got {:?}", failure.class);
    };
    assert_eq!(details.provider.as_deref(), Some("azure"));
    assert_eq!(details.status.0, 401);
}

#[test]
fn error_the_sync_handler_failure_uses_the_decoded_provider() {
    let mut fixture = Fixture::new();
    fixture.container_id = build_container_id(Some("azure"), Some("deployment"), CONTAINER_ID);
    fixture.decoder =
        StubDecoder::answering(decoding_to(CONTAINER_ID, "azure", Some("deployment")));
    fixture.lookup = StubLookup::carrying("azure-config");
    fixture.handler = StubHandler::failing(provider_error(401, "nope", false));

    let failure = fixture.retrieve().expect_err("the handler fails");

    let ErrorClass::Authentication(details) = &failure.class else {
        panic!("expected an authentication error, got {:?}", failure.class);
    };
    assert_eq!(details.provider.as_deref(), Some("azure"));
    assert_eq!(details.status.0, 401);
}
