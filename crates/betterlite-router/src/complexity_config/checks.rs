use std::collections::BTreeMap;

use betterlite_config::sections::complexity_config as section;
use betterlite_config::sections::custom_dimensions as dimensions_section;
use betterlite_config::sections::tier_definitions as tier_section;
use serde::{Deserialize, Serialize};

use crate::tier_definitions::checks::{ClassifierFallback, ClassifierKind};

pub const DEFAULT_STALL_ESCALATION_WINDOW: u32 = 6;
pub const DEFAULT_STALL_ESCALATION_REPEAT_THRESHOLD: u32 = 3;

// the router capabilities the cross-field rules read; a list-backed one is on when its list is non-empty
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Feature {
    Adaptive,
    Plugins,
    SessionAffinity,
    UserTurnClassification,
    StallEscalation,
    SemanticKeywordMatching,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StallWindow {
    pub window: u32,
    pub repeat_threshold: u32,
}

impl Default for StallWindow {
    fn default() -> Self {
        Self {
            window: DEFAULT_STALL_ESCALATION_WINDOW,
            repeat_threshold: DEFAULT_STALL_ESCALATION_REPEAT_THRESHOLD,
        }
    }
}

// the rest of the complexity router configuration these rules read; the keys themselves belong to other tasks
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Checks<'config> {
    pub classifier_type: ClassifierKind,
    pub classifier_fallback: ClassifierFallback,
    pub fallback_tier: Option<&'config str>,
    pub classification_prompt: Option<&'config str>,
    pub classification_examples: Option<&'config str>,
    pub llm_system_prompt: Option<&'config str>,
    pub classification_rubric: Option<&'config str>,
    pub keyword_rule_tiers: &'config [String],
    pub embedding_model: Option<&'config str>,
    pub stall_escalation: StallWindow,
    pub features: &'config [Feature],
}

// every view the cross-field rules compare: this task's six keys, the tier set, the custom dimensions and the rest
#[derive(Debug, Clone, PartialEq)]
pub struct Declared<'config> {
    pub settings: section::Config,
    pub tiers: tier_section::Config,
    pub dimensions: &'config [dimensions_section::Config],
    pub dimension_weights: &'config BTreeMap<String, f64>,
    pub checks: Checks<'config>,
}
