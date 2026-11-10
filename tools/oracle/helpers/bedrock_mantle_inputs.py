import os

# LiteLLM reads LITELLM_RUST per call; false pins the Python fallback the oracle compares against.
os.environ["LITELLM_RUST"] = "false"

from collections.abc import Callable
from typing import TypeAlias, cast
from unittest.mock import patch

import litellm
from litellm.llms.bedrock_mantle.chat.transformation import BedrockMantleChatConfig
from litellm.llms.bedrock_mantle.common_utils import (
    BedrockMantleAuthMixin,
    mantle_base_segment,  # pyright: ignore[reportUnknownVariableType]
    mantle_supports_responses,  # pyright: ignore[reportUnknownVariableType]
    resolve_mantle_bearer_token,
    resolve_mantle_region,
    split_mantle_region_prefix,
)
from litellm.types.router import GenericLiteLLMParams

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Params: TypeAlias = dict[str, Json]


def object_of(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def list_of(value: Json, name: str) -> list[Json]:
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return value


def environment_of(fields: dict[str, Json]) -> dict[str, str]:
    return cast("dict[str, str]", object_of(fields.get("environment", {}), "environment"))


def optional_string(fields: dict[str, Json], name: str) -> str | None:
    value = fields.get(name)
    return value if isinstance(value, str) else None


def bearer_precedence(params: Params) -> Json:
    tokens: list[Json] = []
    for case in list_of(params["cases"], "cases"):
        fields = object_of(case, "case")
        with patch.dict(os.environ, environment_of(fields), clear=True):
            api_key = optional_string(fields, "api_key")
            resolved = resolve_mantle_bearer_token(api_key)
            delegated = BedrockMantleAuthMixin._resolve_bearer_token(api_key)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        if delegated != resolved:
            raise ValueError("the auth mixin resolved a different bearer token")
        tokens.append(resolved)
    return {"tokens": tokens}


def region_prefix(params: Params) -> Json:
    prefixes: list[Json] = []
    for model in list_of(params["models"], "models"):
        if not isinstance(model, str):
            raise ValueError("models must be strings")
        region, base = split_mantle_region_prefix(model)
        prefixes.append([region, base])
    return {"prefixes": prefixes}


def region_resolution(params: Params) -> Json:
    regions: list[Json] = []
    for case in list_of(params["cases"], "cases"):
        fields = object_of(case, "case")
        mapping = {
            "aws_region_name": optional_string(fields, "aws_region_name"),
            "api_base": optional_string(fields, "api_base"),
        }
        with patch.dict(os.environ, environment_of(fields), clear=True):
            try:
                region = resolve_mantle_region(mapping)
                delegated = BedrockMantleAuthMixin._resolve_region(mapping)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
            except ValueError:
                regions.append({"error": True})
                continue
        if delegated != region:
            raise ValueError("the auth mixin resolved a different region")
        regions.append({"region": region})
    return {"regions": regions}


def model_capabilities(params: Params) -> Json:
    model_cost = object_of(params["model_cost"], "model_cost")
    capabilities: list[Json] = []
    for value in list_of(params["models"], "models"):
        model = value if isinstance(value, str) else None
        capabilities.append(
            {
                "supports_responses": mantle_supports_responses(model, model_cost),
                "base_segment": mantle_base_segment(model, model_cost),
            }
        )
    return {"capabilities": capabilities}


def chat_endpoint_inputs(params: Params) -> Json:
    model_cost = object_of(params["model_cost"], "model_cost")
    config = BedrockMantleChatConfig()
    inputs: list[Json] = []
    for case in list_of(params["cases"], "cases"):
        fields = object_of(case, "case")
        litellm_params = GenericLiteLLMParams(
            aws_region_name=optional_string(fields, "aws_region_name")
        )
        with (
            patch.dict(os.environ, environment_of(fields), clear=True),
            patch.object(litellm, "model_cost", model_cost),
        ):
            try:
                api_base, api_key = config._get_openai_compatible_provider_info(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                    optional_string(fields, "api_base"),
                    optional_string(fields, "api_key"),
                    litellm_params,
                    optional_string(fields, "model"),
                )
            except ValueError:
                inputs.append({"error": True})
                continue
        inputs.append({"api_base": api_base, "api_key": api_key})
    return {"inputs": inputs}


HANDLERS: dict[str, Callable[[Params], Json]] = {
    "mantle_bearer_precedence": bearer_precedence,
    "mantle_region_prefix": region_prefix,
    "mantle_region_resolution": region_resolution,
    "mantle_model_capabilities": model_capabilities,
    "mantle_chat_endpoint_inputs": chat_endpoint_inputs,
}


def run(input_value: dict[str, Json]) -> Json:
    if len(input_value) != 1:
        raise ValueError("bedrock_mantle_inputs takes exactly one operation")
    operation, payload = next(iter(input_value.items()))
    handler = HANDLERS.get(operation)
    if handler is None:
        raise ValueError(f"unknown operation: {operation}")
    _ = litellm
    return handler(cast("Params", payload))
