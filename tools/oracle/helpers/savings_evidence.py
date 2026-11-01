from collections.abc import Callable
from decimal import Decimal
from typing import TYPE_CHECKING, TypeAlias, cast

from litellm.proxy.spend_tracking.compression_savings import (
    extract_compression_saved_tokens,
)
from litellm.proxy.spend_tracking.savings import (
    BaselineCostSnapshot,
    _baseline_usage,  # pyright: ignore[reportPrivateUsage]
    _coerce_billed_at,  # pyright: ignore[reportPrivateUsage]
    _pricing_basis,  # pyright: ignore[reportPrivateUsage]
    _recorded_token_cost,  # pyright: ignore[reportPrivateUsage]
    _usage_from_spend_log,  # pyright: ignore[reportPrivateUsage]
    extract_cache_creation_tokens,
    extract_cache_read_tokens,
    marks_gateway_injection,
    recorded_estimated_autorouter_savings,
)

if TYPE_CHECKING:
    from litellm.types.utils import ModelInfo

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def _decimal_str(value: float) -> str:
    return format(Decimal(repr(value)).normalize(), "f")


def _run_autorouter(input_value: dict[str, Json]) -> Json:
    metadata = cast(dict[str, Json], input_value["metadata"])
    savings = recorded_estimated_autorouter_savings(metadata)
    return {"autorouter_savings": None if savings is None else _decimal_str(savings)}


def _run_recorded_token_cost(input_value: dict[str, Json]) -> Json:
    cost_breakdown = cast("dict[str, Json] | None", input_value.get("cost_breakdown"))
    basis = _pricing_basis(cost_breakdown)
    recorded = _recorded_token_cost(cost_breakdown)
    return {
        "basis": list(basis),
        "recorded_token_cost": None if recorded is None else _decimal_str(recorded),
    }


def _run_cache_extraction(input_value: dict[str, Json]) -> Json:
    usage_object = cast("dict[str, Json] | None", input_value.get("usage_object"))
    return {
        "cache_read_tokens": extract_cache_read_tokens(usage_object),
        "cache_creation_tokens": extract_cache_creation_tokens(usage_object),
    }


def _run_baseline_usage(input_value: dict[str, Json]) -> Json:
    usage_object = cast(dict[str, Json], input_value["usage_object"])
    usage = _usage_from_spend_log(usage_object)
    if usage is None:
        return {"available": False}
    baseline_info = cast("ModelInfo | None", input_value.get("baseline_info"))
    normalized = _baseline_usage(usage, baseline_info)
    dump = normalized.model_dump()
    details = normalized.prompt_tokens_details
    return {
        "available": True,
        "prompt_tokens": normalized.prompt_tokens,
        "completion_tokens": dump.get("completion_tokens", 0),
        "total_tokens": dump.get("total_tokens", 0),
        "cache_read_input_tokens": dump.get("cache_read_input_tokens", 0),
        "cache_creation_input_tokens": dump.get("cache_creation_input_tokens", 0),
        "cached_tokens": None if details is None else details.cached_tokens,
        "cache_write_tokens": None if details is None else details.cache_write_tokens,
        "text_tokens": None if details is None else details.text_tokens,
        "has_cache_creation_token_details": details is not None
        and details.cache_creation_token_details is not None,
    }


def _run_injection(input_value: dict[str, Json]) -> Json:
    metadata = cast("dict[str, Json] | None", input_value.get("metadata"))
    model_id = cast("str | None", input_value.get("model_id"))
    return {"marks_gateway_injection": marks_gateway_injection(metadata, model_id)}


def _run_compression(input_value: dict[str, Json]) -> Json:
    metadata = cast(dict[str, Json], input_value["metadata"])
    return {"tokens_saved": extract_compression_saved_tokens(metadata)}


def _run_snapshot(input_value: dict[str, Json]) -> Json:
    snapshot = BaselineCostSnapshot.model_validate(input_value["snapshot"])
    return {
        "ok": True,
        "model": snapshot.model,
        "provider": snapshot.provider,
        "basis": list(snapshot.basis),
        "actual_spend": _decimal_str(snapshot.actual_spend),
        "actual_token_cost": (
            None if snapshot.actual_token_cost is None else _decimal_str(snapshot.actual_token_cost)
        ),
        "classifier_cost": _decimal_str(snapshot.classifier_cost),
        "has_prices": snapshot.prices is not None,
    }


def _run_billed_at(input_value: dict[str, Json]) -> Json:
    text = cast(str, input_value["text"])
    decoded = _coerce_billed_at(text)
    return {
        "decoded_unix_micros": None if decoded is None else round(decoded.timestamp() * 1_000_000),
    }


_OPERATIONS: dict[str, Callable[[dict[str, Json]], Json]] = {
    "autorouter": _run_autorouter,
    "recorded_token_cost": _run_recorded_token_cost,
    "cache_extraction": _run_cache_extraction,
    "baseline_usage": _run_baseline_usage,
    "injection": _run_injection,
    "compression": _run_compression,
    "snapshot": _run_snapshot,
    "billed_at": _run_billed_at,
}


def run(input_value: dict[str, Json]) -> Json:
    operation = cast(str, input_value["operation"])
    handler = _OPERATIONS.get(operation)
    if handler is None:
        raise ValueError(f"unknown savings_evidence operation: {operation}")
    return handler(input_value)
