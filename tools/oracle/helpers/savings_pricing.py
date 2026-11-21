from decimal import Decimal
from typing import Literal, NotRequired, TypedDict, cast

from litellm.proxy.spend_tracking.savings import (
    BaselineCostSnapshot,
    compute_autorouter_savings,
    compute_savings_spend,
    price_baseline_comparison,
)
from litellm.types.utils import ModelInfo, Usage
from pydantic import JsonValue, TypeAdapter


class CacheCreationDetails(TypedDict, total=False):
    ephemeral_5m_input_tokens: int
    ephemeral_1h_input_tokens: int


class PromptDetails(TypedDict, total=False):
    cached_tokens: int
    cache_creation_tokens: int
    cache_write_tokens: int
    text_tokens: int
    audio_tokens: int
    image_tokens: int
    video_tokens: int
    cache_creation_token_details: CacheCreationDetails


class UsagePayload(TypedDict, total=False):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int
    cache_read_input_tokens: int
    cache_creation_input_tokens: int
    prompt_tokens_details: PromptDetails


class CostBreakdownPayload(TypedDict, total=False):
    input_cost: float
    output_cost: float
    service_tier: str
    data_residency: str
    vertex_location: str


class RoutingPayload(TypedDict, total=False):
    classifier_cost: float
    savings_baseline_model: str
    savings_baseline_deployment_id: str


class EstimatePayload(TypedDict):
    version: int
    status: str


class AutorouterCase(TypedDict):
    operation: Literal["compute_autorouter_savings"]
    usage: UsagePayload
    baseline_model: NotRequired[str]
    selected_model: NotRequired[str]
    selected_provider: NotRequired[str]
    baseline_usage: NotRequired[UsagePayload]
    selected_info: NotRequired[dict[str, JsonValue]]
    baseline_info: NotRequired[dict[str, JsonValue]]
    cost_breakdown: NotRequired[CostBreakdownPayload]
    baseline_deployment_id: NotRequired[str]
    selected_deployment_id: NotRequired[str]
    baseline_provenance: NotRequired[Literal["observed_initial", "modeled"]]


class ComparisonCase(TypedDict):
    operation: Literal["price_baseline_comparison"]
    snapshot: dict[str, JsonValue]
    baseline_usage: NotRequired[UsagePayload]
    provenance: NotRequired[Literal["observed_identical", "modeled"]]


class SpendCase(TypedDict):
    operation: Literal["compute_savings_spend"]
    compression_saved_tokens: int
    gateway_injected_cache: bool
    model: NotRequired[str]
    custom_llm_provider: NotRequired[str]
    routing_decision: NotRequired[RoutingPayload]
    usage: NotRequired[UsagePayload]
    model_id: NotRequired[str]
    cost_breakdown: NotRequired[CostBreakdownPayload]
    recorded_autorouter_savings: NotRequired[float]
    recorded_estimate: NotRequired[EstimatePayload]


class CaseTag(TypedDict):
    operation: str


_TAG: TypeAdapter[CaseTag] = TypeAdapter(CaseTag)
_AUTOROUTER: TypeAdapter[AutorouterCase] = TypeAdapter(AutorouterCase)
_COMPARISON: TypeAdapter[ComparisonCase] = TypeAdapter(ComparisonCase)
_SPEND: TypeAdapter[SpendCase] = TypeAdapter(SpendCase)


