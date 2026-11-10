import os
from collections.abc import Callable, Iterable, Sequence
from typing import TYPE_CHECKING, TypeAlias, cast
from unittest.mock import patch

import litellm
from litellm.integrations import anthropic_cache_control_hook as hook
from litellm.integrations.anthropic_cache_control_hook import (
    AnthropicCacheControlHook,
    supports_openai_prompt_cache_breakpoint,
)
from litellm.types.integrations.anthropic_cache_control_hook import (
    CacheControlInjectionPoint,
)

if TYPE_CHECKING:
    from litellm.types.llms.openai import AllMessageValues

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Params: TypeAlias = dict[str, Json]
Point = CacheControlInjectionPoint


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


def version_of(model: str) -> Json:
    matched = hook._GPT_VERSION_PATTERN.match(model.rsplit("/", 1)[-1].lower())  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    if matched is None:
        return None
    return [int(matched.group(1)), int(matched.group(2) or 0)]


def gpt_versions(fields: Params) -> Json:
    versions: list[Json] = []
    supports: list[Json] = []
    no_model_cost: dict[str, Json] = {}
    with patch.object(litellm, "model_cost", no_model_cost):
        for value in list_of(fields["models"], "models"):
            if not isinstance(value, str):
                raise ValueError("models must be strings")
            versions.append(version_of(value))
            supports.append(supports_openai_prompt_cache_breakpoint(value))
    return {"versions": versions, "supports": supports}


def model_map(fields: Params) -> Json:
    flags: list[Json] = []
    supports: list[Json] = []
    for value in list_of(fields["probes"], "probes"):
        probe = object_of(value, "probe")
        model = string_of(probe, "model")
        with patch.object(litellm, "model_cost", model_cost_of(probe)):
            flags.append(hook._model_map_prompt_cache_breakpoint_flag(model))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            supports.append(supports_openai_prompt_cache_breakpoint(model))
    return {"flags": flags, "supports": supports}


def targets_api(probe: Params) -> Json:
    api_base = optional_string(probe, "api_base") or optional_string(probe, "base_url")
    environment = environment_of(probe)
    with patch.dict(os.environ), patch.object(
        litellm, "api_base", optional_string(probe, "global_api_base")
    ):
        for name in ("OPENAI_BASE_URL", "OPENAI_API_BASE"):
            os.environ.pop(name, None)
        os.environ.update(environment)
        return hook.targets_openai_api(api_base)


def openai_bases(fields: Params) -> Json:
    probes: list[Json] = []
    for value in list_of(fields["probes"], "probes"):
        probe = object_of(value, "probe")
        targets_breakpoint: Json = None
        if "model" in probe:
            api_base = optional_string(probe, "api_base") or optional_string(probe, "base_url")
            with patch.object(litellm, "model_cost", model_cost_of(probe)):
                targets_breakpoint = (
                    AnthropicCacheControlHook._targets_openai_prompt_cache_breakpoint(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                        string_of(probe, "model"),
                        optional_string(probe, "custom_llm_provider"),
                        api_base,
                        probe.get("prompt_cache_options"),
                    )
                )
        probes.append(
            {"targets_api": targets_api(probe), "targets_breakpoint": targets_breakpoint}
        )
    return {"probes": probes}


def carrier_detection(fields: Params) -> Json:
    blocks: list[Json] = [
        {
            "carries": hook._carries_cache_breakpoint(value),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            "accepts": hook._accepts_prompt_cache_breakpoint(value),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        }
        for value in list_of(fields.get("blocks", []), "blocks")
    ]
    tools: list[Json] = [
        {
            "carries": hook._tool_carries_cache_breakpoint(value),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            "drops": hook._chat_transform_drops_tool_cache_control(value),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        }
        for value in list_of(fields.get("tools", []), "tools")
    ]
    return {"blocks": blocks, "tools": tools}


def request_extraction(fields: Params) -> Json:
    probes: list[Json] = []
    for value in list_of(fields["probes"], "probes"):
        probe = object_of(value, "probe")
        request_kwargs = probe.get("request_kwargs")
        probes.append(
            {
                "user_agent": AnthropicCacheControlHook._request_user_agent(request_kwargs),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                "system": cast(
                    Json,
                    AnthropicCacheControlHook._request_system(request_kwargs),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                ),
            }
        )
    return {"probes": probes}


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


