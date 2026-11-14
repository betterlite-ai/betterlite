use std::str::FromStr as _;

use betterlite_config::sections::adaptive_classification as section;

use crate::adaptive_classification::constants::{DEFAULT_COST_WEIGHT, DEFAULT_QUALITY_WEIGHT};
use crate::adaptive_classification::error::AdaptiveClassificationError;
use crate::adaptive_classification::request_type::RequestType;
use crate::preference_math;

const MIN_QUALITY_TIER: u8 = 1;
const MAX_QUALITY_TIER: u8 = 3;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AdaptiveRouterWeights {
    pub quality: f64,
    // LiteLLM's "cost" weight is a dimensionless score weight, not a price
    pub economy: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AdaptiveRouterConfig {
    pub available_models: Vec<String>,
    pub weights: AdaptiveRouterWeights,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdaptiveRouterPreferences {
    pub quality_tier: u8,
    pub strengths: Vec<RequestType>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeploymentConfig {
    pub model_name: String,
    pub config: Option<AdaptiveRouterConfig>,
    pub preferences: Option<AdaptiveRouterPreferences>,
}

pub fn resolve(
    settings: &section::Settings,
) -> Result<Vec<Option<DeploymentConfig>>, AdaptiveClassificationError> {
    settings
        .deployments
        .iter()
        .map(|deployment| deployment.as_ref().map(resolve_deployment).transpose())
        .collect()
}

impl AdaptiveRouterWeights {
    pub fn resolve(input: Option<&section::Weights>) -> Result<Self, AdaptiveClassificationError> {
        let quality = input
            .and_then(|weights| weights.quality)
            .unwrap_or(DEFAULT_QUALITY_WEIGHT);
        if !(0.0_f64..=1.0_f64).contains(&quality) {
            return Err(AdaptiveClassificationError::QualityWeight { value: quality });
        }
        // LiteLLM sums the weights only when the request carries a cost weight
        let Some(economy) = input.and_then(|weights| weights.economy) else {
            return Ok(Self {
                quality,
                economy: DEFAULT_COST_WEIGHT,
            });
        };
        if !(0.0_f64..=1.0_f64).contains(&economy) {
            return Err(AdaptiveClassificationError::EconomyWeight { value: economy });
        }
        let total = preference_math::weights_total(quality, economy);
        if !total.within_tolerance {
            return Err(AdaptiveClassificationError::WeightsSum {
                quality,
                economy,
                sum: total.total,
            });
        }
        Ok(Self { quality, economy })
    }
}

impl AdaptiveRouterConfig {
    pub fn resolve(input: &section::RouterConfig) -> Result<Self, AdaptiveClassificationError> {
        Ok(Self {
            available_models: input.available_models.clone(),
            weights: AdaptiveRouterWeights::resolve(input.weights.as_ref())?,
        })
    }
}

impl AdaptiveRouterPreferences {
    pub fn resolve(input: &section::Preferences) -> Result<Self, AdaptiveClassificationError> {
        let quality_tier = u8::try_from(input.quality_tier)
            .ok()
            .filter(|tier| (MIN_QUALITY_TIER..=MAX_QUALITY_TIER).contains(tier))
            .ok_or(AdaptiveClassificationError::QualityTier {
                value: input.quality_tier,
            })?;
        let strengths = input
            .strengths
            .iter()
            .map(|name| {
                RequestType::from_str(name).map_err(|_error| {
                    AdaptiveClassificationError::UnknownStrength {
                        value: name.clone(),
                    }
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            quality_tier,
            strengths,
        })
    }
}

fn resolve_deployment(
    deployment: &section::Deployment,
) -> Result<DeploymentConfig, AdaptiveClassificationError> {
    let config = deployment
        .config
        .as_ref()
        .map(AdaptiveRouterConfig::resolve)
        .transpose()?;
    let preferences = deployment
        .preferences
        .as_ref()
        .map(AdaptiveRouterPreferences::resolve)
        .transpose()?;
    Ok(DeploymentConfig {
        model_name: deployment.model_name.clone(),
        config,
        preferences,
    })
}
