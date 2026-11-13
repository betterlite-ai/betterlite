use betterlite_openai::fine_tuning::job::{
    FineTuneJob, JobObject, JobStatus, WandbIntegration, decode_fine_tune_job,
};
use serde::{Deserialize, Serialize};

use super::TuningRecordError;
use super::timestamp::created_at;
use crate::provider_metadata::timestamp::LocalTimeContext;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ResponseHyperparameters {
    #[serde(rename = "epoch_count", with = "serde_with::rust::double_option")]
    epoch_count: Option<Option<i64>>,
    #[serde(rename = "learning_rate_multiplier")]
    learning_rate_multiplier: Option<f64>,
    #[serde(rename = "adapter_size", with = "serde_with::rust::double_option")]
    adapter_size: Option<Option<String>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ResponseSupervisedTuningSpec {
    #[serde(rename = "trainingDatasetUri")]
    training_dataset_uri: Option<String>,
    #[serde(rename = "hyperParameters")]
    hyperparameters: Option<ResponseHyperparameters>,
}

#[derive(Debug, Deserialize)]
struct VertexTuningJob {
    #[serde(default)]
    name: Option<String>,
    #[serde(
        rename = "tunedModelDisplayName",
        default,
        with = "serde_with::rust::double_option"
    )]
    tuned_model_display_name: Option<Option<String>>,
    #[serde(rename = "baseModel", default)]
    base_model: Option<String>,
    #[serde(rename = "supervisedTuningSpec", default)]
    supervised_tuning_spec: Option<ResponseSupervisedTuningSpec>,
    #[serde(default, with = "serde_with::rust::double_option")]
    state: Option<Option<String>>,
    #[serde(rename = "createTime", default)]
    create_time: Option<String>,
}

#[derive(Debug, Serialize)]
struct OpenAiHyperparameters {
    learning_rate_multiplier: Option<f64>,
    n_epochs: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    adapter_size: Option<Option<String>>,
}

#[derive(Debug, Serialize)]
struct OpenAiTuningJob {
    id: String,
    created_at: i64,
    fine_tuned_model: Option<String>,
    hyperparameters: OpenAiHyperparameters,
    model: String,
    object: JobObject,
    organization_id: String,
    result_files: Vec<String>,
    seed: i64,
    status: JobStatus,
    training_file: String,
    integrations: Vec<WandbIntegration>,
}

pub fn decode_vertex_tuning_job(
    bytes: &[u8],
    local: &LocalTimeContext,
) -> Result<FineTuneJob, TuningRecordError> {
    let response: VertexTuningJob =
        serde_json::from_slice(bytes).map_err(|source| TuningRecordError::Json(source))?;
    let record = openai_record(&response, local)?;
    let encoded =
        serde_json::to_vec(&record).map_err(|source| TuningRecordError::Json(source))?;
    decode_fine_tune_job(&encoded, false).map_err(|source| TuningRecordError::Job(source))
}

fn openai_record(
    response: &VertexTuningJob,
    local: &LocalTimeContext,
) -> Result<OpenAiTuningJob, TuningRecordError> {
    let state = response
        .state
        .as_ref()
        .ok_or(TuningRecordError::MissingState)?;
    let spec = response.supervised_tuning_spec.as_ref();
    Ok(OpenAiTuningJob {
        id: response.name.clone().unwrap_or_default(),
        created_at: created_at(response.create_time.as_deref(), local),
        // LiteLLM's .get(..., "") keeps a missing name as "" but passes an explicit null through
        fine_tuned_model: match &response.tuned_model_display_name {
            None => Some(String::new()),
            Some(value) => value.clone(),
        },
        hyperparameters: openai_hyperparameters(
            spec.and_then(|spec| spec.hyperparameters.as_ref()),
        ),
        model: response.base_model.clone().unwrap_or_default(),
        object: JobObject::FineTuningJob,
        organization_id: String::new(),
        result_files: Vec::new(),
        seed: 0,
        status: match state.as_deref() {
            Some("JOB_STATE_RUNNING") => JobStatus::Running,
            Some("JOB_STATE_SUCCEEDED") => JobStatus::Succeeded,
            Some("JOB_STATE_FAILED") => JobStatus::Failed,
            Some("JOB_STATE_CANCELLED") => JobStatus::Cancelled,
            // LiteLLM starts at queued, so pending, null and every state it does not know stay there
            _ => JobStatus::Queued,
        },
        training_file: spec
            .and_then(|spec| spec.training_dataset_uri.clone())
            .unwrap_or_default(),
        integrations: Vec::new(),
    })
}

fn openai_hyperparameters(values: Option<&ResponseHyperparameters>) -> OpenAiHyperparameters {
    OpenAiHyperparameters {
        learning_rate_multiplier: values.and_then(|values| values.learning_rate_multiplier),
        // LiteLLM pops epoch_count with a default of 0, and a null value stays null
        n_epochs: match values.and_then(|values| values.epoch_count) {
            None => Some(0),
            Some(None) => None,
            Some(Some(count)) => Some(count),
        },
        adapter_size: values.and_then(|values| values.adapter_size),
    }
}
