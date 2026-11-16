#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_openai::api::container_contracts::error::BaseLlmException;
use betterlite_openai::containers::container_file_http::ContainerFileHttpError;

use super::execution::UploadFailure;
use super::test_support::timeout_failure;

#[test]
fn a_provider_answer_carries_its_status_into_the_failure() {
    let error = ContainerFileHttpError::Provider(BaseLlmException {
        status_code: 429,
        message: "too many requests".to_owned(),
        headers: BTreeMap::new(),
    });

    assert_eq!(error.status_code(), Some(429));
    assert_eq!(error.message(), "too many requests");
}

#[test]
fn a_handler_failure_without_a_status_has_none() {
    let error = ContainerFileHttpError::UnknownEndpoint {
        name: "archive_container".to_owned(),
    };

    assert_eq!(error.status_code(), None);
    assert_eq!(error.message(), "Unknown endpoint: archive_container");
}

#[tokio::test]
async fn a_request_timeout_carries_the_timeout_status() {
    let error = timeout_failure().await;

    assert_eq!(error.status_code(), Some(408));
    assert_eq!(
        error.message(),
        "the request ran past its read timeout or total deadline"
    );
}
