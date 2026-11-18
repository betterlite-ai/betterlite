#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::complexity_scoring::tier::{ComplexityTier, tier_severity_order};
use crate::tier_definitions::model::{TierDefinition, TierSet};

use super::reply::parse_class_reply;

mod classifier_verdict_behaviors;
mod classifier_verdict_boundaries;

// the labels LiteLLM's default TIER_SEVERITY_ORDER_LABELED carries
pub(crate) fn canonical() -> Vec<(ComplexityTier, &'static str)> {
    tier_severity_order(false)
        .iter()
        .map(|tier| (*tier, tier.name()))
        .collect()
}

// a tier set with no custom definitions and no display labels: the pinned built-in ladder
pub(crate) fn blank_tiers() -> TierSet {
    TierSet {
        definitions: None,
        pools: BTreeMap::new(),
        labels: BTreeMap::new(),
        non_reasoning: false,
        fallback: None,
    }
}

pub(crate) fn tiers_with_labels(pairs: &[(ComplexityTier, &str)]) -> TierSet {
    let mut tiers = blank_tiers();
    tiers.labels = pairs
        .iter()
        .map(|(tier, label)| (*tier, (*label).to_owned()))
        .collect();
    tiers
}

pub(crate) fn tiers_with_definitions(
    definitions: &[(&str, Option<&str>)],
    non_reasoning: bool,
) -> TierSet {
    TierSet {
        definitions: Some(
            definitions
                .iter()
                .map(|(name, description)| TierDefinition {
                    name: (*name).to_owned(),
                    description: description.map(str::to_owned),
                })
                .collect(),
        ),
        pools: BTreeMap::new(),
        labels: BTreeMap::new(),
        non_reasoning,
        fallback: None,
    }
}

#[rstest]
fn replays_the_class_reply_corpus(#[files("fuzz/corpus/class_reply/*")] path: PathBuf) {
    let bytes = fs::read(&path).unwrap();
    let name = path.file_name().unwrap().to_str().unwrap().to_owned();
    let outcome = parse_class_reply(&bytes, &blank_tiers());
    if !name.contains("-valid") {
        assert!(outcome.is_err(), "{name} must not decode");
    } else {
        assert!(outcome.is_ok(), "{name} must decode");
    }
}
