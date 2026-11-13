from typing import Protocol

from litellm.llms.nvidia_nim.chat.transformation import NvidiaNimConfig
from litellm.llms.nvidia_nim.embed import NvidiaNimEmbeddingConfig
from pydantic import JsonValue, TypeAdapter

from .types import ChatMap, ChatSupported, EmbeddingMap, JsonObject

ADAPTER: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


class ChatConfig(Protocol):
    def get_supported_openai_params(self, model: str) -> list[str]: ...
    def map_openai_params(
        self,
        non_default_params: JsonObject,
        optional_params: JsonObject,
        model: str,
        *,
        drop_params: bool,
    ) -> JsonObject: ...


class EmbeddingConfig(Protocol):
    def get_supported_openai_params(self) -> list[str]: ...
    def map_openai_params(
        self,
        non_default_params: JsonObject,
        optional_params: JsonObject,
        kwargs: JsonObject | None = None,
    ) -> JsonObject: ...


CHAT: ChatConfig = NvidiaNimConfig()
EMBEDDING: EmbeddingConfig = NvidiaNimEmbeddingConfig()


def chat_supported(input_value: ChatSupported) -> JsonValue:
    supported = [CHAT.get_supported_openai_params(model) for model in input_value["models"]]
    return ADAPTER.validate_python({"supported": supported})


def chat_map(input_value: ChatMap) -> JsonValue:
    mapped = [
        CHAT.map_openai_params(
            non_default_params=dict(call["non_default"]),
            optional_params=dict(call["optional"]),
            model=call["model"],
            drop_params=False,
        )
        for call in input_value["cases"]
    ]
    return ADAPTER.validate_python({"mapped": mapped})


def embedding_map(input_value: EmbeddingMap) -> JsonValue:
    results: list[JsonValue] = []
    for call in input_value["cases"]:
        try:
            mapped = EMBEDDING.map_openai_params(
                non_default_params=dict(call["non_default"]),
                optional_params=dict(call["optional"]),
                kwargs=call["kwargs"],
            )
        except AttributeError:
            results.append({"ok": False})
            continue
        results.append({"ok": True, "mapped": mapped})
    supported = EMBEDDING.get_supported_openai_params()
    return ADAPTER.validate_python({"supported": supported, "mapped": results})
