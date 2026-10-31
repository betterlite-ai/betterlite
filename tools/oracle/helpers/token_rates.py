from decimal import Decimal
from typing import TypeAlias, cast

from litellm.litellm_core_utils.llm_cost_calc.utils import (
    _get_token_base_cost,  # pyright: ignore[reportPrivateUsage]
    calculate_cost_component,
    deployment_pricing,
    get_batch_cost_rates,
)
from litellm.types.utils import ModelInfo, Usage

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def _usage(value: dict[str, Json]) -> Usage:
    return Usage(prompt_tokens=int(cast(int, value["prompt_tokens"])))


def _strings(value: object) -> Json:
    if isinstance(value, Decimal):
        return format(value.normalize(), "f")
    if isinstance(value, dict):
        entries = cast(dict[object, object], value)
        return {str(key): _strings(item) for key, item in entries.items()}
    if isinstance(value, tuple):
        items = cast(tuple[object, ...], value)
        return [_strings(item) for item in items]
    return cast(Json, value)


def run(input_value: dict[str, Json]) -> Json:
    operation = input_value["operation"]
    model_info = cast(ModelInfo, input_value.get("model_info", {}))
    usage = _usage(cast(dict[str, Json], input_value.get("usage", {"prompt_tokens": 0})))
    if operation == "base":
        rates = _get_token_base_cost(
            model_info,
            usage,
            cast(str | None, input_value.get("service_tier")),
            None,
            threshold_is_inclusive=input_value.get("provider") == "xai",
        )
        return _strings(rates)
    if operation == "batch":
        provider = cast(str | None, input_value.get("provider"))
        return _strings(get_batch_cost_rates(model_info, usage, provider))
    if operation == "component":
        value = input_value.get("usage")
        usage_value = None if value is None else float(cast(float | int, value))
        cost_key = str(input_value["cost_key"])
        return _strings(calculate_cost_component(model_info, cost_key, usage_value))
    if operation == "deployment":
        return _strings(deployment_pricing(cast(ModelInfo | None, input_value.get("model_info"))))
    raise ValueError(f"unknown token_rates operation: {operation}")
