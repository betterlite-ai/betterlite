from decimal import Decimal
from typing import TypeAlias, cast

import litellm
from litellm.llms.azure_ai.cost_calculator import (
    _router_fee_entry_name,  # pyright: ignore[reportPrivateUsage]
    _router_fee_name,  # pyright: ignore[reportPrivateUsage]
    calculate_azure_model_router_flat_cost,
    is_azure_model_router,
    is_router_fee_entry,
)
from litellm.llms.azure_ai.cost_calculator import (
    cost_per_token as foundry_cost_per_token,
)
from litellm.types.utils import Usage
from litellm.utils import _cached_get_model_info  # pyright: ignore[reportPrivateUsage]

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def _decimal_str(value: float) -> str:
    return format(Decimal(repr(value)).normalize(), "f")


def _object(value: Json) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError("expected a JSON object")
    return cast("dict[str, Json]", value)


def _objects(fields: dict[str, Json], key: str) -> list[dict[str, Json]]:
    value = fields.get(key)
    if not isinstance(value, list):
        raise ValueError(f"{key} must be a JSON array")
    return [_object(cast("Json", item)) for item in value]


def _text(fields: dict[str, Json], key: str) -> str:
    value = fields.get(key)
    if not isinstance(value, str):
        raise ValueError(f"{key} must be a string")
    return value


def _optional_text(fields: dict[str, Json], key: str) -> str | None:
    value = fields.get(key)
    if value is None:
        return None
    if not isinstance(value, str):
        raise ValueError(f"{key} must be a string or null")
    return value


def _integer(fields: dict[str, Json], key: str) -> int:
    value = fields.get(key)
    if not isinstance(value, int) or isinstance(value, bool):
        raise ValueError(f"{key} must be a JSON integer")
    return value


def _apply_catalog(input_value: dict[str, Json]) -> dict[str, Json]:
    catalog = input_value.get("catalog")
    contents: dict[str, Json] = {} if catalog is None else _object(catalog)
    model_cost = cast("dict[str, Json]", litellm.model_cost)
    original = dict(model_cost)
    model_cost.clear()
    model_cost.update(contents)
    # get_model_info memoizes per name, so a swapped catalog drops the previous answers
    _cached_get_model_info.cache_clear()
    return original


def _restore_catalog(original: dict[str, Json]) -> None:
    model_cost = cast("dict[str, Json]", litellm.model_cost)
    model_cost.clear()
    model_cost.update(original)
    _cached_get_model_info.cache_clear()


def _classify_call(call: dict[str, Json]) -> dict[str, Json]:
    model = _text(call, "model")
    return {
        "is_router": is_azure_model_router(model),
        "is_fee_entry": is_router_fee_entry(model),
        "fee_entry_name": _router_fee_entry_name(model),
        "fee_name": _router_fee_name(model, _optional_text(call, "request_model")),
    }


def _run_classify(input_value: dict[str, Json]) -> Json:
    return cast("Json", [_classify_call(call) for call in _objects(input_value, "calls")])


def _run_flat(input_value: dict[str, Json]) -> Json:
    original = _apply_catalog(input_value)
    try:
        return cast(
            "Json",
            [
                {
                    "fee": _decimal_str(
                        calculate_azure_model_router_flat_cost(
                            model=_text(call, "model"),
                            prompt_tokens=_integer(call, "prompt_tokens"),
                        )
                    )
                }
                for call in _objects(input_value, "calls")
            ],
        )
    finally:
        _restore_catalog(original)


def _run_cost(input_value: dict[str, Json]) -> Json:
    original = _apply_catalog(input_value)
    try:
        reports: list[Json] = []
        for call in _objects(input_value, "calls"):
            prompt_tokens = _integer(call, "prompt_tokens")
            completion_tokens = _integer(call, "completion_tokens")
            usage = Usage.model_construct(
                prompt_tokens=prompt_tokens,
                completion_tokens=completion_tokens,
                total_tokens=prompt_tokens + completion_tokens,
                prompt_tokens_details=None,
                completion_tokens_details=None,
            )
            prompt_cost, completion_cost = foundry_cost_per_token(
                model=_text(call, "model"),
                usage=usage,
                response_time_ms=cast("float | None", call.get("response_time_ms")),
                request_model=_optional_text(call, "request_model"),
                service_tier=_optional_text(call, "service_tier"),
            )
            reports.append(
                {
                    "prompt_cost": _decimal_str(prompt_cost),
                    "completion_cost": _decimal_str(completion_cost),
                }
            )
        return cast("Json", reports)
    finally:
        _restore_catalog(original)


def run(input_value: dict[str, Json]) -> Json:
    operation = input_value["operation"]
    if operation == "classify":
        return _run_classify(input_value)
    if operation == "flat":
        return _run_flat(input_value)
    if operation == "cost":
        return _run_cost(input_value)
    raise ValueError(f"unknown foundry_router_cost operation: {operation}")
