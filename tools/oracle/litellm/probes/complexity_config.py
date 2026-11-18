from typing import Final, Literal, TypeAlias

from litellm.router_strategy.complexity_router.config import ComplexityRouterConfig
from pydantic import Field, JsonValue, ValidationError
from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false

_PROBE_CLASSIFIER_MODEL: Final[str] = "probe-classifier"
_PROBE_KEYWORD: Final[str] = "probe"
ClassifierType = Literal["heuristic", "llm", "heuristic_first", "hybrid"]
FeatureName = Literal[
    "adaptive",
    "session_affinity",
    "user_turn_classification",
    "stall_escalation",
    "semantic_keyword_matching",
]


class Settings(Strict):
    escalation_keywords: list[str] | None = None
    plan_mode_min_tier: str | None = None
    plan_mode_patterns: list[str] | None = None
    housekeeping_patterns: list[str] | None = None
    heuristic_first_max_tier: str | None = None
    hybrid_boundary_margin: float | None = None


class TierEntryInput(Strict):
    model_name: str
    litellm_params: dict[str, JsonValue] = Field(default_factory=dict)


TierItem: TypeAlias = str | TierEntryInput
TierPoolInput: TypeAlias = str | TierEntryInput | list[TierItem]


class TiersInput(Strict):
    tier_models: dict[str, TierPoolInput] | None = None


class StallWindowInput(Strict):
    window: int = 6
    repeat_threshold: int = 3


class Knobs(Strict):
    classifier_type: ClassifierType = "heuristic"
    classifier_fallback: Literal["heuristic", "default_model"] = "heuristic"
    fallback_tier: str | None = None
    classification_prompt: str | None = None
    classification_examples: str | None = None
    llm_system_prompt: str | None = None
    classification_rubric: str | None = None
    keyword_rule_tiers: list[str] = Field(default_factory=list)
    embedding_model: str | None = None
    stall_escalation: StallWindowInput = Field(default_factory=StallWindowInput)
    features: list[FeatureName] = Field(default_factory=list)


class Output(Strict):
    tier_names: list[str]
    escalation_keywords: list[str] | None
    plan_mode_min_tier: str | None
    plan_mode_patterns: list[str] | None
    housekeeping_patterns: list[str] | None
    heuristic_first_max_tier: str | None
    hybrid_boundary_margin: float | None
    classification_prompt: str | None
    classification_examples: str | None


class ComplexityConfig(Operation):
    settings: Settings = Field(default_factory=Settings)
    tiers: TiersInput = Field(default_factory=TiersInput)
    knobs: Knobs = Field(default_factory=Knobs)

    @override
    def run(self) -> Output:
        values: dict[str, object] = {}
        _settings_values(values, self.settings)
        _knob_values(values, self.knobs, self.tiers)
        try:
            config = ComplexityRouterConfig.model_validate(values)
        except ValidationError as error:
            raise ProbeError("invalid_request", str(error)) from error
        return Output(
            tier_names=list(config.tier_names()),
            escalation_keywords=config.escalation_keywords,
            plan_mode_min_tier=config.plan_mode_min_tier,
            plan_mode_patterns=_patterns(config.plan_mode_patterns),
            housekeeping_patterns=_patterns(config.housekeeping_patterns),
            heuristic_first_max_tier=config.heuristic_first_max_tier,
            hybrid_boundary_margin=config.hybrid_boundary_margin,
            classification_prompt=config.classification_prompt,
            classification_examples=config.classification_examples,
        )


def _patterns(value: tuple[str, ...] | None) -> list[str] | None:
    return None if value is None else list(value)


def _settings_values(values: dict[str, object], settings: Settings) -> None:
    if settings.escalation_keywords is not None:
        values["escalation_keywords"] = list(settings.escalation_keywords)
    if settings.plan_mode_min_tier is not None:
        values["plan_mode_min_tier"] = settings.plan_mode_min_tier
    if settings.plan_mode_patterns is not None:
        values["plan_mode_patterns"] = list(settings.plan_mode_patterns)
    if settings.housekeeping_patterns is not None:
        values["housekeeping_patterns"] = list(settings.housekeeping_patterns)
    if settings.heuristic_first_max_tier is not None:
        values["heuristic_first_max_tier"] = settings.heuristic_first_max_tier
    if settings.hybrid_boundary_margin is not None:
        values["hybrid_boundary_margin"] = settings.hybrid_boundary_margin


def _knob_values(values: dict[str, object], knobs: Knobs, tiers: TiersInput) -> None:
    values["classifier_type"] = knobs.classifier_type
    values["classifier_fallback"] = knobs.classifier_fallback
    values["stall_escalation_window"] = knobs.stall_escalation.window
    values["stall_escalation_repeat_threshold"] = knobs.stall_escalation.repeat_threshold
    _scalar_values(values, knobs)
    if knobs.keyword_rule_tiers:
        values["keyword_tier_rules"] = [
            {"keywords": [_PROBE_KEYWORD], "tier": tier} for tier in knobs.keyword_rule_tiers
        ]
    _llm_config_values(values, knobs)
    if tiers.tier_models is not None:
        values["tiers"] = {tier: _pool_value(pool) for tier, pool in tiers.tier_models.items()}
    values.update(_feature_values(knobs.features))


def _scalar_values(values: dict[str, object], knobs: Knobs) -> None:
    for name, value in (
        ("fallback_tier", knobs.fallback_tier),
        ("classification_prompt", knobs.classification_prompt),
        ("classification_examples", knobs.classification_examples),
        ("embedding_model", knobs.embedding_model),
    ):
        if value is not None:
            values[name] = value


def _llm_config_values(values: dict[str, object], knobs: Knobs) -> None:
    llm_config: dict[str, object] = {"model": _PROBE_CLASSIFIER_MODEL}
    if knobs.llm_system_prompt is not None:
        llm_config["system_prompt"] = knobs.llm_system_prompt
    if knobs.classification_rubric is not None:
        llm_config["classification_rubric"] = knobs.classification_rubric
    if knobs.classifier_type in ("llm", "heuristic_first", "hybrid") or len(llm_config) > 1:
        values["classifier_llm_config"] = llm_config


def _feature_values(features: list[FeatureName]) -> dict[str, object]:
    present = frozenset(features)
    values: dict[str, object] = {}
    if "adaptive" in present:
        values["adaptive"] = True
    if "session_affinity" in present:
        values["session_affinity"] = True
    if "user_turn_classification" in present:
        values["classification_mode"] = "user_turn"
    if "stall_escalation" in present:
        values["stall_escalation_enabled"] = True
    if "semantic_keyword_matching" in present:
        values["semantic_keyword_matching"] = True
    return values


def _pool_value(pool: TierPoolInput) -> object:
    if isinstance(pool, str):
        return pool
    if isinstance(pool, list):
        return [_item_value(item) for item in pool]
    return _item_value(pool)


def _item_value(item: TierItem) -> object:
    if isinstance(item, str):
        return item
    return {"model_name": item.model_name, "litellm_params": dict(item.litellm_params)}
