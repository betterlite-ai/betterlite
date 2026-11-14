use super::error::MistralBatchesError;
use super::records::{
    MistralBatchError, MistralBatchJob, MistralBatchStatus, MistralCreateBatchJobRequest,
    MistralGetRequest,
};
use crate::api::batch_records::{BatchFailure, BatchFailures, CreateBatchRequest, RequestCounts};
use crate::batches::records::{BatchObject, BatchObjectKind, BatchStatus};
use crate::mistral_identity::{auth_headers, base_url};
use crate::openai_base_config::Headers;
use betterlite_config::sections::mistral_resources::Settings;
use betterlite_http::url_path::encode_segment;
use http::Method;
use secrecy::SecretString;
use serde_json::Map;
const COMPLETION_WINDOW: &str = "24h";
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchCreatePlan {
    pub url: String,
    pub body: MistralCreateBatchJobRequest,
}
pub fn create_batch(
    data: &CreateBatchRequest,
    model: &str,
    api_base: Option<&str>,
    settings: &Settings,
) -> Result<BatchCreatePlan, MistralBatchesError> {
    let input_file_id = data
        .input_file_id
        .clone()
        .ok_or(MistralBatchesError::InputFileRequired)?;
    let endpoint = data.endpoint.ok_or(MistralBatchesError::EndpointRequired)?;
    let metadata = data.metadata.clone().filter(|map| !map.is_empty());
    Ok(BatchCreatePlan {
        url: format!("{}/v1/batch/jobs", base_url(api_base, settings)),
        body: MistralCreateBatchJobRequest {
            input_files: vec![input_file_id],
            endpoint,
            model: model.to_owned(),
            metadata,
        },
    })
}
pub fn retrieve_batch(
    batch_id: &str,
    api_base: Option<&str>,
    api_key: Option<&SecretString>,
    settings: &Settings,
) -> Result<MistralGetRequest, MistralBatchesError> {
    let encoded = encode_segment(batch_id, "batch_id")?;
    let url = format!("{}/v1/batch/jobs/{encoded}", base_url(api_base, settings));
    let headers = auth_headers(&Headers::new(), api_key, settings)?;
    Ok(MistralGetRequest {
        method: Method::GET,
        url,
        headers,
    })
}
#[must_use]
pub const fn openai_status(status: MistralBatchStatus) -> BatchStatus {
    match status {
        MistralBatchStatus::Queued => BatchStatus::Validating,
        MistralBatchStatus::Running => BatchStatus::InProgress,
        MistralBatchStatus::Success => BatchStatus::Completed,
        MistralBatchStatus::Failed => BatchStatus::Failed,
        MistralBatchStatus::TimeoutExceeded => BatchStatus::Expired,
        MistralBatchStatus::CancellationRequested => BatchStatus::Cancelling,
        MistralBatchStatus::Cancelled => BatchStatus::Cancelled,
    }
}
fn to_batch_errors(errors: &[MistralBatchError]) -> Option<BatchFailures> {
    let data: Vec<BatchFailure> = errors
        .iter()
        .map(|error| BatchFailure {
            code: None,
            line: None,
            message: Some(if error.count > 1 {
                format!("{} (x{})", error.message, error.count)
            } else {
                error.message.clone()
            }),
            param: None,
        })
        .collect();
    (!data.is_empty()).then(|| BatchFailures {
        data: Some(data),
        object: Some(String::from("list")),
    })
}
#[must_use]
pub fn to_batch_object(job: &MistralBatchJob) -> BatchObject {
    let status = openai_status(job.status);
    let terminal =
        |candidate: BatchStatus| (status == candidate).then_some(job.completed_at).flatten();
    BatchObject {
        id: job.id.clone(),
        completion_window: COMPLETION_WINDOW.to_owned(),
        created_at: job.created_at,
        endpoint: job.endpoint.clone(),
        input_file_id: job.input_files.first().cloned().unwrap_or_default(),
        object: BatchObjectKind::Batch,
        status,
        cancelled_at: terminal(BatchStatus::Cancelled),
        cancelling_at: None,
        completed_at: terminal(BatchStatus::Completed),
        error_file_id: job.error_file.clone(),
        errors: to_batch_errors(&job.errors),
        expired_at: terminal(BatchStatus::Expired),
        expires_at: None,
        failed_at: terminal(BatchStatus::Failed),
        finalizing_at: None,
        in_progress_at: job.started_at,
        metadata: job.metadata.clone(),
        model: None,
        output_file_id: job.output_file.clone(),
        request_counts: Some(RequestCounts {
            completed: job.succeeded_requests,
            failed: job.failed_requests,
            total: job.total_requests,
        }),
        usage: None,
        provider_fields: Map::new(),
    }
}
