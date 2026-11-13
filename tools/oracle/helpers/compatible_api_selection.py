from typing import Literal, TypeAlias, TypedDict
from unittest.mock import patch

from litellm.llms.openai_like.json_loader import JSONProviderRegistry, SimpleProviderConfig
from litellm.types.utils import LlmProviders
from litellm.utils import ProviderConfigManager
from pydantic import JsonValue


class Descriptor(TypedDict):
    provider: str
    supported_endpoints: list[str]


class ResponsesCall(TypedDict):
    family: Literal["responses"]
    provider: str
    identity: Literal["name", "enum"]
    native: str | None
    providers: list[Descriptor]


class MessagesCall(TypedDict):
    family: Literal["messages"]
    provider: str
    model: str
    providers: list[Descriptor]


class PlainCall(TypedDict):
    family: Literal["embedding", "rerank", "image_generation"]
    provider: str
    model: str


Call: TypeAlias = ResponsesCall | MessagesCall | PlainCall


class Input(TypedDict):
    calls: list[Call]


class Registry:
    def __init__(self, descriptors: list[Descriptor]) -> None:
        self.entries: dict[str, Descriptor] = {
            entry["provider"]: entry for entry in descriptors
        }
        self.selected: str | None = None

    def exists(self, slug: str) -> bool:
        return slug in self.entries

    def supports_responses_api(self, slug: str) -> bool:
        entry = self.entries.get(slug)
        if entry is None:
            return False
        return "/v1/responses" in entry["supported_endpoints"]

    def get(self, slug: str) -> SimpleProviderConfig | None:
        entry = self.entries.get(slug)
        if entry is None:
            return None
        self.selected = slug
        return SimpleProviderConfig(
            slug,
            {
                "base_url": "",
                "api_key_env": "",
                "supported_endpoints": entry["supported_endpoints"],
            },
        )


def _provider(identity: str, provider: str) -> LlmProviders | str:
    if identity == "enum":
        return LlmProviders(provider)
    return provider


def _responses_answer(call: ResponsesCall) -> JsonValue:
    registry = Registry(call["providers"])
    with (
        patch.object(JSONProviderRegistry, "exists", registry.exists),
        patch.object(
            JSONProviderRegistry,
            "supports_responses_api",
            registry.supports_responses_api,
        ),
        patch.object(JSONProviderRegistry, "get", registry.get),
    ):
        selected = ProviderConfigManager.get_provider_responses_api_config(
            _provider(call["identity"], call["provider"])
        )
    if selected is None:
        return None
    if registry.selected is None:
        return "native"
    return f"json:{registry.selected}"


def _messages_answer(call: MessagesCall) -> JsonValue:
    registry = Registry(call["providers"])
    with patch.object(JSONProviderRegistry, "get", registry.get):
        selected = ProviderConfigManager.get_provider_anthropic_messages_config(
            call["model"], LlmProviders(call["provider"])
        )
    if selected is None:
        return None
    return type(selected).__name__


def _embedding_answer(call: PlainCall) -> JsonValue:
    selected = ProviderConfigManager.get_provider_embedding_config(
        call["model"], LlmProviders(call["provider"])
    )
    if selected is None:
        return None
    return type(selected).__name__


def _rerank_answer(call: PlainCall) -> JsonValue:
    selected = ProviderConfigManager.get_provider_rerank_config(
        call["model"], LlmProviders(call["provider"]), None, []
    )
    return type(selected).__name__


def _image_answer(call: PlainCall) -> JsonValue:
    selected = ProviderConfigManager.get_provider_image_generation_config(
        call["model"], LlmProviders(call["provider"])
    )
    if selected is None:
        return None
    return type(selected).__name__


def _answer(call: Call) -> JsonValue:
    match call["family"]:
        case "responses":
            return _responses_answer(call)
        case "messages":
            return _messages_answer(call)
        case "embedding":
            return _embedding_answer(call)
        case "rerank":
            return _rerank_answer(call)
        case "image_generation":
            return _image_answer(call)


def run(input_value: Input) -> JsonValue:
    answers: list[JsonValue] = [_answer(call) for call in input_value["calls"]]
    return {"calls": answers}
