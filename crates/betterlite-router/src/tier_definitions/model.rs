use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::complexity_scoring::tier::{ComplexityTier, Tier, tier_severity_order};

// MAX_TIER_DEFINITIONS: the largest custom ladder a router accepts
pub const MAX_TIER_DEFINITIONS: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TierDefinition {
    pub(crate) name: String,
    pub(crate) description: Option<String>,
}

impl TierDefinition {
    #[must_use]
    pub const fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn description(&self) -> Option<&str> {
        match &self.description {
            Some(description) => Some(description.as_str()),
            None => None,
        }
    }
}

// ComplexityTierModel: the model a pool entry names plus the request params that entry overrides
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TierModelEntry {
    pub(crate) model_name: String,
    pub(crate) params: IndexMap<String, Json>,
}

impl TierModelEntry {
    pub(crate) fn plain(model_name: String) -> Self {
        Self {
            model_name,
            params: IndexMap::new(),
        }
    }

    #[must_use]
    pub const fn model_name(&self) -> &str {
        &self.model_name
    }

    #[must_use]
    pub const fn params(&self) -> &IndexMap<String, Json> {
        &self.params
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
            Self::Single(entry) => std::slice::from_ref(entry),
            Self::Pool(entries) => entries,
        }
    }

    #[must_use]
    pub const fn is_single(&self) -> bool {
        matches!(self, Self::Single(_))
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
    pub(crate) definitions: Option<Vec<TierDefinition>>,
    pub(crate) pools: BTreeMap<String, TierPool>,
    pub(crate) labels: BTreeMap<ComplexityTier, String>,
    pub(crate) non_reasoning: bool,
    pub(crate) fallback: Option<String>,
}

impl TierSet {
    // has_custom_tiers: true when the operator replaced the built-in ladder
    #[must_use]
    pub const fn is_custom(&self) -> bool {
        self.definitions.is_some()
    }

    #[must_use]
    pub const fn non_reasoning_enabled(&self) -> bool {
        self.non_reasoning
    }

    // active_tier_severity_order: the built-in ladder, which a custom tier set does not define
    #[must_use]
    pub const fn severity_order(&self) -> &'static [ComplexityTier] {
        tier_severity_order(self.non_reasoning)
    }

    #[must_use]
    pub fn definitions(&self) -> Option<&[TierDefinition]> {
        self.definitions.as_deref()
    }

    // tier_names: the defined names, or the built-in names in severity order
    #[must_use]
    pub fn tier_names(&self) -> Vec<&str> {
        match &self.definitions {
            Some(definitions) => definitions.iter().map(TierDefinition::name).collect(),
            None => self
                .severity_order()
                .iter()
                .map(|tier| tier.name())
                .collect(),
        }
    }

    // classifier_wire_labels: the names the classifier is told to emit
    #[must_use]
    pub fn wire_labels(&self) -> Vec<&str> {
        if self.is_custom() {
            return self.tier_names();
        }
        self.labeled_tiers()
            .into_iter()
            .map(|(_, label)| label)
            .collect()
    }

    // labeled_tiers: every built-in tier with its display name, in severity order
    #[must_use]
    pub fn labeled_tiers(&self) -> Vec<(ComplexityTier, &str)> {
        self.severity_order()
            .iter()
            .map(|tier| (*tier, label_for(&self.labels, *tier)))
            .collect()
    }

    // tier_label: the operator's display name, else the canonical one
    #[must_use]
    pub fn label(&self, tier: ComplexityTier) -> &str {
        label_for(&self.labels, tier)
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
        if self.is_custom() {
            let folded = label.trim().to_lowercase();
            return self
                .tier_names()
                .into_iter()
                .find(|name| name.to_lowercase() == folded)
                .map(|name| Tier::Named(name.to_owned()));
        }
        self.tier_for_label(label).map(Tier::BuiltIn)
    }

    #[must_use]
    pub fn pool(&self, tier: &str) -> Option<&TierPool> {
        self.pools.get(tier)
    }

    #[must_use]
    pub fn pools(&self) -> &BTreeMap<String, TierPool> {
        &self.pools
    }

    #[must_use]
    pub fn fallback_tier(&self) -> Option<&str> {
        self.fallback.as_deref()
    }
}

pub(crate) fn label_for<'labels>(
    labels: &'labels BTreeMap<ComplexityTier, String>,
    tier: ComplexityTier,
) -> &'labels str {
    match labels.get(&tier) {
        Some(label) if !label.trim().is_empty() => label.trim(),
        _ => tier.name(),
    }
}
