#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_error::error::ErrorClass;
use betterlite_http::proxy::pool::PoolError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_models::decisions::Json;
use betterlite_openai::api::container_contracts::error::{BaseLlmException, ContainerError};
use betterlite_openai::containers::container_file_http::ContainerFileHttpError;

use super::error::{
    ContainerFileListError, ContainerFileListFailure, FileListHandlerError, MockDecodeError,
    failure,
};
use super::test_support::redactor;
use crate::containers::container_request_parameters::ContainerFieldTypeError;

fn provider_error(status_code: u16, message: &str) -> BaseLlmException {
    BaseLlmException {
        status_code,
        message: message.to_owned(),
        headers: BTreeMap::new(),
    }
}

fn handler_failure(error: FileListHandlerError) -> ContainerFileListFailure {
    failure(
        ContainerFileListError::Handler(error),
        "openai",
        &redactor(),
    )
}

#[test]
fn a_transport_exception_is_a_synthesized_internal_server() {
    let transport = provider_error(500, "the connection was not made");

    let error = handler_failure(FileListHandlerError::Transport(transport));

    let ErrorClass::InternalServer(context) = &error.class else {
        panic!("expected an internal server, got {:?}", error.class);
    };
    assert_eq!(context.provider.as_deref(), Some("openai"));
    assert_eq!(context.model.as_deref(), Some(""));
    assert!(
        context
            .message
            .starts_with("InternalServerError: OpenAIException - ")
    );
}

#[test]
fn a_synthesized_status_does_not_choose_a_phrase_family_status() {
    let transport = provider_error(500, "invalid_request_error model_not_found");

    let error = handler_failure(FileListHandlerError::Transport(transport));

    let ErrorClass::NotFound(details) = &error.class else {
        panic!("expected a not found, got {:?}", error.class);
    };
    assert_eq!(details.status.0, 404);
    assert!(details.message.contains("model_not_found"));
}

#[test]
fn a_provider_status_chooses_the_phrase_family_status() {
    let provider = provider_error(500, "invalid_request_error model_not_found");

    let error = handler_failure(FileListHandlerError::Provider(provider));

    let ErrorClass::NotFound(details) = &error.class else {
        panic!("expected a not found, got {:?}", error.class);
    };
    assert_eq!(details.status.0, 500);
}

#[test]
fn a_provider_authentication_error_keeps_its_status() {
    let provider = provider_error(401, "nope");

    let error = handler_failure(FileListHandlerError::Provider(provider));

    let ErrorClass::Authentication(details) = &error.class else {
        panic!("expected an authentication error, got {:?}", error.class);
    };
    assert_eq!(details.status.0, 401);
    assert_eq!(details.provider.as_deref(), Some("openai"));
    assert_eq!(details.model.as_deref(), Some(""));
    assert!(
        details
            .message
            .contains("AuthenticationError: OpenAIException - nope")
    );
}

#[test]
fn a_configuration_upstream_error_keeps_its_status() {
    let upstream = provider_error(429, "slow down");

    let error = handler_failure(FileListHandlerError::Config(ContainerError::Upstream(
        upstream,
    )));

    let ErrorClass::RateLimit(details) = &error.class else {
        panic!("expected a rate limit, got {:?}", error.class);
    };
    assert_eq!(details.context.provider.as_deref(), Some("openai"));
    assert!(
        details
            .context
            .message
            .contains("RateLimitError: OpenAIException - slow down")
    );
}

#[test]
fn a_decode_failure_names_the_field() {
    let field_error = ContainerFieldTypeError {
        field: "litellm_params.model_id",
    };

    let error = failure(
        ContainerFileListError::Decode(field_error),
        "openai",
        &redactor(),
    );

    let ErrorClass::BadRequest(details) = &error.class else {
        panic!("expected a bad request, got {:?}", error.class);
    };
    assert_eq!(details.status.0, 400);
    assert_eq!(
        details.body_param.as_deref(),
        Some("litellm_params.model_id")
    );
    assert_eq!(details.model.as_deref(), Some(""));
}

