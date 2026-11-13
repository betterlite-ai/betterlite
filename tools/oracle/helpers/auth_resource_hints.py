from collections.abc import Callable
from typing import TypeAlias, cast
from unittest.mock import patch

from litellm._logging import verbose_logger  # pyright: ignore[reportPrivateUsage]
from litellm.llms.azure.passthrough.transformation import azure_router_model_in_endpoint
from litellm.llms.nvidia_nim.passthrough.transformation import (
    is_nvidia_nim_deployment,
    nvidia_nim_model_group_in_path,
    nvidia_nim_model_groups,
    nvidia_nim_router_model_in_endpoint,
)
from litellm.proxy.pass_through_endpoints.llm_passthrough_endpoints import (  # pyright: ignore[reportMissingTypeStubs]
    _extract_model_from_bedrock_endpoint,  # pyright: ignore[reportPrivateUsage]
    is_bedrock_count_tokens_endpoint,
)
from litellm.types.router import DeploymentTypedDict
from litellm.types.videos.utils import (
    decode_character_id_with_provider,
    decode_video_id_with_provider,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Deployment = dict[str, Json]


def run(input_value: dict[str, Json]) -> Json:
    steps = input_value.get("steps")
    if not isinstance(steps, list):
        raise ValueError("steps must be a list")
    return [run_step(step) for step in cast(list[Json], steps)]


def run_step(step: Json) -> Json:  # noqa: C901, PLR0911
    if not isinstance(step, dict):
        raise ValueError("a step must be an object")
    kind = step.get("kind")
    if kind == "video_id":
        return managed_step(step, decode_video_id_with_provider, "video_id")
    if kind == "character_id":
        return managed_step(step, decode_character_id_with_provider, "character_id")
    if kind == "azure":
        return azure_step(step)
    if kind == "nim_deployment":
        return nim_deployment_step(step)
    if kind == "nim_groups":
        groups = nvidia_nim_model_groups(declarations(step))
        return ok_value({"groups": cast(Json, sorted(groups))})
    if kind == "nim_path":
        model = nvidia_nim_model_group_in_path(
            cast(str, step.get("path")), declarations(step)
        )
        return ok_value({"model": model})
    if kind == "nim_endpoint":
        model = nvidia_nim_router_model_in_endpoint(
            cast(str, step.get("endpoint")), strings(step.get("router_models"))
        )
        return ok_value({"model": model})
    if kind == "bedrock_count":
        endpoint = cast(str, step.get("endpoint"))
        return ok_value({"count_endpoint": is_bedrock_count_tokens_endpoint(endpoint)})
    if kind == "bedrock_model":
        return bedrock_model_step(cast(str, step.get("endpoint")))
    raise ValueError(f"unknown step kind: {kind}")


def managed_step(step: dict[str, Json], decode: Callable[[str], object], id_key: str) -> Json:
    encoded = cast(str, step.get("encoded"))
    calls: list[str] = []

    def capture(message: str, *_args: object, **_kwargs: object) -> None:
        calls.append(message)

    with patch.object(verbose_logger, "debug", capture):
        decoded = decode(encoded)
    fields = cast(dict[str, object], decoded)
    value: dict[str, Json] = {name: cast(Json, fields.get(name)) for name in (
        "custom_llm_provider",
        "model_id",
        id_key,
    )}
    return ok_value({"decoded": value, "debug": len(calls)})


def azure_step(step: dict[str, Json]) -> Json:
    model = azure_router_model_in_endpoint(
        cast(str, step.get("endpoint")), strings(step.get("router_models"))
    )
    return ok_value({"model": model})


def nim_deployment_step(step: dict[str, Json]) -> Json:
    declaration = deployment(step)
    return ok_value({"is_nim": is_nvidia_nim_deployment(declaration)})


def bedrock_model_step(endpoint: str) -> Json:
    try:
        model = _extract_model_from_bedrock_endpoint(endpoint)
    except ValueError:
        return {"ok": False}
    return ok_value({"model": model})


def declarations(step: dict[str, Json]) -> list[DeploymentTypedDict]:
    entries = cast(list[Json], step.get("deployments"))
    return [deployment(cast(dict[str, Json], entry)) for entry in entries]


def deployment(entry: dict[str, Json]) -> DeploymentTypedDict:
    params: dict[str, Json] = {}
    provider = entry.get("custom_llm_provider")
    if provider is not None:
        params["custom_llm_provider"] = provider
    model = entry.get("model")
    if model is not None:
        params["model"] = model
    return cast(
        DeploymentTypedDict,
        {"model_name": entry.get("model_name", ""), "litellm_params": params},
    )


def strings(value: Json) -> list[str]:
    return cast(list[str], value)


def ok_value(value: dict[str, Json]) -> Json:
    return {"ok": True, "value": value}
