from typing import Literal, TypedDict

from litellm.llms.nvidia_nim.chat.transformation import NvidiaNimConfig
from litellm.llms.nvidia_nim.embed import NvidiaNimEmbeddingConfig
from pydantic import JsonValue

from .config import JSON, ChatConfig, EmbeddingConfig, JsonObject

CHAT: ChatConfig = NvidiaNimConfig()
EMBEDDING: EmbeddingConfig = NvidiaNimEmbeddingConfig()


class ChatSupportedInput(TypedDict):
    kind: Literal["chat_supported"]
    models: list[str]


class ChatMapCase(TypedDict):
    model: str
    non_default: JsonObject
    optional: JsonObject


class ChatMapInput(TypedDict):
    kind: Literal["chat_map"]
    cases: list[ChatMapCase]


class EmbeddingMapCase(TypedDict):
    non_default: JsonObject
    optional: JsonObject
    kwargs: JsonObject | None


class EmbeddingMapInput(TypedDict):
    kind: Literal["embedding_map"]
    cases: list[EmbeddingMapCase]


def chat_supported(input_value: ChatSupportedInput) -> JsonValue:
    supported = [CHAT.get_supported_openai_params(model) for model in input_value["models"]]
    return {"supported": JSON.validate_python(supported)}


def chat_map(input_value: ChatMapInput) -> JsonValue:
    mapped = [
        CHAT.map_openai_params(
            case["non_default"], dict(case["optional"]), case["model"], drop_params=False
        )
        for case in input_value["cases"]
    ]
    return {"mapped": JSON.validate_python(mapped)}


def embedding_map(input_value: EmbeddingMapInput) -> JsonValue:
    mapped: list[JsonValue] = []
    for case in input_value["cases"]:
        try:
            params = EMBEDDING.map_openai_params(
                case["non_default"], dict(case["optional"]), case["kwargs"]
            )
        except AttributeError:
            mapped.append({"ok": False})
            continue
        mapped.append({"ok": True, "mapped": params})
    supported = EMBEDDING.get_supported_openai_params()
    return {"supported": JSON.validate_python(supported), "mapped": mapped}
