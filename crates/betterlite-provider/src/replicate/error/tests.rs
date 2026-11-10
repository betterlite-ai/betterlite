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
    assert_eq!(status(&failure), Some(422));
    assert_eq!(
        message(&failure),
        Some("LiteLLM Error - prediction not succeeded - {}")
    );
}

#[test]
fn a_failure_that_is_not_replicates_own_carries_no_status_or_message() {
    let supports = ReplicateFailure::SupportsSystemPrompt;
    assert_eq!(status(&supports), None);
    assert_eq!(message(&supports), None);
    let missing = ReplicateFailure::MissingStatus;
    assert_eq!(status(&missing), None);
    assert_eq!(message(&missing), None);
    let url_type = ReplicateFailure::PredictionUrlType;
    assert_eq!(status(&url_type), None);
    assert_eq!(message(&url_type), None);
}

fn status(failure: &ReplicateFailure) -> Option<u16> {
    failure.api().map(|error| error.status_code)
}

fn message(failure: &ReplicateFailure) -> Option<&str> {
    failure.api().map(|error| error.message.as_str())
}
