from collections.abc import Mapping
from typing import Final, Literal, TypeAlias

from litellm.router_strategy.complexity_router.config import (
    ComplexityRouterConfig,
    ComplexityTier,
)
from pydantic import Field, JsonValue, TypeAdapter, ValidationError
from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true

ClassifierType = Literal["heuristic", "llm"]
OrderFeatureName = Literal["adaptive", "session_affinity", "escalation_keywords", "stall_escalation"]

_PROBE_CLASSIFIER_MODEL: Final[str] = "probe-classifier"
_ESCALATION_KEYWORD: Final[str] = "escalate"
_JSON_OBJECT: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])
_EMPTY_PARAMS: Final[dict[str, JsonValue]] = {}


class TierDefinitionInput(Strict):
    name: str
    description: str | None = None


class TierEntryInput(Strict):
    model_name: str
    litellm_params: dict[str, JsonValue] = Field(default_factory=dict)


TierValue: TypeAlias = str | TierEntryInput | list[str | TierEntryInput]


class ConfigInput(Strict):
    tier_definitions: list[TierDefinitionInput] | None = None
    tier_models: dict[str, TierValue] | None = None
    tier_labels: dict[str, str] | None = None
    non_reasoning_tier: bool = False


class Knobs(Strict):
    classifier_type: ClassifierType = "heuristic"
    classifier_fallback: Literal["heuristic", "default_model"] = "heuristic"
    fallback_tier: str | None = None
    classification_prompt: str | None = None
    classification_examples: str | None = None
    llm_system_prompt: str | None = None
    classification_rubric: str | None = None
    order_features: list[OrderFeatureName] = Field(default_factory=list)


class LabeledTier(Strict):
    tier: str
    label: str


class EntryView(Strict):
    model_name: str
    params: dict[str, JsonValue]


class PoolView(Strict):
    single: bool
    entries: list[EntryView]


class Output(Strict):
    custom: bool
    non_reasoning: bool
    uses_llm_classifier: bool
    tier_names: list[str]
    wire_labels: list[str]
    severity_order: list[str]
    labeled_tiers: list[LabeledTier]
    fallback_tier: str | None
    pools: dict[str, PoolView]
    resolved_tiers: list[str | None]
    label_tiers: list[str | None]


class TierDefinitions(Operation):
    config: ConfigInput | None = None
    knobs: Knobs = Field(default_factory=Knobs)
    queries: list[str] = Field(default_factory=list)

    @override
    def run(self) -> Output:
        # the case spells betterlite's key names; the values below use LiteLLM's field names
        values = _config_values(self.config)
        values.update(_knob_values(self.knobs))
        try:
            config = ComplexityRouterConfig.model_validate(values)
        except ValidationError as error:
            raise ProbeError("invalid_request", str(error)) from error
        return Output(
            custom=config.has_custom_tiers,
            non_reasoning=config.enable_non_reasoning_tier,
            uses_llm_classifier=config.uses_llm_classifier,
            tier_names=list(config.tier_names()),
            wire_labels=list(config.classifier_wire_labels()),
            severity_order=[tier.value for tier in config.active_tier_severity_order()],
            labeled_tiers=[LabeledTier(tier=tier.value, label=label) for tier, label in config.labeled_tiers()],
            fallback_tier=config.fallback_tier,
            pools=_pools(config),
            resolved_tiers=[_resolved(config.resolve_classified_tier(query)) for query in self.queries],
            label_tiers=[_tier_name(config.tier_for_label(query)) for query in self.queries],
        )


def _config_values(config: ConfigInput | None) -> dict[str, object]:
    if config is None:
        return {}
    values: dict[str, object] = {"enable_non_reasoning_tier": config.non_reasoning_tier}
    if config.tier_definitions is not None:
        values["tier_definitions"] = [_definition_value(definition) for definition in config.tier_definitions]
    if config.tier_models is not None:
        values["tiers"] = _tier_values(config.tier_models)
    if config.tier_labels is not None:
        values["tier_labels"] = dict(config.tier_labels)
    return values


def _definition_value(definition: TierDefinitionInput) -> dict[str, object]:
    value: dict[str, object] = {"name": definition.name}
    if definition.description is not None:
        value["description"] = definition.description
    return value


def _tier_values(declared: Mapping[str, TierValue]) -> dict[str, object]:
    values: dict[str, object] = {}
    for tier, pool in declared.items():
        if isinstance(pool, list):
            values[tier] = [_entry_value(entry) for entry in pool]
        else:
            values[tier] = _entry_value(pool)
    return values


def _entry_value(entry: str | TierEntryInput) -> object:
    if isinstance(entry, str):
        return entry
    return {"model_name": entry.model_name, "litellm_params": dict(entry.litellm_params)}


def _knob_values(knobs: Knobs) -> dict[str, object]:
    values: dict[str, object] = {
        "classifier_type": knobs.classifier_type,
        "classifier_fallback": knobs.classifier_fallback,
    }
    if knobs.fallback_tier is not None:
        values["fallback_tier"] = knobs.fallback_tier
    if knobs.classification_prompt is not None:
        values["classification_prompt"] = knobs.classification_prompt
    if knobs.classification_examples is not None:
        values["classification_examples"] = knobs.classification_examples
    llm_config: dict[str, object] = {"model": _PROBE_CLASSIFIER_MODEL}
    if knobs.llm_system_prompt is not None:
        llm_config["system_prompt"] = knobs.llm_system_prompt
    if knobs.classification_rubric is not None:
        llm_config["classification_rubric"] = knobs.classification_rubric
    if knobs.classifier_type == "llm" or len(llm_config) > 1:
        values["classifier_llm_config"] = llm_config
    values.update(_feature_values(knobs.order_features))
    return values


def _feature_values(order_features: list[OrderFeatureName]) -> dict[str, object]:
    features = frozenset(order_features)
    values: dict[str, object] = {}
    if "adaptive" in features:
        values["adaptive"] = True
    if "session_affinity" in features:
        values["session_affinity"] = True
    if "escalation_keywords" in features:
        values["escalation_keywords"] = [_ESCALATION_KEYWORD]
    if "stall_escalation" in features:
        values["stall_escalation_enabled"] = True
    return values


def _pools(config: ComplexityRouterConfig) -> dict[str, PoolView]:
    pools: dict[str, PoolView] = {}
    for tier, value in config.tiers.items():
        if isinstance(value, list):
            names = value
            single = False
        else:
            names = [value]
            single = True
        stored = {
            entry.model_name: _JSON_OBJECT.validate_python(dict(entry.litellm_params))
            for entry in config.tier_model_configs.get(tier, ())
        }
        pools[tier] = PoolView(
            single=single,
            entries=[EntryView(model_name=name, params=stored.get(name, _EMPTY_PARAMS)) for name in names],
        )
    return pools


def _resolved(tier: ComplexityTier | str | None) -> str | None:
    if isinstance(tier, ComplexityTier):
        return tier.value
    return tier


def _tier_name(tier: ComplexityTier | None) -> str | None:
    return None if tier is None else tier.value
