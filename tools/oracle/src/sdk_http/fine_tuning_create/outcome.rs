use crate::sdk_http::error::SdkHttpError;
use crate::sdk_http::result::Failure;
use betterlite_openai::fine_tuning::job::FineTuneJob;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

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
#[serde(deny_unknown_fields)]
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
pub fn job_view(job: &FineTuneJob) -> Result<JobView, SdkHttpError> {
    let value = serde_json::to_value(job).map_err(SdkHttpError::Serialize)?;
    let field = |name: &str| value.get(name).cloned().unwrap_or(Value::Null);
    serde_json::from_value(json!({
        "id": field("id"),
        "object": field("object"),
        "created_at": field("created_at"),
        "model": field("model"),
        "status": field("status"),
        "training_file": field("training_file"),
        "validation_file": field("validation_file"),
        "organization_id": field("organization_id"),
        "result_files": field("result_files"),
        "fine_tuned_model": field("fine_tuned_model"),
        "seed": field("seed"),
        "hyperparameters": field("hyperparameters"),
    }))
    .map_err(SdkHttpError::Serialize)
}
