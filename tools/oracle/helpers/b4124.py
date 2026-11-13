import copy
import os
from collections.abc import Callable, Sequence
from typing import TYPE_CHECKING, TypeAlias, cast
from unittest.mock import patch

import litellm
from litellm.integrations.anthropic_cache_control_hook import AnthropicCacheControlHook
from litellm.types.integrations.anthropic_cache_control_hook import (
    CacheControlInjectionPoint,
    CacheControlMessageInjectionPoint,
)

if TYPE_CHECKING:
    from litellm.types.llms.openai import AllMessageValues
    from litellm.types.utils import StandardCallbackDynamicParams

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Params: TypeAlias = dict[str, Json]
Point = CacheControlInjectionPoint
MessagePoint = CacheControlMessageInjectionPoint


def object_of(value: Json, name: str) -> Params:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def list_of(value: Json, name: str) -> list[Json]:
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return value


def string_of(fields: Params, name: str) -> str:
    value = fields.get(name)
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def optional_string(fields: Params, name: str) -> str | None:
    value = fields.get(name)
    return value if isinstance(value, str) else None


def integer_of(fields: Params, name: str, default: int) -> int:
    value = fields.get(name)
    if value is None:
        return default
    if isinstance(value, bool) or not isinstance(value, int):
        raise ValueError(f"{name} must be an integer")
    return value


def flag_of(fields: Params, name: str, *, default: bool) -> bool:
    value = fields.get(name)
    if value is None:
        return default
    if not isinstance(value, bool):
        raise ValueError(f"{name} must be a boolean")
    return value


def model_cost_of(fields: Params) -> Params:
    return object_of(fields.get("model_cost", {}), "model_cost")


def environment_of(fields: Params) -> dict[str, str]:
    return cast("dict[str, str]", object_of(fields.get("environment", {}), "environment"))


def point_json(value: object) -> Json:
    point = object_of(cast(Json, value), "injection point")
    location = point.get("location")
    normalized: dict[str, Json] = {"location": location, "control": point.get("control")}
    if location == "message":
        normalized["role"] = point.get("role")
        normalized["index"] = point.get("index")
    elif location != "tool_config":
        raise ValueError(f"unknown injection point location: {location!r}")
    for stamp in ("_litellm_openai_dialect", "_litellm_external_breakpoints"):
        if stamp in point:
            normalized[stamp] = point[stamp]
    return normalized


def normalized_params(params: Params) -> Json:
    result = dict(params)
    points = result.get("cache_control_injection_points")
    if isinstance(points, list):
        result["cache_control_injection_points"] = [point_json(point) for point in points]
    return result


def message_injections(fields: Params) -> Json:
    results: list[Json] = []
    for value in list_of(fields["probes"], "probes"):
        probe = object_of(value, "probe")
        conversation = list_of(probe.get("messages", []), "messages")
        points = list_of(probe.get("points", []), "points")
        max_blocks = integer_of(probe, "max_blocks", 4)
        openai_dialect = flag_of(probe, "openai_dialect", default=False)
        processed = AnthropicCacheControlHook._apply_message_injections(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            points=cast("Sequence[MessagePoint]", points),
            messages=cast("list[AllMessageValues]", copy.deepcopy(conversation)),
            max_blocks=max_blocks,
            openai_dialect=openai_dialect,
        )
        results.append({"messages": cast(Json, processed)})
    return {"probes": results}


def messages_route(fields: Params) -> Json:
    results: list[Json] = []
    for value in list_of(fields["probes"], "probes"):
        probe = object_of(value, "probe")
        conversation = list_of(probe.get("messages", []), "messages")
        system = probe.get("system")
        if system is not None and not isinstance(system, str | list):
            raise ValueError("system must be a string or a list")
        points = list_of(probe.get("points", []), "points")
        openai_dialect = flag_of(probe, "openai_dialect", default=False)
        external = integer_of(probe, "external_breakpoints", 0)
        processed, processed_system, remaining = cast(
            "tuple[list[dict[object, object]], str | list[object] | None, list[Point]]",
            AnthropicCacheControlHook.apply_to_anthropic_messages_request(  # pyright: ignore[reportUnknownMemberType]
                messages=cast("list[dict[object, object]]", copy.deepcopy(conversation)),
                system=cast("str | list[object] | None", system),
                injection_points=cast("Sequence[Point]", points),
                openai_dialect=openai_dialect,
                external_breakpoints=external,
            ),
        )
        results.append(
            {
                "messages": cast(Json, processed),
                "system": cast(Json, processed_system),
                "remaining": [point_json(point) for point in remaining],
            }
        )
    return {"probes": results}


def chat_injections(fields: Params) -> Json:
    results: list[Json] = []
    for value in list_of(fields["probes"], "probes"):
        probe = object_of(value, "probe")
        model = string_of(probe, "model")
        conversation = list_of(probe.get("messages", []), "messages")
        params = object_of(probe.get("params", {}), "params")
        environment = environment_of(probe)
        global_api_base = optional_string(probe, "global_api_base")
        with (
            patch.object(litellm, "model_cost", model_cost_of(probe)),
            patch.object(litellm, "api_base", global_api_base),
            patch.dict(os.environ),
        ):
            for name in ("OPENAI_BASE_URL", "OPENAI_API_BASE"):
                os.environ.pop(name, None)
            os.environ.update(environment)
            kwargs = copy.deepcopy(params)
            _, processed, returned = cast(
                "tuple[str, list[AllMessageValues], Params]",
                AnthropicCacheControlHook().get_chat_completion_prompt(  # pyright: ignore[reportUnknownMemberType]
                    model=model,
                    messages=cast("list[AllMessageValues]", copy.deepcopy(conversation)),
                    non_default_params=kwargs,
                    prompt_id=None,
                    prompt_variables=None,
                    dynamic_callback_params=cast("StandardCallbackDynamicParams", {}),
                ),
            )
        results.append(
            {
                "messages": cast(Json, processed),
                "params": normalized_params(returned),
            }
        )
    return {"probes": results}


HANDLERS: dict[str, Callable[[Params], Json]] = {
    "chat_injections": chat_injections,
    "message_injections": message_injections,
    "messages_route": messages_route,
}


def run(input_value: Params) -> Json:
    operation = string_of(input_value, "operation")
    handler = HANDLERS.get(operation)
    if handler is None:
        raise ValueError(f"unknown operation: {operation}")
    return handler(input_value)
