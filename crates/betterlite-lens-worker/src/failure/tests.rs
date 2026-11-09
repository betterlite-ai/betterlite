#![cfg(test)]

use betterlite_lens::analysis::AnalysisError;
use betterlite_lens::coordinator::CoordinatorError;
use betterlite_lens::extraction::ExtractionError;
use betterlite_lens::models::decode::DecodeError;
use betterlite_lens::trace_store::TraceStoreError;
use rstest::rstest;

use super::{DECODE_MESSAGE, STORAGE_MESSAGE, analysis_failure, failure_message};
use crate::api::WorkerError;

#[test]
fn returns_the_storage_text() {
    assert_eq!(
        failure_message(&WorkerError::Storage(String::from("sqlite is full"))),
        STORAGE_MESSAGE
    );
}

#[test]
fn returns_the_timeout_text() {
    assert_eq!(
        failure_message(&WorkerError::Timeout),
        "The worker timed out waiting for the proxy. Check proxy availability and model response times."
    );
}

#[test]
fn returns_the_transport_text() {
    assert_eq!(
        failure_message(&WorkerError::Transport),
        "The worker could not connect to the proxy. Check the proxy URL, network access, and TLS configuration."
    );
}

#[test]
fn returns_the_decode_text() {
    assert_eq!(
        failure_message(&WorkerError::Decode(String::from("bad json"))),
        DECODE_MESSAGE
    );
}

#[rstest]
#[case::model("/lens/worker/l1/j1/model", "Model request")]
#[case::sample("/lens/worker/l1/j1/sample", "Reading trace data")]
#[case::content("/lens/worker/l1/j1/content", "Reading trace data")]
#[case::result("/lens/worker/l1/j1/result", "Saving results")]
#[case::heartbeat("/lens/worker/l1/j1/heartbeat", "Worker request")]
#[case::claim("/lens/worker/claim", "Worker request")]
fn names_the_action_from_the_path_suffix(#[case] path: &str, #[case] action: &str) {
    let message = failure_message(&WorkerError::Status {
        path: path.to_owned(),
        status: 500,
    });
    assert_eq!(
        message,
        format!(
            "{action} failed (HTTP 500). Check proxy and model availability, then retry the investigation."
        )
    );
}

#[rstest]
#[case::bad_request(
    400,
    "Check the configured model and whether the worker's billing key is enabled."
)]
#[case::unauthorized(401, "Check the worker credential and its assigned billing key.")]
#[case::payment_required(
    402,
    "Check the investigation's monthly limit and the worker key's remaining budget."
)]
#[case::forbidden(
    403,
    "Check the worker key's model permissions and access restrictions."
)]
#[case::not_found(
    404,
    "Check that the proxy and worker versions match and the requested model is configured."
)]
#[case::conflict(
    409,
    "This worker no longer owns the run. Check whether it was cancelled or claimed again."
)]
#[case::rate_limited(
    429,
    "The request was rate limited. Retry later or check the worker key's rate limits."
)]
#[case::unlisted(
    500,
    "Check proxy and model availability, then retry the investigation."
)]
fn gives_the_guidance_for_a_status(#[case] status: u16, #[case] guidance: &str) {
    let message = failure_message(&WorkerError::Status {
        path: String::from("/lens/worker/l1/j1/model"),
        status,
    });
    assert_eq!(
        message,
        format!("Model request failed (HTTP {status}). {guidance}")
    );
}

#[test]
fn keeps_the_callback_text_from_an_analysis_failure() {
    let text = String::from(
        "The worker timed out waiting for the proxy. Check proxy availability and model response times.",
    );
    let error = CoordinatorError::Analysis(AnalysisError::Callback(text.clone()));
    assert_eq!(analysis_failure(&error), text);
}

#[test]
fn keeps_the_callback_text_inside_an_extraction_failure() {
    let text = String::from(
        "Saving results failed (HTTP 500). Check proxy and model availability, then retry the investigation.",
    );
    let error = CoordinatorError::Extraction(ExtractionError::Analysis(AnalysisError::Callback(
        text.clone(),
    )));
    assert_eq!(analysis_failure(&error), text);
}

#[test]
fn maps_a_trace_store_failure_to_the_storage_text() {
    let error = CoordinatorError::Analysis(AnalysisError::Store(TraceStoreError::Unavailable));
    assert_eq!(analysis_failure(&error), STORAGE_MESSAGE);
}

#[test]
fn maps_a_trace_store_failure_inside_extraction_to_the_storage_text() {
    let error = CoordinatorError::Extraction(ExtractionError::Analysis(AnalysisError::Store(
        TraceStoreError::Unavailable,
    )));
    assert_eq!(analysis_failure(&error), STORAGE_MESSAGE);
}

#[test]
fn maps_a_validation_failure_to_the_decode_text() {
    let error =
        CoordinatorError::Analysis(AnalysisError::Validation(String::from("the record is off")));
    assert_eq!(analysis_failure(&error), DECODE_MESSAGE);
}

#[test]
fn maps_a_json_failure_to_the_decode_text() {
    let source = serde_json::from_str::<u8>("not a number").unwrap_err();
    let error = CoordinatorError::Analysis(AnalysisError::Json(source));
    assert_eq!(analysis_failure(&error), DECODE_MESSAGE);
}

#[test]
fn maps_a_decode_failure_to_the_decode_text() {
    let source = serde_json::from_str::<u8>("not a number").unwrap_err();
    let error = CoordinatorError::Analysis(AnalysisError::Decode(DecodeError::Json(source)));
    assert_eq!(analysis_failure(&error), DECODE_MESSAGE);
}
