from typing import NotRequired, TypedDict

from litellm.types.utils import LlmProviders
from litellm.utils import ProviderConfigManager
from pydantic import JsonValue


class Call(TypedDict):
    provider: str
    model: str
    api_base: NotRequired[str | None]
    present_version_params: NotRequired[list[str]]


class Input(TypedDict):
    calls: list[Call]


def selection_answer(call: Call) -> dict[str, JsonValue]:
    provider = LlmProviders(call["provider"])
    api_base = call.get("api_base")
    called = call.get("present_version_params")
    present: list[str] = [] if called is None else called
    embedding = ProviderConfigManager.get_provider_embedding_config(
        model=call["model"], provider=provider
    )
    rerank = ProviderConfigManager.get_provider_rerank_config(
        model=call["model"],
        provider=provider,
        api_base=api_base,
        present_version_params=present,
    )
    return {
        "embedding": None if embedding is None else type(embedding).__name__,
        "rerank": type(rerank).__name__,
    }


def run(input_value: Input) -> JsonValue:
    calls = input_value["calls"]
    answers: list[JsonValue] = [selection_answer(call) for call in calls]
    return answers
