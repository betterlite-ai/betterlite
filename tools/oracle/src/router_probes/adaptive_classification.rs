use betterlite_config::sections::adaptive_classification as section;
use betterlite_router::adaptive_classification::classifier::Classifier;
use betterlite_router::adaptive_classification::config::{self, DeploymentConfig};
use betterlite_router::adaptive_classification::minimum_quality::minimum_quality_tier;
use serde::{Deserialize, Serialize};

use self::wire::{
    Check, DeploymentInput, Fields, Payload, PreferencesInput, RequestInput, RouterConfigInput,
    WeightsInput,
};
use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

mod wire;

// the enum in mod.rs would otherwise carry every field of this payload inline in its largest variant
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
    let fields = match payload.check {
        Check::Classify => classify_fields(&payload.prompts)?,
        Check::MinimumQuality => minimum_quality_fields(&payload.requests)?,
        Check::RouterConfigs => selected_fields(payload, Kind::Routers)?,
        Check::Preferences => selected_fields(payload, Kind::Preferences)?,
    };
    Ok(ProbeOutput::AdaptiveClassification(Output(Box::new(
        fields,
    ))))
}

fn classifier() -> Result<Classifier, OperationError> {
    Classifier::new().map_err(|source| OperationError::InvalidRequest {
        message: source.to_string(),
    })
}

fn classify_fields(prompts: &[String]) -> Result<Fields, OperationError> {
    let classifier = classifier()?;
    let types = prompts
        .iter()
        .map(|prompt| classifier.classify_prompt(prompt))
        .collect();
    Ok(Fields {
        types: Some(types),
        tiers: None,
        routers: None,
        preferences: None,
        invalid: None,
    })
}

fn minimum_quality_fields(requests: &[RequestInput]) -> Result<Fields, OperationError> {
    let classifier = classifier()?;
    let mut types = Vec::with_capacity(requests.len());
    let mut tiers = Vec::with_capacity(requests.len());
    for request in requests {
        types.push(classifier.classify_prompt(&request.prompt));
        tiers.push(minimum_quality_tier(
            request.headers.as_ref(),
            request.metadata.as_ref(),
        ));
    }
    Ok(Fields {
        types: Some(types),
        tiers: Some(tiers),
        routers: None,
        preferences: None,
        invalid: None,
    })
}

#[derive(Clone, Copy)]
enum Kind {
    Routers,
    Preferences,
}

const fn invalid_fields() -> Fields {
    Fields {
        types: None,
        tiers: None,
        routers: None,
        preferences: None,
        invalid: Some(true),
    }
}

fn selected_fields(payload: &Payload, kind: Kind) -> Result<Fields, OperationError> {
    let Some(resolved) = resolve_settings(payload)? else {
        return Ok(invalid_fields());
    };
    let deployments = resolved.iter().flatten();
    match kind {
        Kind::Routers => {
            let routers = deployments.filter_map(router_output).collect();
            Ok(Fields {
                types: None,
                tiers: None,
                routers: Some(routers),
                preferences: None,
                invalid: None,
            })
        }
        Kind::Preferences => {
            let preferences = deployments
                .filter_map(|deployment| {
                    let configured = deployment.preferences.as_ref()?;
                    Some(wire::PreferenceOutput {
                        model_name: deployment.model_name.clone(),
                        quality_tier: i64::from(configured.quality_tier),
                        strengths: configured.strengths.clone(),
                    })
                })
                .collect();
            Ok(Fields {
                types: None,
                tiers: None,
                routers: None,
                preferences: Some(preferences),
                invalid: None,
            })
        }
    }
}

fn router_output(deployment: &DeploymentConfig) -> Option<wire::RouterOutput> {
    let config = deployment.config.as_ref()?;
    Some(wire::RouterOutput {
        model_name: deployment.model_name.clone(),
        available_models: config.available_models.clone(),
        quality: format!("{:.6}", config.weights.quality),
        cost: format!("{:.6}", config.weights.economy),
    })
}

fn resolve_settings(
    payload: &Payload,
) -> Result<Option<Vec<Option<DeploymentConfig>>>, OperationError> {
    let settings = section_settings(payload)?;
    let Ok(resolved) = config::resolve(&settings) else {
        return Ok(None);
    };
    Ok(Some(resolved))
}

fn section_settings(payload: &Payload) -> Result<section::Settings, OperationError> {
    let deployments = payload
        .deployments
        .iter()
        .map(deployment_settings)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(section::Settings { deployments })
}

fn deployment_settings(
    input: &DeploymentInput,
) -> Result<Option<section::Deployment>, OperationError> {
    if input.adaptive_router_config.is_none() && input.adaptive_router_preferences.is_none() {
        return Ok(None);
    }
    let config = input
        .adaptive_router_config
        .as_ref()
        .map(router_config)
        .transpose()?;
    let preferences = input
        .adaptive_router_preferences
        .as_ref()
        .map(preference_settings);
    Ok(Some(section::Deployment {
        model_name: input.model_name.clone(),
        config,
        preferences,
    }))
}

fn router_config(input: &RouterConfigInput) -> Result<section::RouterConfig, OperationError> {
    let weights = input.weights.as_ref().map(resolve_weights).transpose()?;
    Ok(section::RouterConfig {
        available_models: input.available_models.clone(),
        weights,
    })
}

fn resolve_weights(input: &WeightsInput) -> Result<section::Weights, OperationError> {
    let quality = input
        .quality
        .as_deref()
        .map(|value| number(value, "weights.quality"))
        .transpose()?;
    let economy = input
        .cost
        .as_deref()
        .map(|value| number(value, "weights.cost"))
        .transpose()?;
    Ok(section::Weights { quality, economy })
}

fn preference_settings(input: &PreferencesInput) -> section::Preferences {
    section::Preferences {
        quality_tier: input.quality_tier,
        strengths: input.strengths.clone().unwrap_or_default(),
    }
}

fn number(text: &str, field: &str) -> Result<f64, OperationError> {
    text.parse::<f64>()
        .map_err(|_error| OperationError::InvalidRequest {
            message: format!("{field} is not a number: {text}"),
        })
}
