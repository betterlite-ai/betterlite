use std::collections::BTreeMap;
use std::slice::from_ref;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::complexity_scoring::tier::{ComplexityTier, Tier, tier_severity_order};

// MAX_TIER_DEFINITIONS: the largest custom ladder a router accepts
pub const MAX_TIER_DEFINITIONS: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TierDefinition {
    pub name: String,
    pub description: Option<String>,
}

// ComplexityTierModel: the model a pool entry names plus the request params that entry overrides
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TierModelEntry {
    pub model_name: String,
    pub params: IndexMap<String, Json>,
}

impl TierModelEntry {
    pub(crate) fn plain(model_name: String) -> Self {
        Self {
            model_name,
            params: IndexMap::new(),
        }
    }
}

// one tier's entries: a single pinned entry, or a pool a caller picks from
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TierPool {
    Single(TierModelEntry),
    Pool(Vec<TierModelEntry>),
}

impl TierPool {
    #[must_use]
    pub fn models(&self) -> &[TierModelEntry] {
        match self {
            Self::Single(entry) => from_ref(entry),
            Self::Pool(entries) => entries,
        }
    }

    // the emptiness LiteLLM's truthiness sees: an empty name for a single entry, no entries for a pool
    #[must_use]
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Single(entry) => entry.model_name.is_empty(),
            Self::Pool(entries) => entries.is_empty(),
        }
    }
}

// the validated tier configuration: the custom ladder when configured, the pools, and the labels
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TierSet {
    pub definitions: Option<Vec<TierDefinition>>,
    pub pools: BTreeMap<String, TierPool>,
    pub labels: BTreeMap<ComplexityTier, String>,
    pub non_reasoning: bool,
    pub fallback: Option<String>,
}

impl TierSet {
    // tier_names: the defined names, or the built-in names in severity order
    #[must_use]
    pub fn tier_names(&self) -> Vec<&str> {
        match &self.definitions {
            Some(definitions) => definitions
                .iter()
                .map(|definition| definition.name.as_str())
                .collect(),
            None => tier_severity_order(self.non_reasoning)
                .iter()
                .map(|tier| tier.name())
                .collect(),
        }
    }

    // classifier_wire_labels: the names the classifier is told to emit
    #[must_use]
    pub fn wire_labels(&self) -> Vec<&str> {
        let labeled: Vec<&str> = self
            .labeled_tiers()
            .into_iter()
            .map(|(_, label)| label)
            .collect();
        if self.definitions.is_some() {
            return self.tier_names();
        }
        labeled
    }

    // labeled_tiers: every built-in tier with its display name, in severity order
    #[must_use]
    pub fn labeled_tiers(&self) -> Vec<(ComplexityTier, &str)> {
        tier_severity_order(self.non_reasoning)
            .iter()
            .map(|tier| (*tier, label_for(&self.labels, *tier)))
            .collect()
    }

    // tier_for_label: a display name back to its tier, case-insensitively, then canonical names
    #[must_use]
    pub fn tier_for_label(&self, label: &str) -> Option<ComplexityTier> {
        let folded = label.trim().to_lowercase();
        let labeled = self.labeled_tiers();
        let by_label = labeled
            .iter()
            .find(|(_, name)| name.to_lowercase() == folded)
            .map(|(tier, _)| *tier);
        by_label.or_else(|| {
            labeled
                .iter()
                .find(|(tier, _)| tier.name().to_lowercase() == folded)
                .map(|(tier, _)| *tier)
        })
    }

    // resolve_classified_tier: the active tier a classifier reply names
    #[must_use]
    pub fn resolve_classified_tier(&self, label: &str) -> Option<Tier> {
        if self.definitions.is_none() {
            return self.tier_for_label(label).map(Tier::BuiltIn);
        }
        let folded = label.trim().to_lowercase();
        self.tier_names()
            .into_iter()
            .find(|name| name.to_lowercase() == folded)
            .map(|name| Tier::Named(name.to_owned()))
    }
}

pub(crate) fn label_for(labels: &BTreeMap<ComplexityTier, String>, tier: ComplexityTier) -> &str {
    match labels.get(&tier) {
        Some(label) if !label.trim().is_empty() => label.trim(),
        _ => tier.name(),
    }
}
