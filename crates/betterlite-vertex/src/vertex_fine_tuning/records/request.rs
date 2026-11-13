use std::str::FromStr;

use betterlite_openai::fine_tuning::job::FineTuneCreate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::error::TuningRecordError;

// LiteLLM reads adapter_size from the caller's original hyperparameters dict, not the coerced model
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct OriginalHyperparameters {
    pub adapter_size: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub enum AdapterSize {
    #[serde(rename = "ADAPTER_SIZE_UNSPECIFIED")]
    Unspecified,
    #[serde(rename = "ADAPTER_SIZE_ONE")]
    One,
    #[serde(rename = "ADAPTER_SIZE_FOUR")]
    Four,
    #[serde(rename = "ADAPTER_SIZE_EIGHT")]
    Eight,
    #[serde(rename = "ADAPTER_SIZE_SIXTEEN")]
    Sixteen,
}

impl FromStr for AdapterSize {
    type Err = TuningRecordError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "ADAPTER_SIZE_UNSPECIFIED" => Ok(Self::Unspecified),
            "ADAPTER_SIZE_ONE" => Ok(Self::One),
            "ADAPTER_SIZE_FOUR" => Ok(Self::Four),
            "ADAPTER_SIZE_EIGHT" => Ok(Self::Eight),
            "ADAPTER_SIZE_SIXTEEN" => Ok(Self::Sixteen),
            _ => Err(TuningRecordError::Hyperparameter {
                field: "adapter_size",
            }),
        }
    }
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
    pub adapter_size: Option<AdapterSize>,
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

// the value's JSON text and whether it arrived as text, since LiteLLM tests truthiness before int()/float()
type HyperparameterText = Option<(String, bool)>;

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
        hyperparameters.epoch_count = epoch_count(raw.get("n_epochs").map(describe))?;
        hyperparameters.learning_rate_multiplier =
            learning_rate_multiplier(raw.get("learning_rate_multiplier").map(describe))?;
    }
    hyperparameters.adapter_size = match original.adapter_size.as_deref() {
        None | Some("") => None,
        Some(text) => Some(AdapterSize::from_str(text)?),
    };
    Ok(vertex_job(create, hyperparameters))
}

fn epoch_count(value: HyperparameterText) -> Result<Option<i64>, TuningRecordError> {
    let Some((text, from_text)) = value else {
        return Ok(None);
    };
    if text.is_empty() {
        return Ok(None);
    }
    let count = text
        .trim()
        .parse::<i64>()
        .map_err(|_error| TuningRecordError::Hyperparameter { field: "n_epochs" })?;
    Ok((from_text || count != 0).then_some(count))
}

fn learning_rate_multiplier(
    value: HyperparameterText,
) -> Result<Option<Decimal>, TuningRecordError> {
    let Some((text, from_text)) = value else {
        return Ok(None);
    };
    if text.is_empty() {
        return Ok(None);
    }
    let rate =
        Decimal::from_str(text.trim()).map_err(|_error| TuningRecordError::Hyperparameter {
            field: "learning_rate_multiplier",
        })?;
    Ok((from_text || !rate.is_zero()).then_some(rate))
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
