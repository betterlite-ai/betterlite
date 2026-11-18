use std::collections::BTreeMap;

use serde::Deserialize;

use crate::complexity_scoring::config::{TierBoundaries, TokenThresholds};

// the pinned LiteLLM keyword, weight and boundary data, kept as one reviewed fixture
pub(crate) const SOURCE: &str = include_str!("fixtures/defaults.json");

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Defaults {
    pub code_keywords: Vec<String>,
    pub reasoning_keywords: Vec<String>,
    pub technical_keywords: Vec<String>,
    pub simple_keywords: Vec<String>,
    pub dimension_weights: BTreeMap<String, f64>,
    pub tier_boundaries: TierBoundaries,
    pub token_thresholds: TokenThresholds,
    pub tier_models: BTreeMap<String, String>,
    pub multi_step_patterns: Vec<String>,
}
