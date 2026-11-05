use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use serde_with::skip_serializing_none;

use super::error::FineTuneError;
use super::normalize::job_from_response;

type Extra = Map<String, Value>;

macro_rules! record {
    ($name:ident { $($field:ident: $kind:ty),* $(,)? }) => {
        #[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
        pub struct $name {
            $(pub $field: $kind,)*
            #[serde(flatten)]
            pub extra: Extra,
        }
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Auto {
    Auto,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum AutoOr<T> {
    Auto(Auto),
    Value(T),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub enum JobObject {
    #[serde(rename = "fine_tuning.job")]
    FineTuningJob,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    ValidatingFiles,
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MethodType {
    Supervised,
    Dpo,
    Reinforcement,
}

type Count = Option<AutoOr<i64>>;
type Rate = Option<AutoOr<f64>>;

record!(JobError { code: String, message: String, param: Option<String> });
record!(Hyperparameters {
    batch_size: Count,
    learning_rate_multiplier: Rate,
    n_epochs: Count
});
record!(DpoHyperparameters {
    batch_size: Count,
    beta: Rate,
    learning_rate_multiplier: Rate,
    n_epochs: Count,
});
record!(ReinforcementHyperparameters {
    batch_size: Count, compute_multiplier: Rate, eval_interval: Count, eval_samples: Count,
    learning_rate_multiplier: Rate, n_epochs: Count, reasoning_effort: Option<String>,
});
record!(SupervisedMethod { hyperparameters: Option<Hyperparameters> });
record!(DpoMethod { hyperparameters: Option<DpoHyperparameters> });
record!(ReinforcementMethod { grader: Value, hyperparameters: Option<ReinforcementHyperparameters> });
record!(Method {
    r#type: MethodType, dpo: Option<DpoMethod>, reinforcement: Option<ReinforcementMethod>,
    supervised: Option<SupervisedMethod>,
});
record!(Wandb { project: String, entity: Option<String>, name: Option<String>, tags: Option<Vec<String>> });
record!(WandbIntegration {
    r#type: String,
    wandb: Wandb
});
record!(FineTuneJob {
    id: String, created_at: i64, error: Option<JobError>, fine_tuned_model: Option<String>,
    finished_at: Option<i64>, hyperparameters: Hyperparameters, model: String, object: JobObject,
    organization_id: String, result_files: Vec<String>, seed: Option<i64>, status: JobStatus,
    trained_tokens: Option<i64>, training_file: String, validation_file: Option<String>,
    estimated_finish: Option<i64>, integrations: Option<Vec<WandbIntegration>>,
    metadata: Option<BTreeMap<String, String>>, method: Option<Method>,
});

#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Default, Deserialize, Serialize)]
pub struct FineTuneCreate {
    pub model: String,
    pub training_file: String,
    pub hyperparameters: Option<Extra>,
    pub integrations: Option<Vec<Extra>>,
    pub metadata: Option<BTreeMap<String, String>>,
    pub method: Option<Extra>,
    pub seed: Option<i64>,
    pub suffix: Option<String>,
    pub validation_file: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

pub fn decode_fine_tune_job(bytes: &[u8], is_azure: bool) -> Result<FineTuneJob, FineTuneError> {
    let record = serde_json::from_slice::<Extra>(bytes)
        .map_err(|source| FineTuneError::Decode { source })?;
    job_from_response(record, is_azure)
}
