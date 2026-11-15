from collections.abc import Mapping, Sequence
from typing import Literal

from pydantic import JsonValue
from typing_extensions import ReadOnly, TypedDict


# ast-grep-ignore: no-long-names
class StandardLoggingRoutingDecisionTierBoundaries(TypedDict):
    simple_medium: float
    medium_complex: float
    complex_reasoning: float


RoutingDecisionCause = Literal[
    "prompt_cache_cost",
    "heuristic_scorer",
    "heuristic_v2",
    "reasoning_override",
    "llm_classifier",
    "capability_classifier",
    "jev_classifier",
    "llm_v2_classifier",
    "llm_v2_fallback",
    "heuristic_first_short_circuit",
    "hybrid_short_circuit",
    "classifier_plugin",
    "classifier_fallback",
    "capability_classifier_fallback",
    "default_model_fallback",
    "literal_keyword_match",
    "semantic_keyword_match",
    "plan_mode",
    "housekeeping",
    "modality_escalation",
    "modality_pin_override",
    "health_failover",
    "health_default_fallback",
    "session_affinity_pin",
    "session_affinity_escalation",
    "user_turn_continuation",
    "default_fallback",
    "keyword",
    "quality_tier",
    "bandit",
]


# ast-grep-ignore: no-long-names
class StandardLoggingHeuristicV2Forecast(TypedDict):
    probabilities: ReadOnly[Mapping[str, float]]
    threshold: ReadOnly[float]
    predicted_tier: ReadOnly[str]
    request_type: ReadOnly[str]


class StandardLoggingRoutingDecision(TypedDict, total=False):
    router_model_name: str
    router_type: Literal["complexity", "adaptive", "quality"]
    routed_model: str
    cause: RoutingDecisionCause
    tier: str
    tier_label: str
    request_type: str
    score: float
    signals: Sequence[str]
    matched_keyword: str
    escalation_keyword: str
    classifier_model: str
    classifier_cost: float
    classifier_probabilities: ReadOnly[Mapping[str, float]]
    classifier_confidence: ReadOnly[float]
    # ast-grep-ignore: no-long-names
    heuristic_v2_forecast: ReadOnly[StandardLoggingHeuristicV2Forecast]
    classifier_crux: str
    classifier_primary_rule: str
    classifier_capability_boundary: str
    classifier_p_solve: float
    classifier_calibrated_p_solve: ReadOnly[float]
    classifier_calibration_version: ReadOnly[str]
    classifier_efficient_p_solve: ReadOnly[float]
    classifier_capable_p_solve: ReadOnly[float]
    # ast-grep-ignore: no-long-names
    classifier_calibrated_efficient_p_solve: ReadOnly[float]
    # ast-grep-ignore: no-long-names
    classifier_calibrated_capable_p_solve: ReadOnly[float]
    classifier_max_quality_gap: ReadOnly[float]
    classifier_prompt_version: ReadOnly[str]
    classifier_threshold: float
    escalated: bool
    context_escalated: bool
    # ast-grep-ignore: no-long-names
    context_escalation_original_tier: str
    # ast-grep-ignore: no-long-names
    tier_boundaries: StandardLoggingRoutingDecisionTierBoundaries
    reasoning_override_min_score: float
    conversation_continuing: bool
    savings_baseline_model: str
    savings_baseline_deployment_id: str
    tier_litellm_params: Mapping[str, JsonValue]
