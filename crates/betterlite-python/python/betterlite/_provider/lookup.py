import logging
from collections.abc import Callable
from typing import Final, TypeAlias

from pydantic import JsonValue, TypeAdapter

import betterlite
from betterlite import exceptions
from betterlite._completion.errors import raise_native_error
from betterlite._native import NativeError, NativeProviders
from betterlite._provider.settings import CUSTOM_PROVIDERS, JSON, provider_settings
from betterlite._router_types_params import GenericLiteLLMParams

LOGGER: Final = logging.getLogger("LiteLLM")
TEXT: Final[TypeAdapter[str | None]] = TypeAdapter(str | None)
RESULT: Final[TypeAdapter[tuple[str, str, str | None, str | None]]] = TypeAdapter(
    tuple[str, str, str | None, str | None]
)
NativeCall: TypeAlias = Callable[[str], str]


def call_lookup(method: NativeCall, request: dict[str, JsonValue]) -> str:
    answer: str
    try:
        answer = method(JSON.dump_json(request).decode())
    except NativeError as error:
        raise_native_error(error)
    return answer


def _send(
    method: NativeCall, request: dict[str, JsonValue]
) -> tuple[str, str, str | None, str | None]:
    answer = call_lookup(method, request)
    return RESULT.validate_json(answer)


def get_llm_provider(
    model: str | None,
    custom_llm_provider: str | None = None,
    api_base: str | None = None,
    api_key: str | None = None,
    litellm_params: GenericLiteLLMParams | None = None,
) -> tuple[str, str, str | None, str | None]:
    # the params fill in only when all three arguments are None, as LiteLLM reads them
    if (
        litellm_params is not None
        and custom_llm_provider is None
        and api_base is None
        and api_key is None
    ):
        custom_llm_provider = litellm_params.custom_llm_provider
        api_base = litellm_params.api_base
        api_key = litellm_params.api_key
    request: dict[str, JsonValue] = {
        "model": TEXT.validate_python(model),
        "custom_llm_provider": TEXT.validate_python(custom_llm_provider),
        "api_base": TEXT.validate_python(api_base),
        "api_key": TEXT.validate_python(api_key),
    }
    return _send(NativeProviders(provider_settings()).get_llm_provider, request)


# ast-grep-ignore: no-long-names
def declared_authenticating_provider(
    model: str | None, custom_llm_provider: str | None = None
) -> str | None:
    request: dict[str, JsonValue] = {
        "model": TEXT.validate_python(model),
        "custom_llm_provider": TEXT.validate_python(custom_llm_provider),
    }
    answer = call_lookup(NativeProviders.declared_authenticating, request)
    return TEXT.validate_json(answer)


def inferred_provider(model: str | None) -> str | None:
    if not model:
        return None
    declared = declared_authenticating_provider(model)
    if declared is not None:
        return declared
    try:
        provider = get_llm_provider(model)[1]
    except exceptions.BadRequestError:
        return None
    return provider


def is_registered_custom_provider(custom_llm_provider: str | None) -> bool:
    entries = CUSTOM_PROVIDERS.validate_python(betterlite.custom_provider_map)
    return any(entry["provider"] == custom_llm_provider for entry in entries)


# ast-grep-ignore: no-long-names
def _get_openai_compatible_provider_info(
    model: str,
    api_base: str | None,
    api_key: str | None,
    dynamic_api_key: str | None,
    litellm_params: GenericLiteLLMParams | None = None,
) -> tuple[str, str, str | None, str | None]:
    # litellm_params is read only for bedrock_mantle, whose defaults the Rust tables resolve
    if litellm_params is not None:
        LOGGER.debug("litellm_params is not used by the OpenAI-compatible provider lookup")
    fields: list[tuple[str, str | None]] = [
        ("model", model),
        ("api_base", api_base),
        ("api_key", api_key),
        ("dynamic_api_key", dynamic_api_key),
    ]
    request: dict[str, JsonValue] = {}
    for name, value in fields:
        request[name] = TEXT.validate_python(value)
    return _send(NativeProviders(provider_settings()).openai_compatible_info, request)


__all__ = [
    "_get_openai_compatible_provider_info", "call_lookup", "declared_authenticating_provider",
    "get_llm_provider", "inferred_provider", "is_registered_custom_provider",
]
