from typing import Final, NotRequired, TypedDict

from litellm.types.utils import LlmProviders
from litellm.utils import ProviderConfigManager, should_use_cohere_v1_client
from pydantic import JsonValue, TypeAdapter

V1_CLIENT: Final = TypeAdapter(bool)


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
    present: list[str] = call.get("present_version_params", [])
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
        "v1_client": V1_CLIENT.validate_python(
            should_use_cohere_v1_client(api_base, present)
        ),
    }


def run(input_value: Input) -> JsonValue:
    return [selection_answer(call) for call in input_value["calls"]]
