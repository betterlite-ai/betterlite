use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ComplexityTier {
    NonReasoning,
    Simple,
    Medium,
    Complex,
    Reasoning,
}

impl ComplexityTier {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::NonReasoning => "NON_REASONING",
            Self::Simple => "SIMPLE",
            Self::Medium => "MEDIUM",
            Self::Complex => "COMPLEX",
            Self::Reasoning => "REASONING",
        }
    }

    // _built_in_tier_or_none: a name that spells a built-in tier is that tier
    #[must_use]
    pub fn built_in(name: &str) -> Option<Self> {
        NON_REASONING_ORDER
            .iter()
            .copied()
            .find(|tier| tier.name() == name)
    }
}

// a pipeline tier is either a built-in one or an operator-defined name from a custom tier set
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Tier {
    BuiltIn(ComplexityTier),
    Named(String),
}

impl Tier {
    #[must_use]
    pub fn from_name(name: &str) -> Self {
        ComplexityTier::built_in(name).map_or_else(|| Self::Named(name.to_owned()), Self::BuiltIn)
    }

    // _tier_name: the plain tier name, whether the pipeline carries a built-in tier or a defined name
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::BuiltIn(tier) => tier.name(),
            Self::Named(name) => name,
        }
    }
}

const NON_REASONING_ORDER: [ComplexityTier; 5] = [
    ComplexityTier::NonReasoning,
    ComplexityTier::Simple,
    ComplexityTier::Medium,
    ComplexityTier::Complex,
    ComplexityTier::Reasoning,
];

const SEVERITY_ORDER: [ComplexityTier; 4] = [
    ComplexityTier::Simple,
    ComplexityTier::Medium,
    ComplexityTier::Complex,
    ComplexityTier::Reasoning,
];

// tier_severity_order: the NON_REASONING tier leads only when the deployment enables it
#[must_use]
pub const fn tier_severity_order(non_reasoning_enabled: bool) -> &'static [ComplexityTier] {
    if non_reasoning_enabled {
        &NON_REASONING_ORDER
    } else {
        &SEVERITY_ORDER
    }
}
