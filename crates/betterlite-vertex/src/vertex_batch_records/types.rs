use betterlite_openai::batches::records::BatchStatus;
use serde::{Deserialize, Serialize};

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

#[derive(Clone, Debug, Default, Deserialize)]
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

// Vertex names twelve job states; the mapping below is LiteLLM's, so an unknown spelling is a decode error
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
pub enum VertexJobState {
    #[serde(rename = "JOB_STATE_UNSPECIFIED")]
    Unspecified,
    #[serde(rename = "JOB_STATE_QUEUED")]
    Queued,
    #[serde(rename = "JOB_STATE_PENDING")]
    Pending,
    #[serde(rename = "JOB_STATE_RUNNING")]
    Running,
    #[serde(rename = "JOB_STATE_SUCCEEDED")]
    Succeeded,
    #[serde(rename = "JOB_STATE_FAILED")]
    Failed,
    #[serde(rename = "JOB_STATE_CANCELLING")]
    Cancelling,
    #[serde(rename = "JOB_STATE_CANCELLED")]
    Cancelled,
    #[serde(rename = "JOB_STATE_PAUSED")]
    Paused,
    #[serde(rename = "JOB_STATE_EXPIRED")]
    Expired,
    #[serde(rename = "JOB_STATE_UPDATING")]
    Updating,
    #[serde(rename = "JOB_STATE_PARTIALLY_SUCCEEDED")]
    PartiallySucceeded,
}

impl VertexJobState {
    #[must_use]
    pub const fn batch_status(self) -> BatchStatus {
        match self {
            Self::Unspecified | Self::Failed => BatchStatus::Failed,
            Self::Queued | Self::Pending => BatchStatus::Validating,
            Self::Running | Self::Paused | Self::Updating => BatchStatus::InProgress,
            Self::Succeeded | Self::PartiallySucceeded => BatchStatus::Completed,
            Self::Cancelling => BatchStatus::Cancelling,
            Self::Cancelled => BatchStatus::Cancelled,
            Self::Expired => BatchStatus::Expired,
        }
    }
}

// LiteLLM reads only gcsSource from a job's inputConfig, and a partial config leaves the file id empty
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
    pub state: Option<Option<VertexJobState>>,
    pub create_time: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct VertexBatchJobPage {
    pub batch_prediction_jobs: Option<Vec<VertexBatchPredictionResponse>>,
    pub next_page_token: Option<String>,
}
