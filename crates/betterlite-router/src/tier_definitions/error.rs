use std::fmt;

use crate::complexity_scoring::tier::ComplexityTier;
use crate::tier_definitions::checks::ClassifierKind;

#[derive(Debug, thiserror::Error)]
pub enum TierDefinitionsError {
    #[error("tier_definitions entries must have a non-empty name")]
    EmptyName,
    #[error("tier_definitions name {name} exceeds {max} characters")]
    NameTooLong { name: String, max: usize },
    #[error("tier_definitions description for {name} exceeds {max} characters")]
    DescriptionTooLong { name: String, max: usize },
    #[error(
        "tier_definitions entry {name} must have a description: only a built-in tier carries one \
         the rubric can inherit"
    )]
    MissingDescription { name: String },
    #[error("tier_definitions entry {name} contains a newline")]
    Newline { name: String },
    #[error("tier_definitions must define between 2 and 8 tiers, got {count}")]
    TierCount { count: usize },
    #[error("tier_definitions names must be unique (case-insensitive): {names:?}")]
    DuplicateNames { names: Vec<String> },
    #[error("tier_definitions requires an LLM, Jev or custom classifier, got {classifier}")]
    ClassifierCannotEmitTiers { classifier: ClassifierKind },
    #[error("{}", join_conflicts(.conflicts))]
    Conflicts { conflicts: Vec<DefinitionConflict> },
    #[error("tiers must map every defined tier to a model; missing: {names:?}")]
    MissingTiers { names: Vec<String> },
    #[error("tiers keys must be defined in tier_definitions; unknown: {names:?}")]
    UnknownTiers { names: Vec<String> },
    #[error("tiers must map every defined tier to at least one model; empty: {names:?}")]
    EmptyPools { names: Vec<String> },
    #[error("fallback_tier is required with tier_definitions")]
    FallbackRequired,
    #[error("fallback_tier {tier} is not one of the defined tiers: {defined:?}")]
    FallbackUnknown { tier: String, defined: Vec<String> },
    #[error("fallback_tier requires tier_definitions")]
    FallbackWithoutTiers,
    #[error("{field} cannot be combined with classifier_llm_config.system_prompt")]
    OpeningWithSystemPrompt { field: &'static str },
    #[error("{field} requires an LLM classifier, got {classifier}")]
    OpeningRequiresClassifier {
        field: &'static str,
        classifier: ClassifierKind,
    },
    #[error("tiers names {tier} but non_reasoning_tier is false")]
    NonReasoningWithoutFlag { tier: &'static str },
    #[error("non_reasoning_tier cannot be combined with tier_definitions")]
    NonReasoningWithTiers,
    #[error("non_reasoning_tier requires an LLM, Jev or custom classifier, got {classifier}")]
    NonReasoningClassifier { classifier: ClassifierKind },
    #[error("non_reasoning_tier requires tiers to map {tier} to at least one model")]
    NonReasoningWithoutModel { tier: &'static str },
    #[error("tier_labels values must be non-empty; blank labels for tiers: {tiers:?}")]
    BlankLabels { tiers: Vec<ComplexityTier> },
    #[error("tier_labels values must not reuse another tier's canonical name: {labels:?}")]
    ShadowedLabels {
        labels: Vec<(ComplexityTier, String)>,
    },
    #[error("tier_labels values must be unique across tiers; shared labels for: {groups:?}")]
    DuplicateLabels { groups: Vec<Vec<ComplexityTier>> },
    #[error("tier_labels names {tier}, which is not a built-in tier")]
    UnknownLabelTier { tier: String },
    #[error("tier {tier} contains duplicate model_name values")]
    DuplicateModelName { tier: String },
    #[error("the pinned tier defaults are unreadable: {source}")]
    Defaults {
        #[source]
        source: serde_json::Error,
    },
}

// one reason a custom tier set cannot coexist with a built-in-ladder setting, joined into one error
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DefinitionConflict {
    OrderFeatures(Vec<&'static str>),
    SystemPrompt,
    ClassificationRubric,
    DefaultModelFallback,
    TierLabels,
}

impl fmt::Display for DefinitionConflict {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OrderFeatures(features) => write!(
                formatter,
                "{} cannot be combined with tier_definitions: these features rely on the built-in \
                 tier severity order",
                features.join(", ")
            ),
            Self::SystemPrompt => formatter.write_str(
                "classifier_llm_config.system_prompt cannot be combined with tier_definitions",
            ),
            Self::ClassificationRubric => formatter.write_str(
                "classifier_llm_config.classification_rubric cannot be combined with \
                 tier_definitions",
            ),
            Self::DefaultModelFallback => formatter.write_str(
                "classifier_fallback 'default_model' cannot be combined with tier_definitions",
            ),
            Self::TierLabels => {
                formatter.write_str("tier_labels cannot be combined with tier_definitions")
            }
        }
    }
}

fn join_conflicts(conflicts: &[DefinitionConflict]) -> String {
    conflicts
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}
