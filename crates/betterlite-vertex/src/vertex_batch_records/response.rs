use betterlite_openai::batches::records::{BatchObject, BatchObjectKind, BatchStatus};
use serde::Serialize;

use crate::provider_metadata::timestamp::{LocalTimeContext, decode_vertex_timestamp};
use crate::vertex_batch_records::error::BatchRecordsError;
use crate::vertex_batch_records::types::{
    VertexBatchJobPage, VertexBatchPredictionResponse, VertexJobState,
};

const COMPLETION_WINDOW: &str = "24h";
const PREDICTIONS_FILE: &str = "predictions.jsonl";

#[derive(Clone, Copy, Debug, Serialize)]
pub enum BatchListObject {
    #[serde(rename = "list")]
    List,
}

#[derive(Clone, Debug, Serialize)]
pub struct BatchList {
    pub object: BatchListObject,
    pub data: Vec<BatchObject>,
    pub first_id: Option<String>,
    pub last_id: Option<String>,
    pub has_more: bool,
    pub next_page_token: Option<String>,
}

pub fn to_batch_object(
    response: &VertexBatchPredictionResponse,
    local: &LocalTimeContext,
) -> Result<BatchObject, BatchRecordsError> {
    let created_at = decode_vertex_timestamp(
        response
            .create_time
            .as_deref()
            .unwrap_or_default()
            .as_bytes(),
        local,
    )
    .map_err(BatchRecordsError::CreateTime)?;
    Ok(BatchObject {
        id: batch_id(response),
        completion_window: COMPLETION_WINDOW.to_owned(),
        created_at,
        endpoint: String::new(),
        input_file_id: input_file_id(response),
        object: BatchObjectKind::Batch,
        status: batch_status(response.state.as_ref())?,
        cancelled_at: None,
        cancelling_at: None,
        completed_at: None,
        error_file_id: None,
        errors: None,
        expired_at: None,
        expires_at: None,
        failed_at: None,
        finalizing_at: None,
        in_progress_at: None,
        metadata: None,
        model: None,
        output_file_id: output_file_id(response),
        request_counts: None,
        usage: None,
        // a projection has no passthrough fields, and serde_json's map is a disallowed type to name
        provider_fields: <_>::default(),
    })
}

pub fn to_batch_list(
    page: &VertexBatchJobPage,
    local: &LocalTimeContext,
) -> Result<BatchList, BatchRecordsError> {
    let jobs = page.batch_prediction_jobs.as_deref().unwrap_or_default();
    let mut data = Vec::with_capacity(jobs.len());
    for job in jobs {
        data.push(to_batch_object(job, local)?);
    }
    let token = page.next_page_token.clone();
    let has_more = token.as_deref().is_some_and(|value| !value.is_empty());
    Ok(BatchList {
        object: BatchListObject::List,
        first_id: data.first().map(|batch| batch.id.clone()),
        last_id: data.last().map(|batch| batch.id.clone()),
        data,
        has_more,
        next_page_token: token,
    })
}

// Vertex reports the job's resource name; the last path segment is the id OpenAI clients see
fn batch_id(response: &VertexBatchPredictionResponse) -> String {
    let name = response.name.as_deref().unwrap_or_default();
    if name.is_empty() {
        return String::new();
    }
    name.rsplit('/').next().unwrap_or_default().to_owned()
}

fn input_file_id(response: &VertexBatchPredictionResponse) -> String {
    let Some(config) = response.input_config.as_ref() else {
        return String::new();
    };
    let Some(source) = config.gcs_source.as_ref() else {
        return String::new();
    };
    source.uris.first().cloned().unwrap_or_default()
}

fn output_file_id(response: &VertexBatchPredictionResponse) -> Option<String> {
    let directory = response
        .output_info
        .as_ref()
        .and_then(|info| info.gcs_output_directory.as_deref())
        .unwrap_or_default()
        .trim_end_matches('/');
    if directory.is_empty() {
        return None;
    }
    Some(format!("{directory}/{PREDICTIONS_FILE}"))
}

const fn batch_status(
    state: Option<&Option<VertexJobState>>,
) -> Result<BatchStatus, BatchRecordsError> {
    match state {
        None => Ok(VertexJobState::Unspecified.batch_status()),
        Some(None) => Err(BatchRecordsError::NullJobState),
        Some(Some(job_state)) => Ok(job_state.batch_status()),
    }
}
