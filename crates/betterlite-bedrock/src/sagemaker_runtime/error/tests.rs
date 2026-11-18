#![cfg(test)]

use crate::sagemaker_embedding::hf::HfReplyError;
use std::io;

use super::{RuntimeError, RuntimeFailure, timeout_error};
use aws_credential_types::provider::error::CredentialsError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json as Plain;
use betterlite_openai::sagemaker_completion::error::SagemakerError;
use rstest::rstest;

use crate::aws_settings::sources::SourceError;
use crate::credentials::error::CredentialError;
use crate::signing::error::SigningError;

#[test]
fn runtime_errors_preserve_sagemaker_errors_and_map_http_failures() {
    let upstream = SagemakerError {
        status_code: 429,
        message: String::from("throttled"),
        headers: None,
    };
    assert_eq!(
        SagemakerError::from(RuntimeError::Sagemaker(upstream.clone())),
        upstream
    );
    let mapped = SagemakerError::from(RuntimeError::Field { field: "model_id" });
    assert_eq!(mapped.status_code, 500);
    assert!(mapped.message.contains("model_id"));
}

#[test]
fn hf_reply_error_keeps_its_wire_status() {
    let source = serde_json::from_str::<Plain>("invalid").expect_err("bad JSON");
    let mapped = SagemakerError::from(RuntimeError::HfReply(HfReplyError::Parse {
        status_code: 422,
        source,
    }));
    assert_eq!(mapped.status_code, 422);
}

#[rstest]
#[case::missing_credentials(RuntimeError::Credential(CredentialError::MissingCredentials))]
#[case::missing_provider(RuntimeError::Credential(CredentialError::Source(
    SourceError::MissingProvider
)))]
#[case::not_loaded(
    RuntimeError::Credential(CredentialError::Source(SourceError::Provider(
        CredentialsError::not_loaded_no_source()
    )))
)]
#[case::signing_without_credentials(RuntimeError::Signing(SigningError::NoCredentials))]
#[case::signing_credential(RuntimeError::Signing(SigningError::Credential(
    CredentialError::MissingCredentials
)))]
fn missing_credentials_become_a_400_with_botocores_message(#[case] failure: RuntimeError) {
    let mapped = SagemakerError::from(failure);
    assert_eq!(mapped.status_code, 400);
    assert_eq!(mapped.message, "Unable to locate credentials");
}

#[rstest]
#[case::provider_failure(RuntimeError::Credential(CredentialError::Source(
    SourceError::Provider(CredentialsError::unhandled(io::Error::other(
        "the metadata endpoint answered 500"
    )))
)))]
#[case::closed_transport(RuntimeError::Transport(TransportError::Closed))]
fn other_credential_and_transport_failures_stay_a_500(#[case] failure: RuntimeError) {
    let mapped = SagemakerError::from(failure);
    assert_eq!(mapped.status_code, 500);
}

#[test]
fn runtime_failures_keep_a_status_only_for_wire_errors() {
    let wire = RuntimeFailure::from(SagemakerError {
        status_code: 503,
        message: String::from("throttled"),
        headers: None,
    });
    assert_eq!(wire.status_code(), Some(503));
    let source = serde_json::from_str::<Plain>("invalid").expect_err("bad JSON");
    let reply = RuntimeFailure::from(source);
    assert_eq!(reply.status_code(), None);
    assert!(reply.to_string().contains("not JSON"));
}

#[test]
fn an_unreadable_request_body_is_an_internal_failure() {
    let source = serde_json::from_str::<Plain>("invalid").expect_err("bad JSON");
    let mapped = SagemakerError::from(RuntimeError::Json(source));
    assert_eq!(mapped.status_code, 500);
    assert!(mapped.message.contains("cannot be read as JSON"));
}

#[test]
fn a_closed_transport_is_not_a_timeout() {
    let mapped = timeout_error(TransportError::Closed);
    assert_eq!(mapped.status_code, 500);
}
