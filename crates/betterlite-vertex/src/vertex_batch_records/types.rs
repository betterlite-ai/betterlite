use betterlite_openai::batches::records::BatchStatus;
use serde::{Deserialize, Serialize};

use crate::vertex_batch_records::error::BatchRecordsError;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct GcsSource {
    pub uris: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InputConfig {
    pub instances_format: String,
    pub gcs_source: GcsSource,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GcsDestination {
    pub output_uri_prefix: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputConfig {
    pub predictions_format: String,
    pub gcs_destination: GcsDestination,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct OutputInfo {
    pub gcs_output_directory: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VertexAIBatchPredictionJob {
    pub display_name: String,
    pub model: String,
    pub input_config: InputConfig,
    pub output_config: OutputConfig,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BatchJobInputConfig {
    pub gcs_source: Option<GcsSource>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct VertexBatchPredictionResponse {
    pub name: Option<String>,
    pub input_config: Option<BatchJobInputConfig>,
    pub output_info: Option<OutputInfo>,
    // absent takes Vertex's default state, while an explicit null is the KeyError LiteLLM raises
    #[serde(with = "serde_with::rust::double_option")]
    pub state: Option<Option<String>>,
    pub create_time: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct VertexBatchJobPage {
    pub batch_prediction_jobs: Option<Vec<VertexBatchPredictionResponse>>,
    pub next_page_token: Option<String>,
}

// Vertex names twelve job states; LiteLLM's map fails on any other spelling
pub fn job_status(state: Option<&Option<String>>) -> Result<BatchStatus, BatchRecordsError> {
    let name = match state {
        None => "JOB_STATE_UNSPECIFIED",
        Some(None) => return Err(BatchRecordsError::NullJobState),
        Some(Some(value)) => value.as_str(),
    };
    match name {
        "JOB_STATE_UNSPECIFIED" | "JOB_STATE_FAILED" => Ok(BatchStatus::Failed),
        "JOB_STATE_QUEUED" | "JOB_STATE_PENDING" => Ok(BatchStatus::Validating),
        "JOB_STATE_RUNNING" | "JOB_STATE_PAUSED" | "JOB_STATE_UPDATING" => {
            Ok(BatchStatus::InProgress)
        }
        "JOB_STATE_SUCCEEDED" | "JOB_STATE_PARTIALLY_SUCCEEDED" => Ok(BatchStatus::Completed),
        "JOB_STATE_CANCELLING" => Ok(BatchStatus::Cancelling),
        "JOB_STATE_CANCELLED" => Ok(BatchStatus::Cancelled),
        "JOB_STATE_EXPIRED" => Ok(BatchStatus::Expired),
        other => Err(BatchRecordsError::UnknownJobState {
            state: other.to_owned(),
        }),
    }
}
