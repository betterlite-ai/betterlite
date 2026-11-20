use crate::sdk_http::error::SdkHttpError;
use crate::sdk_http::result::Failure;
use betterlite_openai::fine_tuning::job::FineTuneJob;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub runs: Vec<RunOutcome>,
}
// exactly one of job and failure is set; request_url is what the Vertex transport resolved before redirecting
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunOutcome {
    pub job: Option<JobView>,
    pub failure: Option<Failure>,
    pub request_url: Option<String>,
}
// the fields of the OpenAI-spec job both sides keep, under the SDK's own names
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobView {
    pub id: String,
    pub object: String,
    pub created_at: i64,
    pub model: String,
    pub status: String,
    pub training_file: String,
    pub validation_file: Option<String>,
    pub organization_id: String,
    pub result_files: Vec<String>,
    pub fine_tuned_model: Option<String>,
    pub seed: Option<i64>,
    pub hyperparameters: Value,
}
pub(crate) fn job_view(job: &FineTuneJob) -> Result<JobView, SdkHttpError> {
    serde_json::from_value(serde_json::to_value(job).map_err(SdkHttpError::Serialize)?)
        .map_err(SdkHttpError::Serialize)
}
