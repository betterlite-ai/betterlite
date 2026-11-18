#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::tier_definitions as section;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::tier_definitions::checks::{
    ClassifierFallback, ClassifierKind, OrderFeature, TierChecks,
};
use crate::tier_definitions::error::TierDefinitionsError;
use crate::tier_definitions::model::TierSet;

mod tier_definitions_behaviors;
mod tier_definitions_boundaries;

// the section view of a deployment that configures nothing, so a test names only the keys it sets
pub(crate) fn blank() -> section::Config {
    section::Config::default()
}

// every checks field, so a test names only the ones it varies
pub(crate) fn base_checks() -> TierChecks<'static> {
    TierChecks {
        classifier_type: ClassifierKind::Heuristic,
        classifier_fallback: ClassifierFallback::Heuristic,
        fallback_tier: None,
        classification_prompt: None,
        classification_examples: None,
        llm_system_prompt: None,
        classification_rubric: None,
        order_features: &[],
    }
}

// a definition list in the shape the config section resolves to
pub(crate) fn ladder(pairs: &[(&str, Option<&str>)]) -> Vec<section::TierDefinition> {
    pairs
        .iter()
        .map(|(name, description)| section::TierDefinition {
            name: (*name).to_owned(),
            description: description.map(str::to_owned),
        })
        .collect()
}

pub(crate) fn single(model: &str) -> section::TierPool {
    section::TierPool::Single(section::TierEntry::Model(model.to_owned()))
}

pub(crate) fn pool(models: &[&str]) -> section::TierPool {
    section::TierPool::Pool(
        models
            .iter()
            .map(|model| section::TierEntry::Model((*model).to_owned()))
            .collect(),
    )
}

pub(crate) fn detailed(model: &str, params: &[(&str, Json)]) -> section::TierEntry {
    section::TierEntry::Detailed(section::TierModelEntry {
        model_name: model.to_owned(),
        litellm_params: params
            .iter()
            .map(|(key, value)| ((*key).to_owned(), stored(value)))
            .collect(),
    })
}

fn stored(value: &Json) -> section::TierParam {
    serde_json::from_value(serde_json::to_value(value).expect("the parameter serializes"))
        .expect("the parameter deserializes")
}

// the exact parameter map a resolved entry carries, for a test that pins every value
pub(crate) fn params(pairs: &[(&str, Json)]) -> IndexMap<String, section::TierParam> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), stored(value)))
        .collect()
}

pub(crate) fn labels(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(tier, label)| ((*tier).to_owned(), (*label).to_owned()))
        .collect()
}

pub(crate) fn pools(pairs: &[(&str, section::TierPool)]) -> BTreeMap<String, section::TierPool> {
    pairs
        .iter()
        .map(|(tier, pool)| ((*tier).to_owned(), pool.clone()))
        .collect()
}

// a two-tier custom set that validates, for a test that varies one setting
pub(crate) fn custom() -> section::Config {
    section::Config {
        tier_definitions: Some(ladder(&[
            ("ECO", Some("cheap asks")),
            ("DEEP", Some("hard asks")),
        ])),
        tier_models: Some(pools(&[
            ("ECO", single("eco-mini")),
            ("DEEP", pool(&["deep-a", "deep-b"])),
        ])),
        ..blank()
    }
}

// a custom set with `count` defined tiers, each mapped to its own model
pub(crate) fn custom_with(count: usize) -> section::Config {
    let names: Vec<String> = (0..count)
        .map(|position| format!("TIER_{position}"))
        .collect();
    let definitions: Vec<section::TierDefinition> = names
        .iter()
        .map(|name| section::TierDefinition {
            name: name.clone(),
            description: Some("asks".to_owned()),
        })
        .collect();
    let mapped: BTreeMap<String, section::TierPool> = names
        .iter()
        .map(|name| (name.clone(), single(&format!("model-{name}"))))
        .collect();
    section::Config {
        tier_definitions: Some(definitions),
        tier_models: Some(mapped),
        ..blank()
    }
}

// the checks a custom tier set needs: an LLM classifier with a defined fallback
pub(crate) fn llm(fallback: &str) -> TierChecks<'_> {
    TierChecks {
        classifier_type: ClassifierKind::Llm,
        fallback_tier: Some(fallback),
        ..base_checks()
    }
}

pub(crate) fn checked(config: &section::Config, checks: &TierChecks<'_>) -> TierSet {
    TierSet::resolve(config, checks).expect("the test config resolves")
}

pub(crate) fn error(config: &section::Config, checks: &TierChecks<'_>) -> TierDefinitionsError {
    TierSet::resolve(config, checks).expect_err("the test config is rejected")
}

// the order features a checks value carries, in LiteLLM's rendering order
pub(crate) fn order_features(features: &[OrderFeature]) -> Vec<&'static str> {
    TierChecks {
        order_features: features,
        ..base_checks()
    }
    .order_feature_labels()
}
