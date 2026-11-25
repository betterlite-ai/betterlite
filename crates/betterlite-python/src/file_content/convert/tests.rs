#![cfg(test)]

use betterlite_azure::azure_files::error::AzureFilesError;
use betterlite_azure::endpoints::initialize::EndpointError;
use betterlite_core::api::file_content_stream::error::FileContentError;
use betterlite_core::api::file_stream_logging::source::StreamError;
use betterlite_core::api::fine_tuning_read::{ReadTimeout, ReadTimeoutValue};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::files::error::{FilesError, StatusError};
use rstest::rstest;

use super::super::dispatch::DispatchError;
use super::super::error::{Failure, azure_failure, failure_for, files_failure, stream_failure};
use super::{ConvertError, parse_call};

const FULL: &str = r#"{"file_id":"file-abc","model":"gpt-4o","custom_llm_provider":"openai","extra_headers":{"x-test":"1"},"extra_body":{"note":"n"},"chunk_size":2048,"stream":true,"api_key":"sk-test","api_base":"http://127.0.0.1:1/v1","api_version":"2024-01-01","organization":"org-1","timeout":2.5,"request_timeout":"3","max_retries":4,"litellm_call_id":"call-1","id":"fn-1"}"#;

#[test]
fn request_reads_every_field_the_call_takes() {
    let call = parse_call(FULL).unwrap();
    assert_eq!(call.file_id, "file-abc");
    assert_eq!(call.model.as_deref(), Some("gpt-4o"));
    assert_eq!(call.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(
        call.extra_headers
            .as_ref()
            .and_then(|headers| headers.get("x-test"))
            .map(String::as_str),
        Some("1")
    );
    assert_eq!(
        call.extra_body
            .as_ref()
            .and_then(|body| body.get("note"))
            .map(String::as_str),
        Some("n")
    );
    assert_eq!(call.chunk_size, 2_048_u64);
    assert!(call.stream);
    assert!(call.kwargs.api_key.is_some());
    assert_eq!(
        call.kwargs.api_base.as_deref(),
        Some("http://127.0.0.1:1/v1")
    );
    assert_eq!(call.kwargs.api_version.as_deref(), Some("2024-01-01"));
    assert_eq!(call.kwargs.organization.as_deref(), Some("org-1"));
    assert_eq!(call.kwargs.max_retries, Some(4));
    assert_eq!(call.kwargs.litellm_call_id.as_deref(), Some("call-1"));
    assert_eq!(call.kwargs.id.as_deref(), Some("fn-1"));
}

#[test]
fn request_chunk_size_defaults_to_one_mebibyte() {
    let call = parse_call(r#"{"file_id":"file-abc"}"#).unwrap();
    assert_eq!(call.chunk_size, 0x0010_0000_u64);
    assert!(!call.stream);
    assert_eq!(call.model, None);
    assert_eq!(call.custom_llm_provider, None);
    assert!(call.kwargs.timeout.is_none());
    assert!(call.kwargs.request_timeout.is_none());
}

#[test]
fn request_structured_timeout_becomes_the_phases() {
    let call =
        parse_call(r#"{"file_id":"f","timeout":{"connect":1.5,"read":2,"write":3,"pool":4}}"#)
            .unwrap();
    let Some(ReadTimeout::Structured(phases)) = call.kwargs.timeout else {
        panic!("the timeout is not the phases it was given");
    };
    assert_eq!(
        phases.connect.and_then(|number| number.as_f64()),
        Some(1.5_f64)
    );
    assert_eq!(
        phases.read.and_then(|number| number.as_f64()),
        Some(2.0_f64)
    );
    assert_eq!(
        phases.write.and_then(|number| number.as_f64()),
        Some(3.0_f64)
    );
    assert_eq!(
        phases.pool.and_then(|number| number.as_f64()),
        Some(4.0_f64)
    );
}

#[test]
fn request_a_number_timeout_stays_a_number() {
    let call = parse_call(r#"{"file_id":"f","timeout":2.5}"#).unwrap();
    let Some(ReadTimeout::Plain(ReadTimeoutValue::Number(number))) = call.kwargs.timeout else {
        panic!("the timeout is not the number it was given");
    };
    assert_eq!(number.as_f64(), Some(2.5_f64));
}

#[test]
fn request_a_text_timeout_stays_text() {
    let call = parse_call(r#"{"file_id":"f","timeout":"1.5"}"#).unwrap();
    let Some(ReadTimeout::Plain(ReadTimeoutValue::Text(text))) = call.kwargs.timeout else {
        panic!("the timeout is not the text it was given");
    };
    assert_eq!(text, "1.5");
}

#[test]
fn request_request_timeout_takes_a_plain_value() {
    let call = parse_call(r#"{"file_id":"f","request_timeout":7}"#).unwrap();
    match call.kwargs.request_timeout {
        Some(ReadTimeoutValue::Number(number)) => assert_eq!(number.as_f64(), Some(7.0_f64)),
        Some(ReadTimeoutValue::Text(text)) => panic!("the number became the text {text}"),
        None => panic!("the request timeout was dropped"),
    }
}

#[test]
fn request_a_timeout_shape_it_does_not_take_is_refused() {
    let error = parse_call(r#"{"file_id":"f","timeout":true}"#).unwrap_err();
    assert!(matches!(error, ConvertError::TimeoutShapes(_)));
}

#[test]
fn request_request_timeout_refuses_the_phases() {
    let error = parse_call(r#"{"file_id":"f","request_timeout":{"read":1}}"#).unwrap_err();
    assert!(matches!(error, ConvertError::TimeoutShapes(_)));
}

#[test]
fn request_an_unknown_field_is_refused() {
    let error = parse_call(r#"{"file_id":"f","nope":1}"#).unwrap_err();
    assert!(matches!(error, ConvertError::Json { .. }));
}

#[test]
fn error_a_status_is_the_class_its_code_names() {
    let status = StatusError {
        status: 404_u16,
        message: String::from("Error code: 404 - missing"),
        error_type: Some(String::from("invalid_request_error")),
        param: None,
        code: Some(String::from("file_not_found")),
    };
    let failure = files_failure(&FilesError::Status(Box::new(status)));
    let Failure::Payload(payload) = failure else {
        panic!("the status error did not become a payload");
    };
    assert_eq!(payload.class, "NotFoundError");
    assert_eq!(payload.status_code, Some(404_u16));
    assert_eq!(payload.message, "Error code: 404 - missing");
    let Some(JsonValue::Object(body)) = payload.body else {
        panic!("the status body is not an object");
    };
    assert_eq!(
        body.get("code"),
        Some(&JsonValue::Text(String::from("file_not_found")))
    );
    assert_eq!(body.get("param"), None);
}

#[test]
fn error_a_status_without_fields_carries_no_body() {
    let status = StatusError {
        status: 400_u16,
        message: String::from("Error code: 400"),
        error_type: None,
        param: None,
        code: None,
    };
    let failure = files_failure(&FilesError::Status(Box::new(status)));
    let Failure::Payload(payload) = failure else {
        panic!("the status error did not become a payload");
    };
    assert_eq!(payload.class, "BadRequestError");
    assert_eq!(payload.body, None);
}

#[rstest]
#[case::bad_request(400_u16, "BadRequestError")]
#[case::authentication(401_u16, "AuthenticationError")]
#[case::permission_denied(403_u16, "PermissionDeniedError")]
#[case::not_found(404_u16, "NotFoundError")]
#[case::unprocessable_entity(422_u16, "UnprocessableEntityError")]
#[case::rate_limit(429_u16, "RateLimitError")]
#[case::internal_server(500_u16, "InternalServerError")]
#[case::internal_server_at_the_top(599_u16, "InternalServerError")]
#[case::any_other_code(418_u16, "APIError")]
fn error_each_status_class_follows_its_code(#[case] status_code: u16, #[case] class: &str) {
    let status = StatusError {
        status: status_code,
        message: String::new(),
        error_type: None,
        param: None,
        code: None,
    };
    let Failure::Payload(payload) = files_failure(&FilesError::Status(Box::new(status))) else {
        panic!("the status error did not become a payload");
    };
    assert_eq!(payload.class, class);
}

#[test]
fn error_a_status_with_only_a_type_keeps_the_body() {
    let status = StatusError {
        status: 404_u16,
        message: String::from("Error code: 404 - missing"),
        error_type: Some(String::from("invalid_request_error")),
        param: None,
        code: None,
    };
    let Failure::Payload(payload) = files_failure(&FilesError::Status(Box::new(status))) else {
        panic!("the status error did not become a payload");
    };
    let Some(JsonValue::Object(body)) = payload.body else {
        panic!("the status body is not an object");
    };
    assert_eq!(
        body.get("type"),
        Some(&JsonValue::Text(String::from("invalid_request_error")))
    );
    assert_eq!(body.get("code"), None);
}

#[test]
fn error_an_azure_endpoint_key_failure_is_a_key_error() {
    let failure = azure_failure(&AzureFilesError::Endpoint {
        source: EndpointError::MissingApiVersion,
    });
    assert_eq!(
        failure,
        Failure::Key(String::from("the client custom query has no api-version"))
    );
}

#[test]
fn error_an_uninitialized_client_is_a_value_error() {
    let failure = files_failure(&FilesError::NotInitialized);
    let Failure::Value(message) = failure else {
        panic!("the uninitialized client did not become a value error");
    };
    assert!(message.contains("not initialized"));
}

#[test]
fn error_a_rejected_provider_is_a_bad_request_with_the_placeholder_body() {
    let error = FileContentError::<DispatchError>::Unsupported {
        provider: String::from("cohere"),
    };
    let failure = failure_for(&error);
    let Failure::Payload(payload) = failure else {
        panic!("the rejected provider did not become a payload");
    };
    assert_eq!(payload.class, "BadRequestError");
    assert_eq!(payload.status_code, Some(400_u16));
    assert_eq!(payload.llm_provider.as_deref(), Some("cohere"));
    assert_eq!(payload.model.as_deref(), Some("n/a"));
    assert_eq!(
        payload.body,
        Some(JsonValue::Text(String::from("Unsupported provider")))
    );
}

#[test]
fn error_an_unknown_provider_is_a_value_error() {
    let error = FileContentError::<DispatchError>::UnknownProvider {
        provider: String::from("'nope'"),
    };
    let Failure::Value(message) = failure_for(&error) else {
        panic!("the unknown provider did not become a value error");
    };
    assert!(message.contains("is not a valid LlmProviders"));
}

#[test]
fn error_stream_unsupported_sync_is_the_litellm_type_error() {
    let failure = stream_failure(&StreamError::UnsupportedSync);
    assert_eq!(
        failure,
        Failure::Type(String::from(
            "File content stream does not support sync iteration"
        ))
    );
}
