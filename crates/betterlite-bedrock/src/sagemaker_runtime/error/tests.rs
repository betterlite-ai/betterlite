#![cfg(test)]

use super::RuntimeError;
use aws_credential_types::provider::error::CredentialsError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::sagemaker_completion::error::SagemakerError;

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
    let source = serde_json::from_str::<serde_json::Value>("invalid").expect_err("bad JSON");
    let mapped = SagemakerError::from(RuntimeError::HfReply(
        crate::sagemaker_embedding::hf::HfReplyError::Parse {
            status_code: 422,
            source,
        },
    ));
    assert_eq!(mapped.status_code, 422);
}

#[test]
fn missing_credentials_become_a_400_with_botocores_message() {
    let failures = [
        RuntimeError::Credential(CredentialError::MissingCredentials),
        RuntimeError::Credential(CredentialError::Source(SourceError::MissingProvider)),
        RuntimeError::Credential(CredentialError::Source(SourceError::Provider(
            CredentialsError::not_loaded_no_source(),
        ))),
        RuntimeError::Signing(SigningError::NoCredentials),
        RuntimeError::Signing(SigningError::Credential(
            CredentialError::MissingCredentials,
        )),
    ];
    for failure in failures {
        let mapped = SagemakerError::from(failure);
        assert_eq!(mapped.status_code, 400);
        assert_eq!(mapped.message, "Unable to locate credentials");
    }
}

#[test]
fn other_credential_and_transport_failures_stay_a_500() {
    let provider =
        CredentialsError::unhandled(std::io::Error::other("the metadata endpoint answered 500"));
    let failures = [
        RuntimeError::Credential(CredentialError::Source(SourceError::Provider(provider))),
        RuntimeError::Transport(TransportError::Closed),
    ];
    for failure in failures {
        let mapped = SagemakerError::from(failure);
        assert_eq!(mapped.status_code, 500);
    }
}
