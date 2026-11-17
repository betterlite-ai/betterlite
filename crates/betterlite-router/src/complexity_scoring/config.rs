use std::collections::BTreeMap;

use betterlite_config::sections::complexity_scoring as section;
use serde::{Deserialize, Serialize};

use crate::complexity_scoring::defaults::{self, Defaults};
use crate::complexity_scoring::dimensions::CustomDimension;
use crate::complexity_scoring::error::ScoringError;
use crate::complexity_scoring::tier::Tier;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TierBoundaries {
    pub simple_medium: f64,
    pub medium_complex: f64,
    pub complex_reasoning: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenThresholds {
    pub simple: i64,
    pub complex: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TierModelValue {
    Single(String),
    Pool(Vec<String>),
}

impl TierModelValue {
    fn from_section(value: &section::TierModelValue) -> Self {
        match value {
            section::TierModelValue::Single(model) => Self::Single(model.clone()),
            section::TierModelValue::Pool(models) => Self::Pool(models.clone()),
        }
    }

    // a pool with one model has the same single answer as a bare name
    fn model(&self) -> Option<&str> {
        match self {
            Self::Single(model) => Some(model),
            Self::Pool(models) => match models.as_slice() {
                [model] => Some(model),
                _ => None,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub tier_models: BTreeMap<String, TierModelValue>,
    pub token_thresholds: TokenThresholds,
    pub dimension_weights: BTreeMap<String, f64>,
    pub tier_boundaries: TierBoundaries,
    // the configured floor, or the SIMPLE/MEDIUM boundary when unset; an explicit zero stays zero
    pub reasoning_override_min_score: f64,
    pub code_keywords: Vec<String>,
    pub reasoning_keywords: Vec<String>,
    pub technical_keywords: Vec<String>,
    pub custom_technical_keywords: Vec<String>,
    pub simple_keywords: Vec<String>,
    pub default_model: Option<String>,
    pub multi_step_patterns: Vec<String>,
    pub custom_dimensions: Vec<CustomDimension>,
}

impl Config {
    pub fn resolve(input: &section::Config) -> Result<Self, ScoringError> {
        let defaults: Defaults = serde_json::from_str(defaults::SOURCE)?;
        let tier_boundaries = boundaries(input.tier_boundaries.as_ref(), defaults.tier_boundaries)?;
        let keywords = keyword_lists(input, &defaults);
        Ok(Self {
            tier_models: tier_models(input.tier_models.as_ref(), &defaults),
            token_thresholds: thresholds(
                input.complexity_thresholds.as_ref(),
                defaults.token_thresholds,
            ),
            dimension_weights: weights(
                input
                    .dimension_weights
                    .as_ref()
                    .unwrap_or(&defaults.dimension_weights),
            )?,
            tier_boundaries,
            reasoning_override_min_score: require_finite(
                input
                    .reasoning_override_min_score
                    .unwrap_or(tier_boundaries.simple_medium),
                "reasoning_override_min_score",
            )?,
            code_keywords: keywords.code,
            reasoning_keywords: keywords.reasoning,
            technical_keywords: keywords.technical,
            custom_technical_keywords: input.custom_technical_keywords.clone().unwrap_or_default(),
            simple_keywords: keywords.simple,
            default_model: input.default_model.clone(),
            multi_step_patterns: defaults.multi_step_patterns.clone(),
            custom_dimensions: Vec::new(),
        })
    }

    // get_model_for_tier for one model per tier: the tier's model, else the default model, else MEDIUM's; B334 owns a pool
    pub fn single_model_for_tier(&self, tier: &Tier) -> Result<&str, ScoringError> {
        if let Some(value) = self.tier_models.get(tier.name()) {
            return value.model().ok_or_else(|| ScoringError::TierPool {
                tier: tier.name().to_owned(),
            });
        }
        if let Some(default) = &self.default_model {
            return Ok(default.as_str());
        }
        let medium = self
            .tier_models
            .get("MEDIUM")
            .ok_or_else(|| ScoringError::NoTierModel {
                tier: tier.name().to_owned(),
            })?;
        medium.model().ok_or_else(|| ScoringError::TierPool {
            tier: "MEDIUM".to_owned(),
        })
    }
}

struct KeywordLists {
    code: Vec<String>,
    reasoning: Vec<String>,
    technical: Vec<String>,
    simple: Vec<String>,
}

// a configured keyword list replaces its pinned one, and an empty configured list is used as given
fn keyword_lists(input: &section::Config, defaults: &Defaults) -> KeywordLists {
    KeywordLists {
        code: input
            .code_keywords
            .as_deref()
            .unwrap_or(&defaults.code_keywords)
            .to_vec(),
        reasoning: input
            .reasoning_keywords
            .as_deref()
            .unwrap_or(&defaults.reasoning_keywords)
            .to_vec(),
        technical: input
            .technical_keywords
            .as_deref()
            .unwrap_or(&defaults.technical_keywords)
            .to_vec(),
        simple: input
            .simple_keywords
            .as_deref()
            .unwrap_or(&defaults.simple_keywords)
            .to_vec(),
    }
}

// a configured tier map replaces the pinned default map entirely
fn tier_models(
    configured: Option<&BTreeMap<String, section::TierModelValue>>,
    defaults: &Defaults,
) -> BTreeMap<String, TierModelValue> {
    configured.map_or_else(
        || {
            defaults
                .tier_models
                .iter()
                .map(|(tier, model)| (tier.clone(), TierModelValue::Single(model.clone())))
                .collect()
        },
        |values| {
            values
                .iter()
                .map(|(tier, value)| (tier.clone(), TierModelValue::from_section(value)))
                .collect()
        },
    )
}

fn thresholds(
    configured: Option<&BTreeMap<String, i64>>,
    default: TokenThresholds,
) -> TokenThresholds {
    TokenThresholds {
        simple: configured
            .and_then(|values| values.get("simple"))
            .copied()
            .unwrap_or(default.simple),
        complex: configured
            .and_then(|values| values.get("complex"))
            .copied()
            .unwrap_or(default.complex),
    }
}

fn boundaries(
    values: Option<&BTreeMap<String, f64>>,
    default: TierBoundaries,
) -> Result<TierBoundaries, ScoringError> {
    let simple_medium = values
        .and_then(|configured| configured.get("simple_medium"))
        .copied()
        .unwrap_or(default.simple_medium);
    let medium_complex = values
        .and_then(|configured| configured.get("medium_complex"))
        .copied()
        .unwrap_or(default.medium_complex);
    let complex_reasoning = values
        .and_then(|configured| configured.get("complex_reasoning"))
        .copied()
        .unwrap_or(default.complex_reasoning);
    Ok(TierBoundaries {
        simple_medium: require_finite(simple_medium, "tier_boundaries.simple_medium")?,
        medium_complex: require_finite(medium_complex, "tier_boundaries.medium_complex")?,
        complex_reasoning: require_finite(complex_reasoning, "tier_boundaries.complex_reasoning")?,
    })
}

fn weights(values: &BTreeMap<String, f64>) -> Result<BTreeMap<String, f64>, ScoringError> {
    let mut resolved = BTreeMap::new();
    for (name, weight) in values {
        resolved.insert(
            name.clone(),
            require_finite(*weight, &format!("dimension_weights.{name}"))?,
        );
    }
    Ok(resolved)
}

fn require_finite(value: f64, field: &str) -> Result<f64, ScoringError> {
    if value.is_finite() {
        return Ok(value);
    }
    Err(ScoringError::NonFinite {
        field: field.to_owned(),
    })
}
