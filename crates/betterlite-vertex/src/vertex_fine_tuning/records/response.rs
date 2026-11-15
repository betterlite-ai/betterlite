use betterlite_openai::fine_tuning::job::{
    FineTuneJob, JobObject, JobStatus, WandbIntegration, decode_fine_tune_job,
};
use serde::{Deserialize, Serialize};

use super::TuningRecordError;
use super::timestamp::created_at;
use crate::provider_metadata::timestamp::LocalTimeContext;

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(untagged)]
enum Nullable<T> {
    Value(T),
    Null(()),
    #[default]
    #[serde(skip_deserializing)]
    Absent,
}

impl<T: Clone> Nullable<T> {
    fn or_if_absent(&self, absent: T) -> Option<T> {
        match self {
            Self::Absent => Some(absent),
            Self::Null(()) => None,
            Self::Value(value) => Some(value.clone()),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ResponseHyperparameters {
    epoch_count: Nullable<i64>,
    learning_rate_multiplier: Option<f64>,
    adapter_size: Nullable<String>,
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
    #[serde(rename = "tunedModelDisplayName", default)]
    tuned_model_display_name: Nullable<String>,
    #[serde(rename = "baseModel", default)]
    base_model: Option<String>,
    #[serde(rename = "supervisedTuningSpec", default)]
    supervised_tuning_spec: Option<ResponseSupervisedTuningSpec>,
    #[serde(default)]
    state: Nullable<String>,
    #[serde(rename = "createTime", default)]
    create_time: Option<String>,
}

#[derive(Debug, Serialize)]
struct OpenAiHyperparameters {
    learning_rate_multiplier: Option<f64>,
    n_epochs: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    adapter_size: Option<Nullable<String>>,
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
        serde_json::from_slice(bytes).map_err(TuningRecordError::Json)?;
    let record = openai_record(&response, local)?;
    let encoded = serde_json::to_vec(&record).map_err(TuningRecordError::Json)?;
    decode_fine_tune_job(&encoded, false).map_err(TuningRecordError::Job)
}

fn openai_record(
    response: &VertexTuningJob,
    local: &LocalTimeContext,
) -> Result<OpenAiTuningJob, TuningRecordError> {
    let state = match &response.state {
        Nullable::Absent => return Err(TuningRecordError::MissingState),
        Nullable::Null(()) => None,
        Nullable::Value(text) => Some(text.as_str()),
    };
    let spec = response.supervised_tuning_spec.as_ref();
    Ok(OpenAiTuningJob {
        id: response.name.clone().unwrap_or_default(),
        created_at: created_at(response.create_time.as_deref(), local),
        fine_tuned_model: response
            .tuned_model_display_name
            .or_if_absent(String::new()),
        hyperparameters: openai_hyperparameters(
            spec.and_then(|found| found.hyperparameters.as_ref()),
        ),
        model: response.base_model.clone().unwrap_or_default(),
        object: JobObject::FineTuningJob,
        organization_id: String::new(),
        result_files: Vec::new(),
        seed: 0,
        status: match state {
            Some("JOB_STATE_RUNNING") => JobStatus::Running,
            Some("JOB_STATE_SUCCEEDED") => JobStatus::Succeeded,
            Some("JOB_STATE_FAILED") => JobStatus::Failed,
            Some("JOB_STATE_CANCELLED") => JobStatus::Cancelled,
            _ => JobStatus::Queued,
        },
        training_file: spec
            .and_then(|found| found.training_dataset_uri.clone())
            .unwrap_or_default(),
        integrations: Vec::new(),
    })
}

fn openai_hyperparameters(found: Option<&ResponseHyperparameters>) -> OpenAiHyperparameters {
    let fallback = ResponseHyperparameters::default();
    let values = found.unwrap_or(&fallback);
    OpenAiHyperparameters {
        learning_rate_multiplier: values.learning_rate_multiplier,
        // LiteLLM pops epoch_count with a default of 0, and a null value stays null
        n_epochs: values.epoch_count.or_if_absent(0),
        adapter_size: (!matches!(values.adapter_size, Nullable::Absent))
            .then(|| values.adapter_size.clone()),
    }
}
