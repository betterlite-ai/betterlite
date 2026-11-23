from typing import Final

from pydantic import JsonValue, TypeAdapter

from betterlite._native import NativeProviders
from betterlite._provider.lookup import TEXT, NativeCall, call_lookup
from betterlite._provider.settings import provider_settings

PAIR: Final[TypeAdapter[tuple[str, str | None]]] = TypeAdapter(tuple[str, str | None])


def _pair(method: NativeCall, model: str, custom: str | None) -> tuple[str, str | None]:
    request: dict[str, JsonValue] = {
        "model": TEXT.validate_python(model),
        "custom_llm_provider": TEXT.validate_python(custom),
    }
    return PAIR.validate_json(call_lookup(method, request))


# ast-grep-ignore: no-long-names
def handle_cohere_chat_model_custom_llm_provider(
    model: str, custom_llm_provider: str | None = None
) -> tuple[str, str | None]:
    method = NativeProviders(provider_settings()).handle_cohere
    return _pair(method, model, custom_llm_provider)


# ast-grep-ignore: no-long-names
def handle_anthropic_text_model_custom_llm_provider(
    model: str, custom_llm_provider: str | None = None
) -> tuple[str, str | None]:
    method = NativeProviders(provider_settings()).handle_anthropic
    return _pair(method, model, custom_llm_provider)
