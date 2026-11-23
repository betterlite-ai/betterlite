from typing import Final, TypeAdapter

from pydantic import JsonValue

from betterlite._native import NativeProviders
from betterlite._provider.lookup import TEXT, call_lookup
from betterlite._provider.settings import provider_settings

PAIR: Final[TypeAdapter[tuple[str, str | None]]] = TypeAdapter(tuple[str, str | None])


# ast-grep-ignore: no-long-names
def handle_cohere_chat_model_custom_llm_provider(
    model: str, custom_llm_provider: str | None = None
) -> tuple[str, str | None]:
    request: dict[str, JsonValue] = {
        "model": TEXT.validate_python(model),
        "custom_llm_provider": TEXT.validate_python(custom_llm_provider),
    }
    answer = call_lookup(NativeProviders(provider_settings()).handle_cohere, request)
    return PAIR.validate_json(answer)


# ast-grep-ignore: no-long-names
def handle_anthropic_text_model_custom_llm_provider(
    model: str, custom_llm_provider: str | None = None
) -> tuple[str, str | None]:
    request: dict[str, JsonValue] = {
        "model": TEXT.validate_python(model),
        "custom_llm_provider": TEXT.validate_python(custom_llm_provider),
    }
    answer = call_lookup(NativeProviders(provider_settings()).handle_anthropic, request)
    return PAIR.validate_json(answer)
