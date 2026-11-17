use betterlite_config::sections::adaptive_classification as section;
use betterlite_router::adaptive_classification::config::{
    AdaptiveRouterConfig, AdaptiveRouterPreferences, AdaptiveRouterWeights,
};
use betterlite_router::adaptive_classification::error::AdaptiveClassificationError;
use betterlite_router::adaptive_selection::AdaptiveSelection;
use betterlite_router::adaptive_selection::error::AdaptiveSelectionError;
use betterlite_router::bandit_math::error::BanditMathError;
use betterlite_router::bandit_math::sampler::{QualitySampler, ScriptedSampler};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use self::fields::fields_for;
use self::wire::{Fields, Payload};
use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

mod fields;
mod wire;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Input(Box<Payload>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Output(Box<Fields>);

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let payload = &input.0;
    let fields = fields_for(payload)?;
    Ok(ProbeOutput::AdaptiveSelection(Output(Box::new(fields))))
}

fn number(text: &str, field: &str) -> Result<f64, OperationError> {
    text.parse::<f64>()
        .map_err(|_error| OperationError::InvalidRequest {
            message: format!("{field} is not a number: {text}"),
        })
}

fn decimal(text: &str, field: &str) -> Result<Decimal, OperationError> {
    text.parse::<Decimal>()
        .map_err(|_error| OperationError::InvalidRequest {
            message: format!("{field} is not a decimal: {text}"),
        })
}

impl From<BanditMathError> for OperationError {
    fn from(error: BanditMathError) -> Self {
        let message = error.to_string();
        match error {
            BanditMathError::SamplesExhausted => Self::SamplesExhausted { message },
            BanditMathError::NonFinite { .. }
            | BanditMathError::UnsupportedTier { .. }
            | BanditMathError::NoModels
            | BanditMathError::Distribution { .. } => Self::InvalidRequest { message },
        }
    }
}

impl From<AdaptiveSelectionError> for OperationError {
    fn from(error: AdaptiveSelectionError) -> Self {
        let message = error.to_string();
        match error {
            AdaptiveSelectionError::Bandit(inner) => Self::from(inner),
            AdaptiveSelectionError::Classification(_)
            | AdaptiveSelectionError::CostNormalization
            | AdaptiveSelectionError::NoEligibleModels { .. }
            | AdaptiveSelectionError::MissingCell { .. } => Self::InvalidRequest { message },
        }
    }
}

fn weights(payload: &Payload) -> Result<AdaptiveRouterWeights, OperationError> {
    let Some(raw) = payload.weights.as_ref() else {
        return Ok(AdaptiveRouterWeights::resolve(None)?);
    };
    let input = section::Weights {
        quality: raw
            .quality
            .as_deref()
            .map(|text| number(text, "weights.quality"))
            .transpose()?,
        economy: raw
            .cost
            .as_deref()
            .map(|text| number(text, "weights.cost"))
            .transpose()?,
    };
    Ok(AdaptiveRouterWeights::resolve(Some(&input))?)
}

impl From<AdaptiveClassificationError> for OperationError {
    fn from(error: AdaptiveClassificationError) -> Self {
        Self::InvalidRequest {
            message: error.to_string(),
        }
    }
}

fn preference_map(payload: &Payload) -> IndexMap<String, AdaptiveRouterPreferences> {
    let mut preferences = IndexMap::with_capacity(payload.preferences.len());
    for preference in &payload.preferences {
        preferences.insert(
            preference.model.clone(),
            AdaptiveRouterPreferences {
                quality_tier: preference.quality_tier,
                strengths: preference.strengths.clone().unwrap_or_default(),
            },
        );
    }
    preferences
}

fn cost_map(payload: &Payload) -> Result<IndexMap<String, Decimal>, OperationError> {
    let mut costs = IndexMap::with_capacity(payload.costs.len());
    for cost in &payload.costs {
        costs.insert(cost.model.clone(), decimal(&cost.amount, "cost")?);
    }
    Ok(costs)
}

fn sample_values(payload: &Payload) -> Result<Vec<f64>, OperationError> {
    payload
        .samples
        .iter()
        .map(|text| number(text, "sample"))
        .collect()
}

fn service(payload: &Payload) -> Result<AdaptiveSelection, OperationError> {
    let preferences = preference_map(payload);
    let costs = cost_map(payload)?;
    Ok(AdaptiveSelection::new(
        "probe".to_owned(),
        &AdaptiveRouterConfig {
            available_models: payload.available_models.clone(),
            weights: weights(payload)?,
        },
        &preferences,
        &costs,
        QualitySampler::Scripted(Box::new(ScriptedSampler {
            samples: sample_values(payload)?,
            position: 0,
        })),
    )?)
}