def usage_value(payload: UsagePayload) -> Usage:
    details_payload = payload.get("prompt_tokens_details")
    details: dict[str, JsonValue] | None = None
    if details_payload is not None:
        details = {
            "cached_tokens": details_payload.get("cached_tokens"),
            "cache_creation_tokens": details_payload.get("cache_creation_tokens"),
            "cache_write_tokens": details_payload.get("cache_write_tokens"),
            "text_tokens": details_payload.get("text_tokens"),
            "audio_tokens": details_payload.get("audio_tokens"),
            "image_tokens": details_payload.get("image_tokens"),
            "video_tokens": details_payload.get("video_tokens"),
        }
        ttl_payload = details_payload.get("cache_creation_token_details")
        if ttl_payload is not None:
            details["cache_creation_token_details"] = {
                "ephemeral_5m_input_tokens": ttl_payload.get("ephemeral_5m_input_tokens"),
                "ephemeral_1h_input_tokens": ttl_payload.get("ephemeral_1h_input_tokens"),
            }
    return Usage(
        prompt_tokens=payload.get("prompt_tokens"),
        completion_tokens=payload.get("completion_tokens"),
        total_tokens=payload.get("total_tokens"),
        prompt_tokens_details=details,
        cache_read_input_tokens=payload.get("cache_read_input_tokens"),
        cache_creation_input_tokens=payload.get("cache_creation_input_tokens"),
    )


def autorouter_value(case: AutorouterCase) -> dict[str, JsonValue]:
    baseline_payload = case.get("baseline_usage")
    baseline_usage = None if baseline_payload is None else usage_value(baseline_payload)
    savings = compute_autorouter_savings(
        baseline_model=case.get("baseline_model"),
        selected_model=case.get("selected_model"),
        selected_provider=case.get("selected_provider"),
        usage=usage_value(case["usage"]),
        baseline_info=cast("ModelInfo | None", case.get("baseline_info")),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L402
        selected_info=cast("ModelInfo | None", case.get("selected_info")),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L402
        cost_breakdown=case.get("cost_breakdown"),
        baseline_deployment_id=case.get("baseline_deployment_id"),
        selected_deployment_id=case.get("selected_deployment_id"),
        baseline_usage=baseline_usage,
        baseline_provenance=case.get("baseline_provenance"),
    )
    return {
        "savings": None
        if savings is None
        else format(Decimal(repr(savings)).normalize(), "f")
    }


def comparison_value(case: ComparisonCase) -> dict[str, JsonValue]:
    baseline_payload = case.get("baseline_usage")
    baseline_usage = None if baseline_payload is None else usage_value(baseline_payload)
    costs = price_baseline_comparison(
        BaselineCostSnapshot.model_validate(case["snapshot"]),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/spend_tracking/savings.py#L125
        baseline_usage,
        case.get("provenance"),
    )
    if costs is None:
        return {"actual": None, "baseline": None}
    return {
        "actual": format(Decimal(repr(costs.actual)).normalize(), "f"),
        "baseline": format(Decimal(repr(costs.baseline)).normalize(), "f"),
    }


def spend_value(case: SpendCase) -> dict[str, JsonValue]:
    spend = compute_savings_spend(
        model=case.get("model"),
        custom_llm_provider=case.get("custom_llm_provider"),
        compression_saved_tokens=case["compression_saved_tokens"],
        gateway_injected_cache=case["gateway_injected_cache"],
        routing_decision=case.get("routing_decision"),
        usage_object=case.get("usage"),
        model_id=case.get("model_id"),
        cost_breakdown=case.get("cost_breakdown"),
        recorded_autorouter_savings=case.get("recorded_autorouter_savings"),
        recorded_autorouter_savings_estimate=case.get("recorded_estimate"),
    )
    return {
        "compression": format(Decimal(repr(spend.compression)).normalize(), "f"),
        "prompt_caching": format(Decimal(repr(spend.prompt_caching)).normalize(), "f"),
        "autorouter": format(Decimal(repr(spend.autorouter)).normalize(), "f"),
        "gateway_injected_caching": format(
            Decimal(repr(spend.gateway_injected_caching)).normalize(), "f"
        ),
    }


def run(input_value: dict[str, JsonValue]) -> dict[str, JsonValue]:
    operation = _TAG.validate_python(input_value)["operation"]
    if operation == "compute_autorouter_savings":
        return autorouter_value(_AUTOROUTER.validate_python(input_value))
    if operation == "price_baseline_comparison":
        return comparison_value(_COMPARISON.validate_python(input_value))
    if operation == "compute_savings_spend":
        return spend_value(_SPEND.validate_python(input_value))
    raise ValueError(f"unknown savings_pricing operation: {operation}")
