use std::time::Duration;

use betterlite_openai::api::batch_records::{CreateBatchRequest, RetrieveBatchRequest};
use betterlite_openai::batches::records::{BatchCreate, BatchQuery};
use betterlite_openai::batches::sdk::Timeout;

const DEFAULT_TIMEOUT_SECONDS: f64 = 600.0;

// a request field B1036 or B1037 left out, which their own builders always set
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub(crate) enum BodyError {
    #[error("the batch request is missing its {field}")]
    Missing { field: &'static str },
}

pub(crate) fn batch_create(body: &CreateBatchRequest) -> Result<BatchCreate, BodyError> {
    Ok(BatchCreate {
        completion_window: body.completion_window.ok_or(BodyError::Missing {
            field: "completion_window",
        })?,
        endpoint: body
            .endpoint
            .ok_or(BodyError::Missing { field: "endpoint" })?,
        input_file_id: body.input_file_id.clone().ok_or(BodyError::Missing {
            field: "input_file_id",
        })?,
        metadata: body.metadata.clone(),
        output_expires_after: body.output_expires_after,
        extra_headers: body.extra_headers.clone(),
        extra_body: body.extra_body.clone(),
        timeout: body.timeout,
    })
}

pub(crate) fn batch_query(request: &RetrieveBatchRequest) -> Result<BatchQuery, BodyError> {
    Ok(BatchQuery {
        batch_id: request
            .batch_id
            .clone()
            .ok_or(BodyError::Missing { field: "batch_id" })?,
        extra_headers: request.extra_headers.clone(),
        extra_body: request.extra_body.clone(),
        timeout: request.timeout,
    })
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub(crate) enum DurationError {
    #[error("the timeout must be a finite, non-negative number of seconds, not {seconds}")]
    Invalid { seconds: f64 },
}

pub(crate) fn duration_of(timeout: &Timeout) -> Result<Duration, DurationError> {
    match timeout {
        Timeout::Seconds(seconds) => Duration::try_from_secs_f64(*seconds)
            .map_err(|_source| DurationError::Invalid { seconds: *seconds }),
        Timeout::Phases(phases) => {
            let seconds = phases.read.unwrap_or(DEFAULT_TIMEOUT_SECONDS);
            Duration::try_from_secs_f64(seconds)
                .map_err(|_source| DurationError::Invalid { seconds })
        }
    }
}
