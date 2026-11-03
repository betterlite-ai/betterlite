from collections.abc import Mapping
from typing import TypeAlias, cast

from litellm.integrations.otel.model.trace_controls import caller_trace_controls
from litellm.integrations.otel.model.utils import (
    as_bool,
    as_float,
    as_int,
    as_str,
    as_str_mapping,
    as_str_tuple,
    to_ns,
    to_seconds,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def run(input_value: dict[str, Json]) -> Json:
    function = input_value.get("function")
    if function == "trace":
        controls = caller_trace_controls(input_value)
        return {
            "name": controls.name,
            "user_id": controls.user_id,
            "session_id": controls.session_id,
            "tags": list(controls.tags),
        }
    if function == "coercion":
        values = input_value.get("values")
        if not isinstance(values, list):
            raise ValueError("values must be a list")
        return [coerce(value) for value in values]
    if function == "time":
        values = input_value.get("values")
        if not isinstance(values, list):
            raise ValueError("values must be a list")
        return [convert_time(value) for value in values]
    raise ValueError(f"unknown b3892 function {function}")


def convert_time(value: Json) -> Json:
    if isinstance(value, bool):
        return {"seconds": None, "nanoseconds": None}
    if isinstance(value, (int, float)):
        return {"seconds": to_seconds(value), "nanoseconds": to_ns(value)}
    if isinstance(value, str):
        return {"seconds": to_seconds(value), "nanoseconds": None}
    return {"seconds": None, "nanoseconds": None}


def coerce(item: Json) -> Json:
    if not isinstance(item, dict):
        raise ValueError("each coercion must be an object")
    kind = item.get("kind")
    value = item.get("value")
    if kind == "string":
        return as_str(value)
    if kind == "integer":
        return as_int(value)
    if kind == "decimal":
        result = as_float(value)
        return None if result is None else str(result)
    if kind == "boolean":
        return as_bool(value)
    if kind == "mapping":
        result = as_str_mapping(value)
        return None if not isinstance(result, Mapping) else cast("dict[str, Json]", dict(result))
    if kind == "tuple":
        result = as_str_tuple(value)
        return None if result is None else list(result)
    raise ValueError(f"unknown coercion kind {kind}")
