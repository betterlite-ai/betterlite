import os
from typing import TypeAlias
from unittest import mock

import litellm
from litellm.litellm_core_utils import get_llm_provider_logic as provider_logic

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

_get_openai_compatible_provider_info = (
    provider_logic._get_openai_compatible_provider_info  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
)


def run(data: dict[str, Json]) -> Json:
    provider = data["provider"]
    model = data["model"]
    if not isinstance(provider, str) or not isinstance(model, str):
        raise ValueError("provider and model must be strings")
    api_base = data.get("api_base")
    api_key = data.get("api_key")
    if api_base is not None and not isinstance(api_base, str):
        raise ValueError("api_base must be a string")
    if api_key is not None and not isinstance(api_key, str):
        raise ValueError("api_key must be a string")

    environment = data.get("environment") or {}
    if not isinstance(environment, dict):
        raise ValueError("environment must be an object")
    variables = {name: value for name, value in environment.items() if isinstance(value, str)}
    if len(variables) != len(environment):
        raise ValueError("environment values must be strings")

    global_api_key = data.get("global_api_key")
    global_api_base = data.get("global_api_base")
    xai_key = data.get("xai_key")
    inception_key = data.get("inception_key")
    lemonade_key = data.get("lemonade_key")
    for name, value in (
        ("global_api_key", global_api_key),
        ("global_api_base", global_api_base),
        ("xai_key", xai_key),
        ("inception_key", inception_key),
        ("lemonade_key", lemonade_key),
    ):
        if value is not None and not isinstance(value, str):
            raise ValueError(f"{name} must be a string")

    full_model = f"{provider}/{model}"
    with mock.patch.dict(os.environ, variables, clear=True):
        litellm.api_key = global_api_key
        litellm.api_base = global_api_base
        litellm.xai_key = xai_key
        litellm.inception_key = inception_key
        litellm.lemonade_key = lemonade_key
        resolved_model, _, dynamic_api_key, resolved_api_base = (
            _get_openai_compatible_provider_info(
                model=full_model,
                api_base=api_base,
                api_key=api_key,
                dynamic_api_key=None,
            )
        )

    return {"model": resolved_model, "api_base": resolved_api_base, "api_key": dynamic_api_key}
