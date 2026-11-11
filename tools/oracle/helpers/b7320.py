import os
from contextlib import ExitStack
from typing import TypeAlias
from unittest import mock

import litellm
from litellm.litellm_core_utils.llm_response_utils import get_api_base as api_base_module
from litellm.litellm_core_utils.llm_response_utils.get_api_base import (
    get_api_base,  # pyright: ignore[reportUnknownVariableType]  # optional_params: dict[Unknown, Unknown]
)
from litellm.types.router import LiteLLM_Params

# get_llm_provider prints the provider list on stdout when an unknown model cannot be resolved.
litellm.suppress_debug_info = True

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def _text(data: dict[str, Json], name: str) -> str | None:
    value = data.get(name)
    if value is None:
        return None
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def _object(data: dict[str, Json], name: str) -> dict[str, Json]:
    value = data.get(name)
    if value is None:
        return {}
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def _string_map(data: dict[str, Json], name: str) -> dict[str, str]:
    result: dict[str, str] = {}
    for key, value in _object(data, name).items():
        if not isinstance(value, str):
            raise ValueError(f"{name}.{key} must be a string")
        result[key] = value
    return result


def _flag(data: dict[str, Json], name: str) -> bool:
    value = data.get(name)
    if value is None:
        return False
    if not isinstance(value, bool):
        raise ValueError(f"{name} must be a boolean")
    return value


def _params(model: str, fields: dict[str, Json]) -> LiteLLM_Params:
    return LiteLLM_Params.model_validate(
        {
            "model": model,
            "api_base": _text(fields, "api_base"),
            "custom_llm_provider": _text(fields, "custom_llm_provider"),
            "api_key": _text(fields, "api_key"),
            "vertex_location": _text(fields, "vertex_location"),
            "vertex_project": _text(fields, "vertex_project"),
            "base_url": _text(fields, "base_url"),
            "stream": _flag(fields, "stream"),
        }
    )


def run(input_value: dict[str, Json]) -> Json:
    model = _text(input_value, "model")
    if model is None:
        raise ValueError("model must be a string")
    fields = _object(input_value, "params")
    aliases = _string_map(input_value, "model_alias_map")
    environment = _string_map(input_value, "environment")
    hooks = _string_map(input_value, "hooks")

    def login_free(provider: str) -> str | None:
        return hooks.get(provider)

    with ExitStack() as stack:
        stack.enter_context(mock.patch.dict(os.environ, environment, clear=True))
        stack.enter_context(mock.patch.object(litellm, "model_alias_map", aliases))
        stack.enter_context(
            mock.patch.object(api_base_module, "_api_base_without_login", login_free)
        )
        api_base = get_api_base(model, _params(model, fields))
    return {"api_base": api_base}
