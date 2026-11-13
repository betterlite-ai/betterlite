import asyncio
import copy
import os
from collections.abc import Callable
from typing import TYPE_CHECKING, TypeAlias, cast
from unittest.mock import patch

import litellm
from litellm.integrations.anthropic_cache_control_hook import AnthropicCacheControlHook
from litellm.integrations.prompt_management_base import PromptManagementClient
from litellm.types.llms.openai import AllMessageValues
from litellm.types.prompts.init_prompts import PromptSpec

if TYPE_CHECKING:
    from litellm.litellm_core_utils.litellm_logging import Logging as LiteLLMLoggingObj
    from litellm.types.utils import StandardCallbackDynamicParams

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Params: TypeAlias = dict[str, Json]

SENTINEL = "custom-logger"


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


def optional_object(fields: Params, name: str) -> Params | None:
    value = fields.get(name)
    if value is None:
        return None
    return object_of(value, name)


def optional_integer(fields: Params, name: str) -> int | None:
    value = fields.get(name)
    if value is None:
        return None
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


def prompt_spec_of(fields: Params) -> PromptSpec | None:
    raw = fields.get("prompt_spec")
    if raw is None:
        return None
    return PromptSpec.model_validate(object_of(raw, "prompt_spec"))


def dynamic_of(fields: Params) -> dict[str, Json]:
    return object_of(fields.get("dynamic_callback_params", {}), "dynamic_callback_params")


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


def client_json(client: PromptManagementClient) -> Json:
    return {
        "prompt_id": client["prompt_id"],
        "messages": cast(Json, client["prompt_template"]),
        "model": client["prompt_template_model"],
        "optional_params": cast(Json, client["prompt_template_optional_params"]),
        "completed_messages": cast(Json, client["completed_messages"]),
    }


def integration_name(_fields: Params) -> Json:
    return {"name": AnthropicCacheControlHook().integration_name}


def prompt_management_gate(fields: Params) -> Json:
    results: list[Json] = []
    for value in list_of(fields["probes"], "probes"):
        probe = object_of(value, "probe")
        spec = prompt_spec_of(probe)
        dynamic = cast("StandardCallbackDynamicParams", dynamic_of(probe))
        results.append(
            cast(
                Json,
                AnthropicCacheControlHook().should_run_prompt_management(
                    prompt_id=optional_string(probe, "prompt_id"),
                    prompt_spec=spec,
                    dynamic_callback_params=dynamic,
                ),
            )
        )
    return {"results": results}


def compile_sync_client(probe: Params) -> PromptManagementClient:
    return AnthropicCacheControlHook()._compile_prompt_helper(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
        prompt_id=optional_string(probe, "prompt_id"),
        prompt_spec=prompt_spec_of(probe),
        prompt_variables=optional_object(probe, "prompt_variables"),
        dynamic_callback_params=cast("StandardCallbackDynamicParams", dynamic_of(probe)),
        prompt_label=optional_string(probe, "prompt_label"),
        prompt_version=optional_integer(probe, "prompt_version"),
    )


def compile_result_empty(fields: Params) -> Json:
    probes = [object_of(value, "probe") for value in list_of(fields["probes"], "probes")]
    return {"clients": [client_json(compile_sync_client(probe)) for probe in probes]}


def compile_async_client(probe: Params) -> PromptManagementClient:
    return asyncio.run(
        AnthropicCacheControlHook().async_compile_prompt_helper(  # pyright: ignore[reportUnknownMemberType]
            prompt_id=optional_string(probe, "prompt_id"),
            prompt_variables=optional_object(probe, "prompt_variables"),
            dynamic_callback_params=cast("StandardCallbackDynamicParams", dynamic_of(probe)),
            prompt_spec=prompt_spec_of(probe),
            prompt_label=optional_string(probe, "prompt_label"),
            prompt_version=optional_integer(probe, "prompt_version"),
        )
    )


def async_parity(fields: Params) -> Json:
    clients: list[Json] = []
    for value in list_of(fields["probes"], "probes"):
        probe = object_of(value, "probe")
        clients.append(
            {
                "sync": client_json(compile_sync_client(probe)),
                "async": client_json(compile_async_client(probe)),
            }
        )
    return {"clients": clients}


