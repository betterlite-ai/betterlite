mod arguments;
mod detection;
mod messages;
pub mod native;
pub mod outcome;
pub mod wire;

use thiserror::Error;

use wire::{Assistant, Call, Response, ResponseFormat, Results};

#[derive(Debug, Error)]
pub enum TransformationError {
    #[error("cannot encode search arguments")]
    Json(#[from] serde_json::Error),
    #[error("search result count {results} is less than call count {calls}")]
    MissingResults { calls: usize, results: usize },
    #[error("search arguments exceed the nesting or size limit")]
    ArgumentsTooComplex,
    #[error("tool_calls count {calls} does not match search_outcomes count {outcomes}")]
    MismatchedOutcomes { calls: usize, outcomes: usize },
}

pub struct WebSearchTransformation;

impl WebSearchTransformation {
    pub fn transform_request(
        response: &Response,
        stream: bool,
        format: &str,
    ) -> Result<(bool, Vec<Call>), TransformationError> {
        if stream {
            tracing::warn!("Unexpected streaming response; skipping search interception");
            return Ok((false, Vec::new()));
        }
        let calls = detection::detect(response, ResponseFormat::parse(format))?;
        Ok((!calls.is_empty(), calls))
    }

    pub fn transform_response(
        calls: &[Call],
        results: &[String],
        format: &str,
    ) -> Result<(Assistant, Results), TransformationError> {
        if results.len() < calls.len() {
            return Err(TransformationError::MissingResults {
                calls: calls.len(),
                results: results.len(),
            });
        }
        messages::assistant_and_results(calls, results, ResponseFormat::parse(format))
    }
}
