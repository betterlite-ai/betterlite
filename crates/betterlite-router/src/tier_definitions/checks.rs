use std::fmt;

use serde::{Deserialize, Serialize};

// ComplexityRouterConfig.classifier_type, the closed set of classifier kinds
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClassifierKind {
    #[default]
    Heuristic,
    HeuristicV2,
    Llm,
    Capability,
    LlmV2,
    Custom,
    HeuristicFirst,
    Hybrid,
    Jev,
}

impl ClassifierKind {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Heuristic => "heuristic",
            Self::HeuristicV2 => "heuristic_v2",
            Self::Llm => "llm",
            Self::Capability => "capability",
            Self::LlmV2 => "llm_v2",
            Self::Custom => "custom",
            Self::HeuristicFirst => "heuristic_first",
            Self::Hybrid => "hybrid",
            Self::Jev => "jev",
        }
    }

    // LLM_CLASSIFIER_TYPES: the kinds that call classifier_llm_config.model
    #[must_use]
    pub const fn uses_llm(self) -> bool {
        matches!(
            self,
            Self::Llm | Self::Capability | Self::LlmV2 | Self::HeuristicFirst | Self::Hybrid
        )
    }

    // the classifier kinds _validate_non_reasoning_tier accepts for NON_REASONING
    #[must_use]
    pub const fn emits_non_reasoning(self) -> bool {
        matches!(self, Self::Llm | Self::Custom | Self::Jev)
    }

    // the classifier kinds a custom tier set cannot run under
    #[must_use]
    pub const fn rejects_custom_tiers(self) -> bool {
        matches!(
            self,
            Self::Heuristic
                | Self::HeuristicV2
                | Self::Capability
                | Self::HeuristicFirst
                | Self::Hybrid
        )
    }
}

impl fmt::Display for ClassifierKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

// ComplexityRouterConfig.classifier_fallback
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClassifierFallback {
    #[default]
    Heuristic,
    DefaultModel,
}

// the settings _tier_definition_conflicts rejects next to a custom tier set because they depend on
// the built-in severity ladder
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderFeature {
    Adaptive,
    SessionAffinity,
    EscalationKeywords,
    StallEscalation,
    Plugins,
}

impl OrderFeature {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Adaptive => "adaptive",
            Self::SessionAffinity => "session_affinity",
            Self::EscalationKeywords => "escalation_keywords",
            Self::StallEscalation => "stall_escalation_enabled",
            Self::Plugins => "plugins",
        }
    }
}

// the order LiteLLM renders the order-dependent settings in
const ORDER_FEATURE_ORDER: [OrderFeature; 5] = [
    OrderFeature::Adaptive,
    OrderFeature::SessionAffinity,
    OrderFeature::EscalationKeywords,
    OrderFeature::StallEscalation,
    OrderFeature::Plugins,
];

// the rest of the router configuration these validations read, supplied by the caller that owns
// those keys; every field is the presence of a setting, never its value
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TierChecks<'a> {
    pub classifier_type: ClassifierKind,
    pub classifier_fallback: ClassifierFallback,
    pub fallback_tier: Option<&'a str>,
    pub classification_prompt: Option<&'a str>,
    pub classification_examples: Option<&'a str>,
    pub llm_system_prompt: Option<&'a str>,
    pub classification_rubric: Option<&'a str>,
    pub order_features: &'a [OrderFeature],
}

impl TierChecks<'_> {
    #[must_use]
    pub fn order_feature_labels(&self) -> Vec<&'static str> {
        ORDER_FEATURE_ORDER
            .iter()
            .filter(|feature| self.order_features.contains(*feature))
            .map(|feature| feature.label())
            .collect()
    }
}
