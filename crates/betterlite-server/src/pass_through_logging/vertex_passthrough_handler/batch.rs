use betterlite_core::api::media_relay_logging::spend::decimal_json;
use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_logging_state::response::PassThroughResponse;
use betterlite_logging::passthrough_stream_logging::types::HandlerOutcome;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use chrono::Utc;

use super::{VertexPassthroughError, VertexPassthroughInput};
use crate::pass_through_logging::anthropic_batch::BatchLoggingResponse;
use crate::pass_through_logging::vertex_batch_logging::{
    VertexBatchInput, VertexBatchKwargs, VertexBatchLogging, VertexBatchState,
};

// batch_prediction_jobs_handler: B3641 writes the call log and hands back the record and its kwargs.
pub(super) fn batch_prediction_jobs(
    logger: &VertexBatchLogging,
    input: VertexPassthroughInput<'_>,
) -> Result<HandlerOutcome, VertexPassthroughError> {
    let request_metadata = request_metadata(&input.kwargs)?;
    let written = logger.batch_prediction_jobs_handler(
        VertexBatchInput {
            status_code: input.status.as_u16(),
            body: input.response_body,
            url_route: input.url_route,
            start_time: input.start_time.with_timezone(&Utc),
            request_metadata: &request_metadata,
        },
        input.call_log,
    );
    let record = batch_record(&written.result)?;
    Ok(HandlerOutcome {
        result: Some(PassThroughResponse::VertexBatchRecord(record)),
        kwargs: batch_kwargs(input.kwargs, &written.kwargs),
    })
}

fn batch_record(result: &BatchLoggingResponse) -> Result<Json, VertexPassthroughError> {
    serde_json::to_value(result)
        .and_then(serde_json::from_value)
        .map_err(|source| VertexPassthroughError::Encode { source })
}

fn batch_kwargs(
    kwargs: PassThroughLoggingKwargs,
    logged: &VertexBatchKwargs,
) -> PassThroughLoggingKwargs {
    let mut entries = kwargs.entries;
    entries.insert(
        "response_cost".to_owned(),
        decimal_json(logged.response_cost),
    );
    entries.insert("model".to_owned(), Json::Text(logged.model.clone()));
    if let Some(batch_id) = &logged.batch_id {
        entries.insert("batch_id".to_owned(), Json::Text(batch_id.clone()));
    }
    if let Some(unified) = &logged.unified_object_id {
        entries.insert("unified_object_id".to_owned(), Json::Text(unified.clone()));
    }
    entries.insert(
        "batch_job_state".to_owned(),
        Json::Text(batch_state_word(logged.batch_job_state).to_owned()),
    );
    PassThroughLoggingKwargs { entries }
}

// LiteLLM reads its metadata from kwargs["litellm_params"]["metadata"], defaulting each level to {}.
fn request_metadata(
    kwargs: &PassThroughLoggingKwargs,
) -> Result<JsonObject, VertexPassthroughError> {
    let params = match kwargs.entries.get("litellm_params") {
        None | Some(Json::Null) => return Ok(JsonObject::new()),
        Some(Json::Object(fields)) => fields,
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(VertexPassthroughError::Detail {
                field: "litellm_params",
            });
        }
    };
    match params.get("metadata") {
        None | Some(Json::Null) => Ok(JsonObject::new()),
        Some(Json::Object(fields)) => Ok(fields.clone()),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(VertexPassthroughError::Detail { field: "metadata" })
        }
    }
}

const fn batch_state_word(state: VertexBatchState) -> &'static str {
    match state {
        VertexBatchState::JobStatePending => "JOB_STATE_PENDING",
        VertexBatchState::JobStateFailed => "JOB_STATE_FAILED",
    }
}