#[test]
fn a_wrongly_typed_param_names_the_field() {
    let param_error = ContainerFileHttpError::ParamType {
        field: "ssl_verify",
        expected: "a boolean or a CA bundle path",
    };

    let error = handler_failure(FileListHandlerError::Params(param_error));

    let ErrorClass::BadRequest(details) = &error.class else {
        panic!("expected a bad request, got {:?}", error.class);
    };
    assert_eq!(details.body_param.as_deref(), Some("ssl_verify"));
    assert!(details.message.contains("ssl_verify"));
}

#[test]
fn a_refused_query_value_is_a_value_error() {
    let error = handler_failure(FileListHandlerError::QueryValue {
        field: "nested".to_owned(),
    });

    let ErrorClass::ApiConnection(context) = &error.class else {
        panic!("expected an api connection, got {:?}", error.class);
    };
    assert_eq!(context.provider.as_deref(), Some("openai"));
    assert!(error.error.to_string().contains("nested"));
}

#[test]
fn a_closed_client_pool_is_a_value_error() {
    let error = handler_failure(FileListHandlerError::Client(PoolError::Closed));

    let ErrorClass::ApiConnection(context) = &error.class else {
        panic!("expected an api connection, got {:?}", error.class);
    };
    assert!(context.message.contains("the pool is closed"));
}

#[test]
fn a_hook_failure_is_a_value_error() {
    let error = handler_failure(FileListHandlerError::Logging(
        PrePostError::RawRequestCaptured,
    ));

    assert!(matches!(&error.class, ErrorClass::ApiConnection(_)));
}

#[test]
fn an_encoding_failure_is_a_value_error() {
    let source = serde_json::from_str::<Json>("{ ").expect_err("the fixture is not JSON");

    let error = handler_failure(FileListHandlerError::Encoding(source));

    assert!(matches!(&error.class, ErrorClass::ApiConnection(_)));
}

#[test]
fn another_handler_failure_that_is_not_a_param_type_is_a_value_error() {
    let error = handler_failure(FileListHandlerError::Params(
        ContainerFileHttpError::Suspended,
    ));

    assert!(matches!(&error.class, ErrorClass::ApiConnection(_)));
}

#[test]
fn a_config_failure_without_an_upstream_answer_is_a_value_error() {
    let error = handler_failure(FileListHandlerError::Config(
        ContainerError::ApiBaseRequired,
    ));

    assert!(matches!(&error.class, ErrorClass::ApiConnection(_)));
}

#[test]
fn a_local_param_encoding_failure_is_a_value_error() {
    let source = serde_json::from_str::<Json>("{ ").expect_err("the fixture is not JSON");

    let error = failure(
        ContainerFileListError::Params(source),
        "openai",
        &redactor(),
    );

    assert!(matches!(&error.class, ErrorClass::ApiConnection(_)));
}

#[test]
fn a_logging_state_failure_is_a_value_error() {
    let error = failure(
        ContainerFileListError::Logging(LoggingStateError::FieldType { field: "user" }),
        "openai",
        &redactor(),
    );

    assert!(matches!(&error.class, ErrorClass::ApiConnection(_)));
}

#[test]
fn a_mock_parse_failure_is_a_value_error() {
    let source = serde_json::from_str::<Json>("{ ").expect_err("the fixture is not JSON");

    let error = failure(
        ContainerFileListError::Mock(MockDecodeError::Json { source }),
        "openai",
        &redactor(),
    );

    let ErrorClass::ApiConnection(context) = &error.class else {
        panic!("expected an api connection, got {:?}", error.class);
    };
    assert_eq!(context.model.as_deref(), Some(""));
    assert!(error.error.to_string().contains("not valid JSON"));
}
