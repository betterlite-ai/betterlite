from __future__ import annotations

import datetime as dt
import json
from decimal import Decimal
from typing import TypeAlias, cast

import litellm
from litellm import cost_calculator
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.litellm_core_utils.llm_cost_calc.utils import BilledTokenRates
from pydantic import BaseModel, ConfigDict, PrivateAttr

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class _Response(BaseModel):
    model_config = ConfigDict(extra="allow")
    model: str | None = None
    _hidden_params: dict[str, Json] = PrivateAttr(default_factory=dict)
    _response_ms: Json = PrivateAttr(default=None)


def _object(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def _optional_object(value: Json, name: str) -> dict[str, Json] | None:
    return None if value is None else _object(value, name)


def _string(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def _optional_string(value: Json, name: str) -> str | None:
    return None if value is None else _string(value, name)


def _number(value: Json, name: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError(f"{name} must be a number")
    return float(value)


def _optional_number(value: Json, name: str) -> float | None:
    return None if value is None else _number(value, name)


def _optional_bool(value: Json, name: str) -> bool | None:
    if value is None:
        return None
    if not isinstance(value, bool):
        raise ValueError(f"{name} must be a boolean")
    return value


def _decimal_str(value: float) -> str:
    return format(Decimal(repr(value)).normalize(), "f")


def _optional_decimal_str(value: float | None) -> Json:
    return None if value is None else _decimal_str(value)


def _apply_catalog(value: Json) -> None:
    model_cost = cast("dict[str, Json]", litellm.model_cost)
    model_cost.clear()
    if value is None:
        return
    if not isinstance(value, list):
        raise ValueError("catalog must be a list")
    for item in value:
        entry = _object(item, "catalog entry")
        key = _string(entry.get("key"), "key")
        model_cost[key] = _object(entry.get("entry"), "entry")


def _response(spec: dict[str, Json]) -> object:
    kind = _string(spec.get("kind"), "kind")
    model = _optional_string(spec.get("model"), "model")
    hidden = spec.get("hidden_params")
    if kind == "dictionary":
        response: dict[str, Json] = {"model": model}
        if hidden is not None:
            response["_hidden_params"] = hidden
        return response
    if kind != "object":
        raise ValueError(f"unknown response kind {kind}")
    response_object = _Response(model=model)
    if hidden is not None:
        response_object._hidden_params = _object(hidden, "hidden_params")  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    stamped = spec.get("response_ms")
    if stamped is not None:
        response_object._response_ms = stamped  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    return response_object


def _identity(call: dict[str, Json]) -> Json:
    spec = _object(call.get("identity"), "identity")
    _apply_catalog(call.get("catalog"))
    response = _optional_object(spec.get("response"), "response")
    selected = cost_calculator._select_model_name_for_cost_calc(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        model=_optional_string(spec.get("model"), "model"),
        completion_response=None if response is None else _response(response),
        base_model=_optional_string(spec.get("base_model"), "base_model"),
        custom_pricing=_optional_bool(spec.get("custom_pricing"), "custom_pricing"),
        custom_llm_provider=_string(spec.get("custom_llm_provider"), "custom_llm_provider"),
        router_model_id=_optional_string(spec.get("router_model_id"), "router_model_id"),
        region_name=_optional_string(spec.get("region_name"), "region_name"),
    )
    return cast(Json, selected)


def _timing_object(spec: dict[str, Json]) -> Logging:
    logging_object = object.__new__(Logging)
    logging_object.model_call_details = {
        "start_time": _datetime(spec.get("start_time")),
        "end_time": _datetime(spec.get("end_time")),
    }
    return logging_object


def _datetime(value: Json) -> dt.datetime:
    return dt.datetime.fromisoformat(_string(value, "timestamp"))


def _response_time(call: dict[str, Json]) -> Json:
    spec = _object(call.get("response_time"), "response_time")
    response = _response(_object(spec.get("response"), "response"))
    timing = _optional_object(spec.get("timing"), "timing")
    observed = cost_calculator._response_time_ms_for_cost(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        response,
        None if timing is None else _timing_object(timing),
        _optional_number(spec.get("total_time"), "total_time"),
    )
    return _decimal_str(float(observed))


def _rates(spec: dict[str, Json]) -> BilledTokenRates:
    return BilledTokenRates(
        input_cost_per_token=_number(spec.get("input_cost_per_token"), "input_cost_per_token"),
        output_cost_per_token=_number(spec.get("output_cost_per_token"), "output_cost_per_token"),
        cache_read_input_token_cost=_number(
            spec.get("cache_read_input_token_cost"), "cache_read_input_token_cost"
        ),
        cache_read_input_audio_token_cost=_number(spec.get("audio_read"), "audio_read"),
        cache_creation_input_token_cost=_number(spec.get("cache_write"), "cache_write"),
        cache_creation_input_token_cost_above_1hr=_number(
            spec.get("cache_write_1h"), "cache_write_1h"
        ),
        output_cost_per_reasoning_token=_number(spec.get("reasoning"), "reasoning"),
    )


def _optional_cost_map(value: Json) -> dict[str, float] | None:
    if value is None:
        return None
    return {key: _number(item, key) for key, item in _object(value, "additional_costs").items()}


def _cost_map_value(value: dict[str, float] | None) -> Json:
    return None if value is None else {key: _decimal_str(item) for key, item in value.items()}


def _record_value(logging_object: Logging) -> Json:
    breakdown = logging_object.cost_breakdown
    rates = logging_object.billed_token_rates
    if breakdown is None:
        return {"cost_breakdown": None, "billed_token_rates": None}
    return {
        "cost_breakdown": {
            "input_cost": _optional_decimal_str(breakdown.get("input_cost")),
            "output_cost": _optional_decimal_str(breakdown.get("output_cost")),
            "total_cost": _optional_decimal_str(breakdown.get("total_cost")),
            "tool_usage_cost": _optional_decimal_str(breakdown.get("tool_usage_cost")),
            "cache_read_cost": _optional_decimal_str(breakdown.get("cache_read_cost")),
            "cache_creation_cost": _optional_decimal_str(breakdown.get("cache_creation_cost")),
            "reasoning_cost": _optional_decimal_str(breakdown.get("reasoning_cost")),
            "additional_costs": _cost_map_value(breakdown.get("additional_costs")),
            "original_cost": _optional_decimal_str(breakdown.get("original_cost")),
            "discount_percent": _optional_decimal_str(breakdown.get("discount_percent")),
            "discount_amount": _optional_decimal_str(breakdown.get("discount_amount")),
            "margin_percent": _optional_decimal_str(breakdown.get("margin_percent")),
            "margin_fixed_amount": _optional_decimal_str(breakdown.get("margin_fixed_amount")),
            "margin_total_amount": _optional_decimal_str(breakdown.get("margin_total_amount")),
            "service_tier": breakdown.get("service_tier"),
            "data_residency": breakdown.get("data_residency"),
            "vertex_location": breakdown.get("vertex_location"),
        },
        "billed_token_rates": None
        if rates is None
        else {
            "input_cost_per_token": _decimal_str(rates.input_cost_per_token),
            "output_cost_per_token": _decimal_str(rates.output_cost_per_token),
            "cache_read_input_token_cost": _decimal_str(rates.cache_read_input_token_cost),
            "audio_read": _decimal_str(rates.cache_read_input_audio_token_cost),
            "cache_write": _decimal_str(rates.cache_creation_input_token_cost),
            "cache_write_1h": _decimal_str(rates.cache_creation_input_token_cost_above_1hr),
            "reasoning": _decimal_str(rates.output_cost_per_reasoning_token),
        },
    }


def _record(logging_object: Logging, spec: dict[str, Json]) -> Json:
    rates = spec.get("billed_token_rates")
    logging_object.set_cost_breakdown(  # pyright: ignore[reportUnknownMemberType]
        input_cost=_number(spec.get("input_cost"), "input_cost"),
        output_cost=_number(spec.get("output_cost"), "output_cost"),
        total_cost=_number(spec.get("total_cost"), "total_cost"),
        cost_for_built_in_tools_cost_usd_dollar=_number(
            spec.get("tool_usage_cost"), "tool_usage_cost"
        ),
        additional_costs=_optional_cost_map(spec.get("additional_costs")),
        original_cost=_optional_number(spec.get("original_cost"), "original_cost"),
        discount_percent=_optional_number(spec.get("discount_percent"), "discount_percent"),
        discount_amount=_optional_number(spec.get("discount_amount"), "discount_amount"),
        margin_percent=_optional_number(spec.get("margin_percent"), "margin_percent"),
        margin_fixed_amount=_optional_number(
            spec.get("margin_fixed_amount"), "margin_fixed_amount"
        ),
        margin_total_amount=_optional_number(
            spec.get("margin_total_amount"), "margin_total_amount"
        ),
        cache_read_cost=_optional_number(spec.get("cache_read_cost"), "cache_read_cost"),
        cache_creation_cost=_optional_number(
            spec.get("cache_creation_cost"), "cache_creation_cost"
        ),
        reasoning_cost=_optional_number(spec.get("reasoning_cost"), "reasoning_cost"),
        service_tier=_optional_string(spec.get("service_tier"), "service_tier"),
        data_residency=_optional_string(spec.get("data_residency"), "data_residency"),
        vertex_location=_optional_string(spec.get("vertex_location"), "vertex_location"),
        billed_token_rates=None if rates is None else _rates(_object(rates, "billed_token_rates")),
    )
    return _record_value(logging_object)


def _breakdown(call: dict[str, Json]) -> Json:
    items = call.get("breakdowns")
    if not isinstance(items, list):
        raise ValueError("breakdowns must be a list")
    logging_object = object.__new__(Logging)
    return [_record(logging_object, _object(item, "breakdown")) for item in items]


def _execute(function: str, call: dict[str, Json]) -> Json:
    match function:
        case "identity":
            return _identity(call)
        case "response_time":
            return _response_time(call)
        case "breakdown":
            return _breakdown(call)
        case _:
            raise ValueError(f"unknown function {function}")


def _run_call(call: dict[str, Json]) -> Json:
    try:
        value = _execute(_string(call.get("function"), "function"), call)
        encoded = cast(Json, json.loads(json.dumps(value)))
    except Exception as error:  # noqa: BLE001 - a rejected probe is part of the compared outcome
        return {"ok": False, "error": {"class": type(error).__name__}}
    return {"ok": True, "value": encoded}


def run(input_value: dict[str, Json]) -> Json:
    calls = input_value.get("calls")
    if not isinstance(calls, list):
        raise ValueError("calls must be a list")
    return [_run_call(_object(call, "call")) for call in calls]
