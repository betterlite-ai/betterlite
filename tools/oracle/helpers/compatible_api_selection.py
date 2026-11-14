from typing import Literal, TypedDict
from unittest.mock import patch

from litellm.llms.openai_like.json_loader import JSONProviderRegistry, SimpleProviderConfig
from litellm.types.utils import LlmProviders
from litellm.utils import ProviderConfigManager
from pydantic import JsonValue


class Descriptor(TypedDict):
    provider: str
    supported_endpoints: list[str]


class Call(TypedDict):
    family: Literal["responses", "messages", "embedding", "rerank", "image_generation"]
    provider: str
    identity: Literal["name", "enum"]
    native: str | None
    providers: list[Descriptor]
    model: str


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


def _responses_answer(call: Call) -> JsonValue:
    registry = Registry(call["providers"])
    provider: LlmProviders | str = call["provider"]
    if call["identity"] == "enum":
        provider = LlmProviders(call["provider"])
    with (
        patch.object(JSONProviderRegistry, "exists", registry.exists),
        patch.object(
            JSONProviderRegistry,
            "supports_responses_api",
            registry.supports_responses_api,
        ),
        patch.object(JSONProviderRegistry, "get", registry.get),
    ):
        selected = ProviderConfigManager.get_provider_responses_api_config(provider)
    if selected is None:
        return None
    if registry.selected is None:
        return "native"
    return f"json:{registry.selected}"


def _plain_answer(call: Call) -> JsonValue:
    provider = LlmProviders(call["provider"])
    match call["family"]:
        case "embedding":
            selected = ProviderConfigManager.get_provider_embedding_config(
                call["model"], provider
            )
        case "rerank":
            selected = ProviderConfigManager.get_provider_rerank_config(
                call["model"], provider, None, []
            )
        case "image_generation":
            selected = ProviderConfigManager.get_provider_image_generation_config(
                call["model"], provider
            )
        case _:
            raise ValueError(f"unknown family {call['family']}")
    if selected is None:
        return None
    return type(selected).__name__


def _answer(call: Call) -> JsonValue:
    match call["family"]:
        case "responses":
            return _responses_answer(call)
        case "messages":
            registry = Registry(call["providers"])
            with patch.object(JSONProviderRegistry, "get", registry.get):
                selected = ProviderConfigManager.get_provider_anthropic_messages_config(
                    call["model"], LlmProviders(call["provider"])
                )
            if selected is None:
                return None
            return type(selected).__name__
        case _:
            return _plain_answer(call)


def run(input_value: list[Call]) -> JsonValue:
    answers: list[JsonValue] = [_answer(call) for call in input_value]
    return answers