def cache_control_check(fields: Params) -> Json:
    results: list[Json] = []
    for value in list_of(fields["probes"], "probes"):
        probe = object_of(value, "probe")
        params = object_of(probe.get("params", {}), "params")
        results.append(
            AnthropicCacheControlHook.should_use_anthropic_cache_control_hook(  # pyright: ignore[reportUnknownMemberType]
                params
            )
        )
    return {"results": results}


class LoggerFactory:
    def __init__(self, *, build: bool) -> None:
        self.build = build
        self.calls: list[str] = []

    def init(
        self, *, logging_integration: str, internal_usage_cache: object, llm_router: object
    ) -> object:
        _ = internal_usage_cache
        _ = llm_router
        self.calls.append(logging_integration)
        return SENTINEL if self.build else None


def logger_factory(fields: Params) -> Json:
    results: list[Json] = []
    for value in list_of(fields["probes"], "probes"):
        probe = object_of(value, "probe")
        params = object_of(probe.get("params", {}), "params")
        factory = LoggerFactory(build=flag_of(probe, "logger_from_factory", default=True))
        with patch(
            "litellm.litellm_core_utils.litellm_logging._init_custom_logger_compatible_class",
            factory.init,
        ):
            logger = AnthropicCacheControlHook.get_custom_logger_for_anthropic_cache_control_hook(  # pyright: ignore[reportUnknownMemberType]
                params
            )
        results.append(
            {
                "called": bool(factory.calls),
                "integration": factory.calls[0] if factory.calls else None,
                "logger": SENTINEL if logger is not None else None,
            }
        )
    return {"results": results}


def run_async_hook(probe: Params) -> tuple[str, list[AllMessageValues], Params]:
    model = string_of(probe, "model")
    conversation = list_of(probe.get("messages", []), "messages")
    params = object_of(probe.get("params", {}), "params")
    environment = environment_of(probe)
    with (
        patch.object(litellm, "model_cost", model_cost_of(probe)),
        patch.object(litellm, "api_base", optional_string(probe, "global_api_base")),
        patch.dict(os.environ),
    ):
        for name in ("OPENAI_BASE_URL", "OPENAI_API_BASE"):
            os.environ.pop(name, None)
        os.environ.update(environment)
        return cast(
            "tuple[str, list[AllMessageValues], Params]",
            asyncio.run(
                AnthropicCacheControlHook().async_get_chat_completion_prompt(  # pyright: ignore[reportUnknownMemberType, reportUnknownArgumentType]
                    model=model,
                    messages=cast("list[AllMessageValues]", copy.deepcopy(conversation)),
                    non_default_params=copy.deepcopy(params),
                    prompt_id=optional_string(probe, "prompt_id"),
                    prompt_variables=optional_object(probe, "prompt_variables"),
                    dynamic_callback_params=cast(
                        "StandardCallbackDynamicParams", dynamic_of(probe)
                    ),
                    litellm_logging_obj=cast("LiteLLMLoggingObj", None),
                    prompt_spec=prompt_spec_of(probe),
                    tools=cast("list[dict[str, Json]] | None", probe.get("tools")),
                    prompt_label=optional_string(probe, "prompt_label"),
                    prompt_version=optional_integer(probe, "prompt_version"),
                    ignore_prompt_manager_model=flag_of(
                        probe, "ignore_model", default=False
                    ),
                    ignore_prompt_manager_optional_params=flag_of(
                        probe, "ignore_optional_params", default=False
                    ),
                )
            ),
        )


def delegation_to_placement(fields: Params) -> Json:
    results: list[Json] = []
    for value in list_of(fields["probes"], "probes"):
        model, messages, params = run_async_hook(object_of(value, "probe"))
        results.append(
            {
                "model": model,
                "messages": cast(Json, messages),
                "params": normalized_params(params),
            }
        )
    return {"results": results}


HANDLERS: dict[str, Callable[[Params], Json]] = {
    "integration_name": integration_name,
    "prompt_management_gate": prompt_management_gate,
    "compile_result_empty": compile_result_empty,
    "async_parity": async_parity,
    "cache_control_check": cache_control_check,
    "logger_factory": logger_factory,
    "delegation_to_placement": delegation_to_placement,
}


def run(input_value: Params) -> Json:
    operation = string_of(input_value, "operation")
    handler = HANDLERS.get(operation)
    if handler is None:
        raise ValueError(f"unknown operation: {operation}")
    return handler(input_value)
