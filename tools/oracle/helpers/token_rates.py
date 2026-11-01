from dataclasses import asdict
from datetime import datetime
from decimal import Decimal
from typing import TypeAlias, cast

from litellm.litellm_core_utils.llm_cost_calc.utils import (
    _get_token_base_cost,  # pyright: ignore[reportPrivateUsage]
    _resolve_billed_reasoning_rate,  # pyright: ignore[reportPrivateUsage]
    calculate_cost_component,
    deployment_pricing,
    get_batch_cost_rates,
)
from litellm.types.utils import ModelInfo, Usage

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def _usage(value: dict[str, Json]) -> Usage:
    return Usage(prompt_tokens=int(cast(int, value["prompt_tokens"])))


def _decimal_str(value: float) -> str:
    return format(Decimal(repr(value)).normalize(), "f")


def _strings(value: object) -> Json:
    if isinstance(value, Decimal):
        return format(value.normalize(), "f")
    if isinstance(value, float):
        return _decimal_str(value)
    if isinstance(value, dict):
        entries = cast(dict[object, object], value)
        return {str(key): _strings(item) for key, item in entries.items()}
    if isinstance(value, (tuple, list)):
        items = cast(tuple[object, ...], value)
        return [_strings(item) for item in items]
    return cast(Json, value)


def _base_model_info(input_value: dict[str, Json]) -> ModelInfo:
    model_info = dict(cast(dict[str, Json], input_value.get("model_info", {})))
    tiered_pricing = input_value.get("tiered_pricing")
    if tiered_pricing:
        model_info["tiered_pricing"] = tiered_pricing
    off_peak_pricing = input_value.get("off_peak_pricing")
    if off_peak_pricing is not None:
        model_info["off_peak_pricing"] = off_peak_pricing
    return cast(ModelInfo, model_info)


def _run_base(input_value: dict[str, Json]) -> Json:
    model_info = _base_model_info(input_value)
    usage = _usage(cast(dict[str, Json], input_value.get("usage", {"prompt_tokens": 0})))
    service_tier = cast("str | None", input_value.get("service_tier"))
    moment_value = input_value.get("moment")
    current_time = None if moment_value is None else datetime.fromisoformat(str(moment_value))
    prompt, completion, cache_creation, cache_creation_above_1hr, cache_read = _get_token_base_cost(
        model_info,
        usage,
        service_tier,
        current_time,
        threshold_is_inclusive=input_value.get("provider") == "xai",
    )
    reasoning_rate = _resolve_billed_reasoning_rate(
        model_info=model_info,
        usage=usage,
        service_tier=service_tier,
        completion_base_cost=completion,
        current_time=current_time,
    )
    return {
        "input_rate": _decimal_str(prompt),
        "output_rate": _decimal_str(completion),
        "cache_read_rate": _decimal_str(cache_read),
        "cache_creation_rate": _decimal_str(cache_creation),
        "cache_creation_above_1hr_rate": _decimal_str(cache_creation_above_1hr),
        "reasoning_rate": _decimal_str(reasoning_rate),
    }


def run(input_value: dict[str, Json]) -> Json:
    operation = input_value["operation"]
    model_info = cast(ModelInfo, input_value.get("model_info", {}))
    usage = _usage(cast(dict[str, Json], input_value.get("usage", {"prompt_tokens": 0})))
    if operation == "base":
        return _run_base(input_value)
    if operation == "batch":
        provider = cast(str | None, input_value.get("provider"))
        rates = get_batch_cost_rates(model_info, usage, provider)
        return _strings(asdict(rates))
    if operation == "component":
        value = input_value.get("usage")
        usage_value = None if value is None else float(cast(float | int, value))
        cost_key = str(input_value["cost_key"])
        return _strings(calculate_cost_component(model_info, cost_key, usage_value))
    if operation == "deployment":
        pricing = deployment_pricing(cast("ModelInfo | None", input_value.get("model_info")))
        return _strings(None if pricing is None else dict(pricing))
    raise ValueError(f"unknown token_rates operation: {operation}")
