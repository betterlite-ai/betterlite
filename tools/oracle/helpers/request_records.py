import copy
import threading
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


class _Uncopyable:
    def __init__(self) -> None:
        self.lock = threading.Lock()

    def __deepcopy__(self, memo: dict[int, object]) -> "_Uncopyable":
        raise TypeError("uncopyable fixture")


Runtime: TypeAlias = (
    None | bool | int | float | str | list["Runtime"] | dict[str, "Runtime"] | _Uncopyable
)


def materialize_uncopyable(value: Json) -> Runtime:
    if isinstance(value, dict):
        return {key: materialize_uncopyable(item) for key, item in value.items()}
    if isinstance(value, list):
        return [materialize_uncopyable(item) for item in value]
    if value == "__uncopyable__":
        return _Uncopyable()
    return value


def normalize_uncopyable(value: Runtime) -> Json:
    if isinstance(value, _Uncopyable):
        return "__uncopyable__"
    if isinstance(value, dict):
        typed_value = cast(dict[str, Runtime], value)
        return {key: normalize_uncopyable(item) for key, item in typed_value.items()}
    if isinstance(value, list):
        typed_value = cast(list[Runtime], value)
        return [normalize_uncopyable(item) for item in typed_value]
    return cast(Json, value)


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
        materialized = materialize_uncopyable(record)
        copied = cast(Runtime, safe_deep_copy(materialized))
        return normalize_uncopyable(copied)
    if function == "independent_snapshot":
        return cast(Json, independent_snapshot(cast(dict[str, Json], record)))
    if function == "filter_parameters":
        depth = integer(input_value.get("max_depth", 20))
        return cast(Json, filter_exceptions_from_params(record, depth))
    if function == "filter_internal":
        additional = input_value["additional"]
        names: set[str] = set()
        if isinstance(additional, list):
            names = {name for name in additional if isinstance(name, str)}
        return cast(Json, filter_internal_params(cast(dict[str, Json], record), names))
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
