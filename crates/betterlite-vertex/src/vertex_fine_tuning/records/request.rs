use betterlite_openai::fine_tuning::job::FineTuneCreate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::TuningRecordError;

#[derive(Debug, Clone, Deserialize)]
pub struct OriginalHyperparameters {
    pub adapter_size: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct VertexHyperparameters {
    #[serde(rename = "epoch_count", skip_serializing_if = "Option::is_none")]
    pub epoch_count: Option<i64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "rust_decimal::serde::arbitrary_precision_option"
    )]
    pub learning_rate_multiplier: Option<Decimal>,
    #[serde(rename = "adapter_size", skip_serializing_if = "Option::is_none")]
    pub adapter_size: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct SupervisedTuningSpec {
    pub training_dataset_uri: String,
    #[serde(rename = "validation_dataset", skip_serializing_if = "Option::is_none")]
    pub validation_dataset: Option<String>,
    #[serde(rename = "hyperParameters", skip_serializing_if = "Option::is_none")]
    pub hyperparameters: Option<VertexHyperparameters>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct VertexTuningJobCreate {
    #[serde(rename = "baseModel")]
    pub base_model: String,
    #[serde(rename = "supervisedTuningSpec")]
    pub supervised_tuning_spec: SupervisedTuningSpec,
    #[serde(rename = "tunedModelDisplayName")]
    pub tuned_model_display_name: Option<String>,
}

pub fn to_vertex_request(
    create: &FineTuneCreate,
    original: &OriginalHyperparameters,
) -> Result<VertexTuningJobCreate, TuningRecordError> {
    let mut hyperparameters = VertexHyperparameters::default();
    // LiteLLM tests each hyperparameter for truthiness: null, the number 0 and "" never reach Vertex
    if let Some(raw) = create.hyperparameters.as_ref() {
        let given = |key: &str| {
            raw.get(key)
                .filter(|value| !value.is_null())
                .map(|value| Given {
                    text: value
                        .as_str()
                        .map_or_else(|| value.to_string(), str::to_owned),
                    is_text: value.is_string(),
                })
        };
        hyperparameters.epoch_count =
            hyperparameter(given("n_epochs"), "n_epochs", |text| text.parse().ok())?;
        hyperparameters.learning_rate_multiplier = hyperparameter(
            given("learning_rate_multiplier"),
            "learning_rate_multiplier",
            // Python's float() reads 2e-05, which Decimal's plain parser refuses
            |text| {
                text.parse::<Decimal>()
                    .or_else(|_error| Decimal::from_scientific(text))
                    .ok()
            },
        )?;
    }
    hyperparameters.adapter_size = original
        .adapter_size
        .clone()
        .filter(|text| !text.is_empty());
    Ok(vertex_job(create, hyperparameters))
}

struct Given {
    text: String,
    is_text: bool,
}

fn hyperparameter<T>(
    candidate: Option<Given>,
    field: &'static str,
    parse: fn(&str) -> Option<T>,
) -> Result<Option<T>, TuningRecordError>
where
    T: Default + PartialEq,
{
    let Some(entry) = candidate.filter(|present| !present.text.is_empty()) else {
        return Ok(None);
    };
    let parsed = parse(entry.text.trim()).ok_or(TuningRecordError::Hyperparameter { field })?;
    Ok((entry.is_text || parsed != T::default()).then_some(parsed))
}

fn vertex_job(
    create: &FineTuneCreate,
    hyperparameters: VertexHyperparameters,
) -> VertexTuningJobCreate {
    let has_hyperparameters = hyperparameters.epoch_count.is_some()
        || hyperparameters.learning_rate_multiplier.is_some()
        || hyperparameters.adapter_size.is_some();
    VertexTuningJobCreate {
        base_model: create.model.clone(),
        supervised_tuning_spec: SupervisedTuningSpec {
            training_dataset_uri: create.training_file.clone(),
            // LiteLLM's `if validation_file:` treats an empty id as absent
            validation_dataset: create
                .validation_file
                .clone()
                .filter(|file| !file.is_empty()),
            hyperparameters: has_hyperparameters.then_some(hyperparameters),
        },
        tuned_model_display_name: create.suffix.clone(),
    }
}
