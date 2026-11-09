#![cfg(test)]

use http::HeaderMap;

use super::{ReplicateError, ReplicateFailure};

fn api_error() -> ReplicateError {
    ReplicateError {
        status_code: 422,
        message: String::from("LiteLLM Error - prediction not succeeded - {}"),
        headers: Some(HeaderMap::new()),
    }
}

#[test]
fn replicate_error_displays_its_message() {
    assert_eq!(
        api_error().to_string(),
        "LiteLLM Error - prediction not succeeded - {}"
    );
}

#[test]
fn a_replicate_error_turns_into_the_api_failure() {
    let failure = ReplicateFailure::from(api_error());
    assert_eq!(failure.status_code(), Some(422));
    assert_eq!(
        failure.api_message(),
        Some("LiteLLM Error - prediction not succeeded - {}")
    );
}

#[test]
fn a_failure_that_is_not_replicates_own_carries_no_status_or_message() {
    let supports = ReplicateFailure::SupportsSystemPrompt;
    assert_eq!(supports.status_code(), None);
    assert_eq!(supports.api_message(), None);
    let missing = ReplicateFailure::MissingStatus;
    assert_eq!(missing.status_code(), None);
    assert_eq!(missing.api_message(), None);
    let url_type = ReplicateFailure::PredictionUrlType;
    assert_eq!(url_type.status_code(), None);
    assert_eq!(url_type.api_message(), None);
}
