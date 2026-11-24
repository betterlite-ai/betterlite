use std::collections::BTreeMap;

use betterlite_config::sections::complexity_scoring;
use betterlite_router::complexity_scoring::config::{Config, TierBoundaries};
use serde::{Deserialize, Serialize};

use super::error::RouterPresetsError;

#[derive(Debug, Deserialize, PartialEq, Serialize)]
pub struct ComplexityScorerDefaults {
    pub tier_boundaries: TierBoundaries,
    pub token_thresholds: TokenThresholds,
    pub dimension_weights: BTreeMap<String, f64>,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TokenThresholds {
    pub simple: i64,
    pub complex: i64,
}

impl ComplexityScorerDefaults {
    pub fn shipped() -> Result<Self, RouterPresetsError> {
        let config = Config::resolve(&complexity_scoring::Config::default())?;
        let token_thresholds = TokenThresholds {
            simple: config.token_thresholds.simple,
            complex: config.token_thresholds.complex,
        };
        Ok(Self {
            tier_boundaries: config.tier_boundaries,
            token_thresholds,
            dimension_weights: config.dimension_weights,
        })
    }
}
