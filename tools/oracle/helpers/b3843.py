from collections.abc import Callable
from typing import TypeAlias, cast

import litellm
from litellm.integrations.otel.model import payloads

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def run(input_value: dict[str, Json]) -> Json:
    function = input_value.get("function")
    value = input_value.get("value")
    if function == "cache_token_value":
        operation_name = "_cache_token_value"
        operation = cast(Callable[..., Json], getattr(payloads, operation_name))
        return operation(*cast(list[object], input_value.get("values", [])))
    if function == "guardrail_mode_str":
        operation_name = "_guardrail_mode_str"
        operation = cast(Callable[..., Json], getattr(payloads, operation_name))
        return operation(value)
    if function == "total_masked_entities":
        operation_name = "_total_masked_entities"
        operation = cast(Callable[..., Json], getattr(payloads, operation_name))
        return operation(value)
    if function == "dicts":
        operation_name = "_dicts"
        operation = cast(Callable[..., Json], getattr(payloads, operation_name))
        return list(cast(list[Json], operation(value)))
    if function == "json_or_none":
        operation_name = "_json_or_none"
        operation = cast(Callable[..., Json], getattr(payloads, operation_name))
        return operation(value)
    if function == "sanitize_event_metadata":
        operation_name = "sanitize_event_metadata"
        operation = cast(Callable[..., Json], getattr(payloads, operation_name))
        return operation(value)
    raise ValueError(f"unknown LiteLLM helper function {function} ({litellm.__name__})")
