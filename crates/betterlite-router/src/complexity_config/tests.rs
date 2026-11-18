#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::complexity_config as section;
use betterlite_config::sections::custom_dimensions as dimensions_section;
use betterlite_config::sections::tier_definitions as tier_section;
use betterlite_models::decisions::Json;

use crate::complexity_config::checks::{Checks, Declared};
use crate::complexity_config::error::ComplexityConfigError;
use crate::complexity_config::{Resolved, resolve};

mod complexity_config_behaviors;
mod complexity_config_boundaries;

pub(crate) fn blank() -> section::Config {
    section::Config::default()
}

pub(crate) fn base_checks() -> Checks<'static> {
    Checks::default()
}

// the section view of a deployment that configures no tier set, so a test names only the keys it sets
pub(crate) fn tiers() -> tier_section::Config {
    tier_section::Config::default()
}

pub(crate) fn single(model: &str) -> tier_section::TierPool {
    tier_section::TierPool::Single(tier_section::TierEntry::Model(model.to_owned()))
}

fn stored(value: &Json) -> tier_section::TierParam {
    serde_json::from_value(serde_json::to_value(value).expect("the parameter serializes"))
        .expect("the parameter deserializes")
}

pub(crate) fn entry(model: &str, params: &[(&str, Json)]) -> tier_section::TierEntry {
    tier_section::TierEntry::Detailed(tier_section::TierModelEntry {
        model_name: model.to_owned(),
        litellm_params: params
            .iter()
            .map(|(key, value)| ((*key).to_owned(), stored(value)))
            .collect(),
    })
}

pub(crate) fn tier_pools(
    pairs: &[(&str, tier_section::TierPool)],
) -> BTreeMap<String, tier_section::TierPool> {
    pairs
        .iter()
        .map(|(tier, pool)| ((*tier).to_owned(), pool.clone()))
        .collect()
}

pub(crate) fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

// the checks a custom tier set needs: an LLM classifier with a defined fallback
pub(crate) fn llm(fallback: &'static str) -> Checks<'static> {
    Checks {
        classifier_type: crate::tier_definitions::checks::ClassifierKind::Llm,
        fallback_tier: Some(fallback),
        ..base_checks()
    }
}

// a two-tier custom set that validates, for a test that varies one setting
pub(crate) fn custom() -> tier_section::Config {
    tier_section::Config {
        tier_definitions: Some(vec![
            tier_section::TierDefinition {
                name: "ECO".to_owned(),
                description: Some("cheap asks".to_owned()),
            },
            tier_section::TierDefinition {
                name: "DEEP".to_owned(),
                description: Some("hard asks".to_owned()),
            },
        ]),
        tier_models: Some(tier_pools(&[
            ("ECO", single("eco-mini")),
            ("DEEP", single("deep")),
        ])),
        ..tiers()
    }
}

pub(crate) fn resolve_with(
    settings: section::Config,
    tiers: tier_section::Config,
    checks: &Checks<'_>,
) -> Result<Resolved, ComplexityConfigError> {
    let dimensions: [dimensions_section::Config; 0] = [];
    let weights = BTreeMap::new();
    resolve(&Declared {
        settings,
        tiers,
        dimensions: &dimensions,
        dimension_weights: &weights,
        checks: checks.clone(),
    })
}

pub(crate) fn checked(
    settings: section::Config,
    tiers: tier_section::Config,
    checks: &Checks<'_>,
) -> Resolved {
    resolve_with(settings, tiers, checks).expect("the test config resolves")
}

pub(crate) fn error(
    settings: section::Config,
    tiers: tier_section::Config,
    checks: &Checks<'_>,
) -> ComplexityConfigError {
    resolve_with(settings, tiers, checks).expect_err("the test config is rejected")
}
