use std::str::FromStr;

use betterlite_openai::fine_tuning::job::FineTuneCreate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::TuningRecordError;

// LiteLLM reads adapter_size from the caller's original hyperparameters dict, not the coerced model
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct OriginalHyperparameters {
    pub adapter_size: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct VertexHyperparameters {
    #[serde(rename = "epoch_count", skip_serializing_if = "Option::is_none")]
    pub epoch_count: Option<i64>,
    #[serde(
        rename = "learning_rate_multiplier",
        skip_serializing_if = "Option::is_none",
        with = "rust_decimal::serde::arbitrary_precision_option"
    )]
    pub learning_rate_multiplier: Option<Decimal>,
    #[serde(rename = "adapter_size", skip_serializing_if = "Option::is_none")]
    pub adapter_size: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct SupervisedTuningSpec {
    #[serde(rename = "training_dataset_uri")]
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
        let describe = |value| {
            if value.is_null() {
                (String::new(), false)
            } else {
                (
                    value
                        .as_str()
                        .map_or_else(|| value.to_string(), str::to_owned),
                    value.as_str().is_some(),
                )
            }
        };
        hyperparameters.epoch_count =
            hyperparameter(raw.get("n_epochs").map(describe), "n_epochs")?;
        hyperparameters.learning_rate_multiplier = hyperparameter(
            raw.get("learning_rate_multiplier").map(describe),
            "learning_rate_multiplier",
        )?;
    }
    hyperparameters.adapter_size = original
        .adapter_size
        .clone()
        .filter(|text| !text.is_empty());
    Ok(vertex_job(create, hyperparameters))
}

fn hyperparameter<T>(
    value: Option<(String, bool)>,
    field: &'static str,
) -> Result<Option<T>, TuningRecordError>
where
    T: FromStr + Default + PartialEq,
{
    let Some((text, from_text)) = value else {
        return Ok(None);
    };
    if text.is_empty() {
        return Ok(None);
    }
    let parsed = text
        .trim()
        .parse::<T>()
        .map_err(|_error| TuningRecordError::Hyperparameter { field })?;
    Ok((from_text || parsed != T::default()).then_some(parsed))
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
