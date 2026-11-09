use betterlite_lens::analysis::AnalysisError;
use betterlite_lens::coordinator::CoordinatorError;
use betterlite_lens::extraction::ExtractionError;

use crate::api::WorkerError;

pub const STORAGE_MESSAGE: &str =
    "Worker temporary storage failed. Increase its capacity or reduce analysis parallelism.";
pub const DECODE_MESSAGE: &str = "The worker could not read an analysis response. Check structured JSON support and matching proxy/worker versions.";

#[must_use]
pub fn failure_message(error: &WorkerError) -> String {
    match error {
        WorkerError::Storage(_text) => STORAGE_MESSAGE.to_owned(),
        WorkerError::Timeout => {
            "The worker timed out waiting for the proxy. Check proxy availability and model response times."
                .to_owned()
        }
        WorkerError::Transport => {
            "The worker could not connect to the proxy. Check the proxy URL, network access, and TLS configuration."
                .to_owned()
        }
        WorkerError::Status { path, status } => {
            let action = if path.ends_with("/model") {
                "Model request"
            } else if path.ends_with("/sample") || path.ends_with("/content") {
                "Reading trace data"
            } else if path.ends_with("/result") {
                "Saving results"
            } else {
                "Worker request"
            };
            format!("{action} failed (HTTP {status}). {}", guidance(*status))
        }
        WorkerError::Decode(_text) => DECODE_MESSAGE.to_owned(),
    }
}

#[must_use]
pub fn analysis_failure(error: &CoordinatorError) -> String {
    match error {
        CoordinatorError::Analysis(inner)
        | CoordinatorError::Extraction(ExtractionError::Analysis(inner)) => analysis_text(inner),
        CoordinatorError::Extraction(ExtractionError::Count(_)) | CoordinatorError::Count(_) => {
            DECODE_MESSAGE.to_owned()
        }
    }
}

fn analysis_text(error: &AnalysisError) -> String {
    match error {
        AnalysisError::Callback(text) => text.clone(),
        AnalysisError::Store(source) => failure_message(&WorkerError::Storage(source.to_string())),
        AnalysisError::Decode(_)
        | AnalysisError::Join(_)
        | AnalysisError::Json(_)
        | AnalysisError::Validation(_) => DECODE_MESSAGE.to_owned(),
    }
}

const fn guidance(status: u16) -> &'static str {
    match status {
        400 => "Check the configured model and whether the worker's billing key is enabled.",
        401 => "Check the worker credential and its assigned billing key.",
        402 => "Check the investigation's monthly limit and the worker key's remaining budget.",
        403 => "Check the worker key's model permissions and access restrictions.",
        404 => {
            "Check that the proxy and worker versions match and the requested model is configured."
        }
        409 => {
            "This worker no longer owns the run. Check whether it was cancelled or claimed again."
        }
        429 => "The request was rate limited. Retry later or check the worker key's rate limits.",
        _ => "Check proxy and model availability, then retry the investigation.",
    }
}

#[path = "failure/tests.rs"]
mod tests;
