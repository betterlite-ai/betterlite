use crate::custom_dimensions::DimensionsError;
use crate::tier_definitions::checks::ClassifierKind;
use crate::tier_definitions::error::TierDefinitionsError;

#[derive(Debug, thiserror::Error)]
pub enum ComplexityConfigError {
    #[error(transparent)]
    Tiers(#[from] TierDefinitionsError),
    #[error(transparent)]
    Dimensions(#[from] DimensionsError),
    #[error("plan_mode_min_tier {tier:?} is not an active tier: it must name one of {names}")]
    PlanModeUnknownTier { tier: String, names: String },
    #[error(
        "plan_mode_min_tier {tier} has no model configured in tiers; a floor pointing at an \
         unconfigured tier would route every plan-mode request to the default fallback instead \
         of the premium pool the operator intended"
    )]
    PlanModeWithoutModel { tier: String },
    #[error(
        "heuristic_first_max_tier is set but classifier_type is {}; the local scorer would never \
         gate the classifier",
        .classifier.name()
    )]
    HeuristicFirstUnused { classifier: ClassifierKind },
    #[error(
        "heuristic_first_max_tier is required when classifier_type is 'heuristic_first': without \
         a threshold there is nothing to decide whether a request escalates to the LLM classifier"
    )]
    HeuristicFirstRequired,
    #[error("heuristic_first_max_tier {tier:?} is not an active tier: it must name one of {names}")]
    HeuristicFirstUnknownTier { tier: String, names: String },
    #[error(
        "heuristic_first_max_tier {tier} is the highest tier, so every request would \
         short-circuit and the LLM classifier would never run; name a lower tier or use \
         classifier_type 'heuristic'"
    )]
    HeuristicFirstHighest { tier: String },
    #[error(
        "heuristic_first_max_tier {tier} has no model configured in tiers; a threshold pointing \
         at an unconfigured tier would route short-circuited requests to the default fallback"
    )]
    HeuristicFirstWithoutModel { tier: String },
    #[error(
        "hybrid_boundary_margin is set but classifier_type is {}; the scorer would never consult \
         the classifier on a near-boundary score",
        .classifier.name()
    )]
    HybridMarginUnused { classifier: ClassifierKind },
    #[error(
        "hybrid_boundary_margin is required when classifier_type is 'hybrid': without a margin no \
         score is ever near enough to a boundary to escalate, which is classifier_type 'heuristic'"
    )]
    HybridMarginRequired,
    #[error("hybrid_boundary_margin {value} must be between 0 and 1")]
    HybridMarginOutOfRange { value: f64 },
    #[error("{field} must be non-empty; omit the field instead")]
    BlankText { field: &'static str },
    #[error("{field} must not exceed {max} characters")]
    TextTooLong { field: &'static str, max: usize },
    #[error("keyword_tier_rules reference unknown tiers: {tiers:?}; valid tiers: {valid}")]
    KeywordRuleUnknownTiers { tiers: Vec<String>, valid: String },
    #[error("adaptive=True requires at least one non-empty tier pool")]
    AdaptiveWithoutPools,
    #[error("adaptive=True tier pools must be non-empty; empty tiers: {tiers:?}")]
    AdaptiveEmptyPools { tiers: Vec<String> },
    #[error("embedding_model is required when semantic_keyword_matching is enabled")]
    SemanticWithoutEmbeddingModel,
    #[error("keyword_tier_rules must be non-empty when semantic_keyword_matching is enabled")]
    SemanticWithoutKeywordRules,
    #[error(
        "plugins and adaptive=True cannot both be set: adaptive's bandit selection doesn't yet \
         consume plugin-narrowed candidate pools. Disable adaptive or remove plugins."
    )]
    PluginsWithAdaptive,
    #[error(
        "stall_escalation_enabled cannot be combined with session_affinity or \
         classification_mode='user_turn': both replay a held routing decision on most turns \
         instead of classifying, so stall detection would never see the tool calls of the turns \
         it needs to look at"
    )]
    StallWithHeldDecision,
    #[error(
        "stall_escalation_repeat_threshold ({threshold}) cannot exceed stall_escalation_window \
         ({window}); the condition could never be reached"
    )]
    StallThresholdAboveWindow { threshold: u32, window: u32 },
    #[error(
        "tier entries carry complexity_router_config settings in their litellm_params, where the \
         router never reads them and the outbound request forwards them to the provider as \
         unknown body fields: {placements:?}. Set these on complexity_router_config itself"
    )]
    MisplacedTierParams { placements: Vec<String> },
}
