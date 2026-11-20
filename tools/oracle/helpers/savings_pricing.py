from decimal import Decimal
from typing import Literal, NotRequired, TypedDict

from litellm.proxy.spend_tracking.savings import (
    BaselineCostSnapshot,
    compute_autorouter_savings,
    compute_savings_spend,
    price_baseline_comparison,
)
from litellm.types.utils import ModelInfo, Usage
from pydantic import TypeAdapter


class CacheCreationDetailsPayload(TypedDict, total=False):
    ephemeral_5m_input_tokens: int
    ephemeral_1h_input_tokens: int


class PromptDetailsPayload(TypedDict, total=False):
    cached_tokens: int
    cache_creation_tokens: int
    cache_write_tokens: int
    text_tokens: int
    audio_tokens: int
    image_tokens: int
    video_tokens: int
    cache_creation_token_details: CacheCreationDetailsPayload


class UsagePayload(TypedDict, total=False):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int
    cache_read_input_tokens: int
    cache_creation_input_tokens: int
    prompt_tokens_details: PromptDetailsPayload


class PricePayload(TypedDict):
    key: str
    max_tokens: int | None
    max_input_tokens: int | None
    max_output_tokens: int | None
    input_cost_per_token: float | None
    output_cost_per_token: float | None
    litellm_provider: str
    mode: Literal["chat"]
    supported_openai_params: list[str] | None
    cache_read_input_token_cost: NotRequired[float | None]
    cache_creation_input_token_cost: NotRequired[float | None]
    cache_creation_input_token_cost_above_1hr: NotRequired[float | None]


class SnapshotPayload(TypedDict):
    model: str
    provider: str
    prices: PricePayload | None
    actual_spend: float
    actual_token_cost: NotRequired[float | None]
    classifier_cost: NotRequired[float]


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


class CaseTag(TypedDict):
    operation: str


class AutorouterCase(TypedDict):
    operation: Literal["compute_autorouter_savings"]
    usage: UsagePayload
    baseline_model: NotRequired[str]
    selected_model: NotRequired[str]
    selected_provider: NotRequired[str]
    baseline_usage: NotRequired[UsagePayload]
    selected_info: NotRequired[PricePayload]
    baseline_info: NotRequired[PricePayload]
    cost_breakdown: NotRequired[CostBreakdownPayload]
    baseline_deployment_id: NotRequired[str]
    selected_deployment_id: NotRequired[str]
    baseline_provenance: NotRequired[Literal["observed_initial", "modeled"]]


class ComparisonCase(TypedDict):
    operation: Literal["price_baseline_comparison"]
    snapshot: SnapshotPayload
    baseline_usage: NotRequired[UsagePayload]
    provenance: NotRequired[Literal["observed_identical", "modeled"]]


class SpendCase(TypedDict, total=False):
    operation: Literal["compute_savings_spend"]
    model: str
    custom_llm_provider: str
    compression_saved_tokens: int
    gateway_injected_cache: bool
    routing_decision: RoutingPayload
    usage: UsagePayload
    model_id: str
    cost_breakdown: CostBreakdownPayload
    recorded_autorouter_savings: float
    recorded_autorouter_savings_estimate: EstimatePayload


_TAG: TypeAdapter[CaseTag] = TypeAdapter(CaseTag)
_AUTOROUTER: TypeAdapter[AutorouterCase] = TypeAdapter(AutorouterCase)
_COMPARISON: TypeAdapter[ComparisonCase] = TypeAdapter(ComparisonCase)
_SPEND: TypeAdapter[SpendCase] = TypeAdapter(SpendCase)
_MODEL_INFO: TypeAdapter[ModelInfo] = TypeAdapter(ModelInfo)


def _decimal_str(value: float) -> str:
    return format(Decimal(repr(value)).normalize(), "f")


def _usage(payload: UsagePayload) -> Usage:
    details_payload = payload.get("prompt_tokens_details")
    details: dict[str, object] | None = None
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
        ttl = details_payload.get("cache_creation_token_details")
        if ttl is not None:
            details["cache_creation_token_details"] = {
                "ephemeral_5m_input_tokens": ttl.get("ephemeral_5m_input_tokens"),
                "ephemeral_1h_input_tokens": ttl.get("ephemeral_1h_input_tokens"),
            }
    return Usage(
        prompt_tokens=payload.get("prompt_tokens"),
        completion_tokens=payload.get("completion_tokens"),
        total_tokens=payload.get("total_tokens"),
        prompt_tokens_details=details,
        cache_read_input_tokens=payload.get("cache_read_input_tokens"),
        cache_creation_input_tokens=payload.get("cache_creation_input_tokens"),
    )


def _model_info(payload: PricePayload | None) -> ModelInfo | None:
    if payload is None:
        return None
    return _MODEL_INFO.validate_python(payload)


def _compute_autorouter(case: AutorouterCase) -> dict[str, object]:
    baseline = case.get("baseline_usage")
    baseline_usage = None if baseline is None else _usage(baseline)
    savings = compute_autorouter_savings(
        baseline_model=case.get("baseline_model"),
        selected_model=case.get("selected_model"),
        selected_provider=case.get("selected_provider"),
        usage=_usage(case["usage"]),
        baseline_info=_model_info(case.get("baseline_info")),
        selected_info=_model_info(case.get("selected_info")),
        cost_breakdown=case.get("cost_breakdown"),
        baseline_deployment_id=case.get("baseline_deployment_id"),
        selected_deployment_id=case.get("selected_deployment_id"),
        baseline_usage=baseline_usage,
        baseline_provenance=case.get("baseline_provenance"),
    )
    return {"savings": None if savings is None else _decimal_str(savings)}


def _price_comparison(case: ComparisonCase) -> dict[str, object]:
    snapshot = BaselineCostSnapshot.model_validate(case["snapshot"])
    baseline = case.get("baseline_usage")
    baseline_usage = None if baseline is None else _usage(baseline)
    costs = price_baseline_comparison(snapshot, baseline_usage, case.get("provenance"))
    if costs is None:
        return {"actual": None, "baseline": None}
    return {"actual": _decimal_str(costs.actual), "baseline": _decimal_str(costs.baseline)}


def _savings_spend(case: SpendCase) -> dict[str, object]:
    spend = compute_savings_spend(
        model=case.get("model"),
        custom_llm_provider=case.get("custom_llm_provider"),
        compression_saved_tokens=case.get("compression_saved_tokens", 0),
        gateway_injected_cache=case.get("gateway_injected_cache", False),
        routing_decision=case.get("routing_decision"),
        usage_object=case.get("usage"),
        model_id=case.get("model_id"),
        cost_breakdown=case.get("cost_breakdown"),
        recorded_autorouter_savings=case.get("recorded_autorouter_savings"),
        recorded_autorouter_savings_estimate=case.get("recorded_autorouter_savings_estimate"),
    )
    return {
        "compression": _decimal_str(spend.compression),
        "prompt_caching": _decimal_str(spend.prompt_caching),
        "autorouter": _decimal_str(spend.autorouter),
        "gateway_injected_caching": _decimal_str(spend.gateway_injected_caching),
    }


def run(input_value: dict[str, object]) -> dict[str, object]:
    operation = _TAG.validate_python(input_value)["operation"]
    if operation == "compute_autorouter_savings":
        return _compute_autorouter(_AUTOROUTER.validate_python(input_value))
    if operation == "price_baseline_comparison":
        return _price_comparison(_COMPARISON.validate_python(input_value))
    if operation == "compute_savings_spend":
        return _savings_spend(_SPEND.validate_python(input_value))
    raise ValueError(f"unknown savings_pricing operation: {operation}")