def breakpoint_counting(fields: Params) -> Json:
    probes: list[Json] = []
    for value in list_of(fields["probes"], "probes"):
        probe = object_of(value, "probe")
        messages = list_of(probe.get("messages", []), "messages")
        system = probe.get("system")
        tools = probe.get("tools")
        cache_control = probe.get("cache_control")
        request_kwargs = probe.get("request_kwargs")
        points = cast("Sequence[Point]", list_of(probe.get("points", []), "points"))
        external = integer_of(probe, "external_breakpoints", 0)
        openai_dialect = flag_of(probe, "openai_dialect", default=False)
        on_wire = integer_of(probe, "breakpoints_on_wire", 0)
        on_route = flag_of(probe, "on_messages_route", default=False)
        typed_messages = cast("Iterable[object]", messages)
        typed_tools = cast("Iterable[object] | None", tools)
        probes.append(
            {
                "request_breakpoints": AnthropicCacheControlHook.count_request_cache_breakpoints(
                    typed_messages, system
                ),
                "external_breakpoints": AnthropicCacheControlHook.count_external_cache_breakpoints(
                    typed_tools, cache_control, request_kwargs
                ),
                "route_breakpoints": (
                    AnthropicCacheControlHook.count_external_cache_breakpoints_on_messages_route(
                        typed_tools, cache_control, request_kwargs
                    )
                ),
                "reserved_blocks": AnthropicCacheControlHook._blocks_reserved_outside_messages(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                    points, external, openai_dialect
                ),
                "points_with_slot": [
                    point_json(point)
                    for point in AnthropicCacheControlHook._points_with_a_slot_left(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                        points, on_wire, openai_dialect
                    )
                ],
                "message_blocks": [
                    AnthropicCacheControlHook._count_cache_control_blocks(message)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                    for message in messages
                ],
                "message_has_control": [
                    AnthropicCacheControlHook._message_has_cache_control(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                        cast("AllMessageValues", message)
                    )
                    for message in messages
                ],
                "request_has_cache_control": AnthropicCacheControlHook._request_has_cache_control(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
                    cast("list[AllMessageValues]", messages),
                    cast("str | list[object] | None", system),
                    cast("list[object] | None", tools),
                    cache_control,
                    request_kwargs,
                    on_route,
                ),
            }
        )
    return {"probes": probes}


def default_controls(fields: Params) -> Json:
    controls: list[Json] = []
    for value in list_of(fields["settings"], "settings"):
        ttl = value if isinstance(value, str) else None
        with patch.object(litellm, "anthropic_prompt_caching_ttl", ttl):
            controls.append(
                cast(Json, AnthropicCacheControlHook._default_control())  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            )
    return {"controls": controls}


def stamping(fields: Params) -> Json:
    probes: list[Json] = []
    for value in list_of(fields["probes"], "probes"):
        probe = object_of(value, "probe")
        points = cast("Sequence[Point]", list_of(probe["points"], "points"))
        model = string_of(probe, "model")
        provider = optional_string(probe, "custom_llm_provider")
        api_base = optional_string(probe, "api_base") or optional_string(probe, "base_url")
        options = probe.get("prompt_cache_options")
        with patch.object(litellm, "model_cost", model_cost_of(probe)):
            with_dialect = AnthropicCacheControlHook._stamped_with_dialect(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                points, model, provider, api_base, options
            )
            for_hook = AnthropicCacheControlHook._stamped_for_prompt_hook(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                points,
                integer_of(probe, "external_breakpoints", 0),
                model,
                provider,
                api_base,
                options,
            )
        probes.append(
            {
                "for_prompt_hook": [point_json(point) for point in for_hook],
                "with_dialect": [point_json(point) for point in with_dialect],
            }
        )
    return {"probes": probes}


HANDLERS: dict[str, Callable[[Params], Json]] = {
    "breakpoint_counting": breakpoint_counting,
    "carrier_detection": carrier_detection,
    "default_control": default_controls,
    "gpt_version": gpt_versions,
    "model_map": model_map,
    "openai_bases": openai_bases,
    "request_extraction": request_extraction,
    "stamping": stamping,
}


def run(input_value: Params) -> Json:
    operation = string_of(input_value, "operation")
    handler = HANDLERS.get(operation)
    if handler is None:
        raise ValueError(f"unknown operation: {operation}")
    return handler(input_value)
