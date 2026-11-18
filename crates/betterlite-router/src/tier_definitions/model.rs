use std::collections::BTreeMap;
use std::slice::from_ref;

use betterlite_config::sections::tier_definitions as section;
use indexmap::IndexMap;

use crate::complexity_scoring::tier::{ComplexityTier, Tier, tier_severity_order};

pub const MAX_TIER_DEFINITIONS: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TierDefinition {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TierModelEntry {
    pub model_name: String,
    pub params: IndexMap<String, section::TierParam>,
}

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TierSet {
    pub definitions: Option<Vec<TierDefinition>>,
    pub pools: BTreeMap<String, TierPool>,
    pub labels: BTreeMap<ComplexityTier, String>,
    pub non_reasoning: bool,
    pub fallback: Option<String>,
}

impl TierSet {
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

    #[must_use]
    pub fn labeled_tiers(&self) -> Vec<(ComplexityTier, &str)> {
        tier_severity_order(self.non_reasoning)
            .iter()
            .map(|tier| (*tier, label_for(&self.labels, *tier)))
            .collect()
    }

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
        Some(_) | None => tier.name(),
    }
}
