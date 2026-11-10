import os
from typing import TypeAlias
from unittest import mock

import litellm
from litellm.litellm_core_utils import get_llm_provider_logic as provider_logic

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

_get_openai_compatible_provider_info = (
    provider_logic._get_openai_compatible_provider_info  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
)


def _string_list(data: dict[str, Json], name: str) -> list[str]:
    value = data.get(name)
    if value is None:
        return []
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list of strings")
    names: list[str] = []
    for item in value:
        if not isinstance(item, str):
            raise ValueError(f"{name} must be a list of strings")
        names.append(item)
    return names


def _string(data: dict[str, Json], name: str) -> str | None:
    value = data.get(name)
    if value is None:
        return None
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def run(data: dict[str, Json]) -> Json:
    provider = data["provider"]
    model = data["model"]
    if not isinstance(provider, str) or not isinstance(model, str):
        raise ValueError("provider and model must be strings")
    api_base = _string(data, "api_base")
    api_key = _string(data, "api_key")

    environment = data.get("environment") or {}
    if not isinstance(environment, dict):
        raise ValueError("environment must be an object")
    variables = {name: value for name, value in environment.items() if isinstance(value, str)}
    if len(variables) != len(environment):
        raise ValueError("environment values must be strings")

    openai_chat = _string_list(data, "openai_chat_models")
    openai_text = _string_list(data, "openai_text_completion_models")
    ai21_chat = _string_list(data, "ai21_chat_models")
    mantle_models = _string_list(data, "mantle_openai_path_models")
    model_cost = {
        f"bedrock_mantle/{name}": {"use_openai_responses_path": True}
        for name in mantle_models
    }

    with (
        mock.patch.dict(os.environ, variables, clear=True),
        mock.patch.object(litellm, "open_ai_chat_completion_models", set(openai_chat)),
        mock.patch.object(litellm, "open_ai_text_completion_models", set(openai_text)),
        mock.patch.object(litellm, "ai21_chat_models", set(ai21_chat)),
        mock.patch.object(litellm, "model_cost", model_cost),
    ):
        resolved_model, resolved_provider, dynamic_api_key, resolved_api_base = (
            _get_openai_compatible_provider_info(
                model=f"{provider}/{model}",
                api_base=api_base,
                api_key=api_key,
                dynamic_api_key=None,
            )
        )

    return {
        "provider": resolved_provider,
        "model": resolved_model,
        "api_base": resolved_api_base,
        "api_key": dynamic_api_key,
    }
