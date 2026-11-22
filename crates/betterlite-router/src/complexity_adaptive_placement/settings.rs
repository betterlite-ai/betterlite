use betterlite_config::sections::complexity_adaptive_placement as section;
use betterlite_config::sections::complexity_adaptive_placement::{
    DEFAULT_COST_WEIGHT, DEFAULT_QUALITY_WEIGHT, DEFAULT_TIER_DISTANCE_PENALTY,
};

use crate::adaptive_classification::config::AdaptiveRouterWeights;
use crate::adaptive_classification::error::AdaptiveClassificationError;

// the complexity router's adaptive_* configuration, with absent keys at their declared defaults
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlacementSettings {
    pub adaptive: bool,
    pub weights: AdaptiveRouterWeights,
    pub tier_distance_penalty: f64,
    pub eligible: section::AdaptiveEligible,
}

impl Default for PlacementSettings {
    fn default() -> Self {
        Self {
            adaptive: section::DEFAULT_ADAPTIVE,
            weights: AdaptiveRouterWeights {
                quality: DEFAULT_QUALITY_WEIGHT,
                economy: DEFAULT_COST_WEIGHT,
            },
            tier_distance_penalty: DEFAULT_TIER_DISTANCE_PENALTY,
            eligible: section::AdaptiveEligible::All,
        }
    }
}

impl PlacementSettings {
    // adaptive_weights defaults to (0.3, 0.7); a declared object keeps its field defaults and sum check
    pub fn resolve(config: &section::Config) -> Result<Self, AdaptiveClassificationError> {
        let weights = match config.weights.as_ref() {
            Some(weights) => AdaptiveRouterWeights::resolve(Some(weights))?,
            None => AdaptiveRouterWeights {
                quality: DEFAULT_QUALITY_WEIGHT,
                economy: DEFAULT_COST_WEIGHT,
            },
        };
        Ok(Self {
            adaptive: config.adaptive,
            weights,
            tier_distance_penalty: config.tier_distance_penalty,
            eligible: config.eligible,
        })
    }
}
