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

// ComplexityRouterConfig.classifier_fallback
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClassifierFallback {
    #[default]
    Heuristic,
    DefaultModel,
}

// the settings _tier_definition_conflicts rejects next to a custom tier set
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderFeature {
    Adaptive,
    SessionAffinity,
    EscalationKeywords,
    StallEscalation,
    Plugins,
}

// the order LiteLLM renders the order-dependent settings in
const ORDER_FEATURE_ORDER: [OrderFeature; 5] = [
    OrderFeature::Adaptive,
    OrderFeature::SessionAffinity,
    OrderFeature::EscalationKeywords,
    OrderFeature::StallEscalation,
    OrderFeature::Plugins,
];

// the rest of the router configuration these validations read; every field is a setting's presence
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
        let mut labels = Vec::new();
        for feature in ORDER_FEATURE_ORDER {
            if !self.order_features.contains(&feature) {
                continue;
            }
            labels.push(match feature {
                OrderFeature::Adaptive => "adaptive",
                OrderFeature::SessionAffinity => "session_affinity",
                OrderFeature::EscalationKeywords => "escalation_keywords",
                OrderFeature::StallEscalation => "stall_escalation_enabled",
                OrderFeature::Plugins => "plugins",
            });
        }
        labels
    }
}
