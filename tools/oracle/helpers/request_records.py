import copy
from typing import TypeAlias, cast

import litellm
from litellm.integrations.anthropic_cache_control_hook import AnthropicCacheControlHook
from litellm.litellm_core_utils.core_helpers import (
    filter_exceptions_from_params,  # pyright: ignore[reportUnknownVariableType]
    filter_internal_params,  # pyright: ignore[reportUnknownVariableType]
    independent_snapshot,  # pyright: ignore[reportUnknownVariableType]
    safe_deep_copy,  # pyright: ignore[reportUnknownVariableType]
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def object_of(value: Json) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise TypeError("expected an object")
    return value


def integer(value: Json) -> int:
    if isinstance(value, (int, float, str)):
        return int(value)
    raise TypeError("expected an integer")


def run(input_value: dict[str, Json]) -> Json:
    function = input_value["function"]
    record = input_value["record"]
    if function == "safe_copy":
        litellm.safe_memory_mode = input_value["safe_memory_mode"] is True
        return cast(Json, safe_deep_copy(record))
    if function == "independent_snapshot":
        return cast(Json, independent_snapshot(object_of(record)))
    if function == "filter_parameters":
        depth = integer(input_value.get("max_depth", 20))
        return cast(Json, filter_exceptions_from_params(record, depth))
    if function == "filter_internal":
        additional = input_value["additional"]
        names: set[str] = set()
        if isinstance(additional, list):
            names = {name for name in additional if isinstance(name, str)}
        return cast(Json, filter_internal_params(object_of(record), names))
    if function == "gateway_injection":
        changed = object_of(copy.deepcopy(record))
        raw_steps = input_value.get("steps")
        if not isinstance(raw_steps, list):
            raise TypeError("expected gateway steps")
        for raw_step in raw_steps:
            step = object_of(raw_step)
            changed["model_info"] = step["model_info"]
            AnthropicCacheControlHook.record_gateway_injection(
                changed, integer(step["added"]), step["every_deployment"] is True
            )
        return changed
    raise ValueError(f"unknown request_records function: {function}")
