use std::collections::{BTreeMap, BTreeSet};

use crate::complexity_scoring::tier::{ComplexityTier, tier_severity_order};
use crate::tier_definitions::error::TierDefinitionsError;
use crate::tier_definitions::model::label_for;

// tier_labels keys name a built-in tier; a label itself is display text and never a config key
pub(super) fn resolve_labels(
    configured: &BTreeMap<String, String>,
) -> Result<BTreeMap<ComplexityTier, String>, TierDefinitionsError> {
    let mut labels = BTreeMap::new();
    for (tier, label) in configured {
        let Some(known) = ComplexityTier::built_in(tier) else {
            return Err(TierDefinitionsError::UnknownLabelTier { tier: tier.clone() });
        };
        labels.insert(known, label.clone());
    }
    Ok(labels)
}

pub(super) fn validate_labels(
    labels: &BTreeMap<ComplexityTier, String>,
    non_reasoning: bool,
) -> Result<(), TierDefinitionsError> {
    if labels.is_empty() {
        return Ok(());
    }
    let blank = blank_labels(labels);
    if !blank.is_empty() {
        return Err(TierDefinitionsError::BlankLabels { tiers: blank });
    }
    let shadowed = shadowed_labels(labels);
    if !shadowed.is_empty() {
        return Err(TierDefinitionsError::ShadowedLabels { labels: shadowed });
    }
    let groups = duplicate_labels(labels, non_reasoning);
    if groups.is_empty() {
        return Ok(());
    }
    Err(TierDefinitionsError::DuplicateLabels { groups })
}

fn blank_labels(labels: &BTreeMap<ComplexityTier, String>) -> Vec<ComplexityTier> {
    labels
        .iter()
        .filter(|(_, label)| label.trim().is_empty())
        .map(|(tier, _)| *tier)
        .collect()
}

fn shadowed_labels(labels: &BTreeMap<ComplexityTier, String>) -> Vec<(ComplexityTier, String)> {
    let mut shadowed: Vec<(ComplexityTier, String)> = Vec::new();
    for (tier, label) in labels {
        let trimmed = label.trim();
        let Some(named) = ComplexityTier::built_in(&trimmed.to_uppercase()) else {
            continue;
        };
        if named != *tier {
            shadowed.push((*tier, trimmed.to_owned()));
        }
    }
    shadowed
}

fn duplicate_labels(
    labels: &BTreeMap<ComplexityTier, String>,
    non_reasoning: bool,
) -> Vec<Vec<ComplexityTier>> {
    let labeled: Vec<(ComplexityTier, String)> = tier_severity_order(non_reasoning)
        .iter()
        .map(|tier| (*tier, label_for(labels, *tier).to_lowercase()))
        .collect();
    let mut groups = Vec::new();
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for (_, folded) in &labeled {
        let shared = labeled.iter().filter(|(_, other)| other == folded).count() > 1;
        if shared && seen.insert(folded.as_str()) {
            groups.push(
                labeled
                    .iter()
                    .filter(|(_, other)| other == folded)
                    .map(|(tier, _)| *tier)
                    .collect(),
            );
        }
    }
    groups
}
