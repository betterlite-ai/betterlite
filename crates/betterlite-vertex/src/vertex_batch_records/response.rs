use betterlite_openai::batches::records::{BatchObject, BatchObjectKind};
use serde::Serialize;

use crate::provider_metadata::timestamp::{LocalTimeContext, decode_vertex_timestamp};
use crate::vertex_batch_records::error::BatchRecordsError;
use crate::vertex_batch_records::types::{
    VertexBatchJobPage, VertexBatchPredictionResponse, job_status,
};

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
    let name = response.name.as_deref().unwrap_or_default();
    let id = if name.is_empty() {
        String::new()
    } else {
        name.rsplit('/').next().unwrap_or_default().to_owned()
    };
    Ok(BatchObject {
        id,
        completion_window: "24h".to_owned(),
        created_at,
        endpoint: String::new(),
        input_file_id: first_input_uri(response),
        object: BatchObjectKind::Batch,
        status: job_status(response.state.as_ref())?,
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
        output_file_id: output_file(response),
        request_counts: None,
        usage: None,
        // a projection has no passthrough fields, and serde_json's map is a disallowed type to name
        provider_fields: <_>::default(),
    })
}

fn first_input_uri(response: &VertexBatchPredictionResponse) -> String {
    response
        .input_config
        .as_ref()
        .and_then(|config| config.gcs_source.as_ref())
        .and_then(|source| source.uris.first())
        .cloned()
        .unwrap_or_default()
}

// LiteLLM strips every trailing slash and appends predictions.jsonl only to a nonempty directory
fn output_file(response: &VertexBatchPredictionResponse) -> Option<String> {
    let directory = response
        .output_info
        .as_ref()
        .and_then(|info| info.gcs_output_directory.as_deref())
        .unwrap_or_default()
        .trim_end_matches('/');
    (!directory.is_empty()).then(|| format!("{directory}/predictions.jsonl"))
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
    Ok(BatchList {
        object: BatchListObject::List,
        first_id: data.first().map(|batch| batch.id.clone()),
        last_id: data.last().map(|batch| batch.id.clone()),
        data,
        has_more: token.as_deref().is_some_and(|value| !value.is_empty()),
        next_page_token: token,
    })
}
