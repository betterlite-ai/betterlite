from typing import Final, Literal, TypedDict

from litellm.llms.base_llm.anthropic_messages.transformation import (
    BaseAnthropicMessagesConfig,
)
from litellm.llms.base_llm.embedding.transformation import BaseEmbeddingConfig
from litellm.llms.base_llm.image_generation.transformation import (
    BaseImageGenerationConfig,
)
from litellm.llms.base_llm.rerank.transformation import BaseRerankConfig
from litellm.types.utils import LlmProviders
from litellm.utils import ProviderConfigManager
from pydantic import JsonValue


class Call(TypedDict):
    family: Literal["responses", "messages", "embedding", "rerank", "image_generation"]
    provider: str
    model: str
    identity: Literal["name", "enum"]


class Input(TypedDict):
    calls: list[Call]


# the native and providers fields steer the Rust selector; LiteLLM's own registry answers this side
JSON_RESPONSES_CLASS: Final = "JSONProviderResponsesConfig"
MESSAGES_CLASSES: Final = frozenset(
    {"DeepSeekAnthropicMessagesConfig", "JSONProviderAnthropicMessagesConfig"}
)
EMBEDDING_CLASSES: Final = frozenset({"CometAPIEmbeddingConfig"})
RERANK_CLASSES: Final = frozenset({"DeepinfraRerankConfig"})
IMAGE_CLASSES: Final = frozenset(
    {"AimlImageGenerationConfig", "CometAPIImageGenerationConfig"}
)

Policy = (
    BaseAnthropicMessagesConfig
    | BaseEmbeddingConfig
    | BaseRerankConfig
    | BaseImageGenerationConfig
)


def _policy(selected: Policy | None, extracted: frozenset[str]) -> JsonValue:
    if selected is None:
        return None
    name = type(selected).__name__
    if name in extracted:
        return name
    return None


def _responses_answer(call: Call) -> JsonValue:
    provider: LlmProviders | str = (
        LlmProviders(call["provider"]) if call["identity"] == "enum" else call["provider"]
    )
    selected = ProviderConfigManager.get_provider_responses_api_config(
        provider, call["model"]
    )
    if selected is None:
        return None
    name = type(selected).__name__
    if name == JSON_RESPONSES_CLASS:
        return f"json:{call['provider']}"
    return "native"


def _answer(call: Call) -> JsonValue:
    match call["family"]:
        case "responses":
            return _responses_answer(call)
        case "messages":
            return _policy(
                ProviderConfigManager.get_provider_anthropic_messages_config(
                    call["model"], LlmProviders(call["provider"])
                ),
                MESSAGES_CLASSES,
            )
        case "embedding":
            return _policy(
                ProviderConfigManager.get_provider_embedding_config(
                    call["model"], LlmProviders(call["provider"])
                ),
                EMBEDDING_CLASSES,
            )
        case "rerank":
            return _policy(
                ProviderConfigManager.get_provider_rerank_config(
                    call["model"], LlmProviders(call["provider"]), None, []
                ),
                RERANK_CLASSES,
            )
        case "image_generation":
            return _policy(
                ProviderConfigManager.get_provider_image_generation_config(
                    call["model"], LlmProviders(call["provider"])
                ),
                IMAGE_CLASSES,
            )


def run(input_value: Input) -> JsonValue:
    answers: list[JsonValue] = [_answer(call) for call in input_value["calls"]]
    return answers
