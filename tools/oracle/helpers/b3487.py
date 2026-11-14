import asyncio
import base64
import datetime
from collections.abc import AsyncIterator, Iterator
from typing import Literal, TypedDict

from litellm.google_genai.streaming_iterator import (
    AsyncGoogleGenAIGenerateContentStreamingIterator,
    GoogleGenAIGenerateContentStreamingIterator,
)
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.google_genai.transformation import (
    BaseGoogleGenAIGenerateContentConfig as GenAiConfig,
)
from litellm.types.utils import LlmProviders
from litellm.utils import ProviderConfigManager
from pydantic import JsonValue


class Input(TypedDict):
    mode: Literal["sync", "async"]
    provider: str
    model: str
    lines: list[str]


class Output(TypedDict):
    provider: str
    model: str
    endpoint: str
    yielded: list[str]
    collected: list[str]
    yielded_matches_collected: bool


class FakeResponse:
    def __init__(self, lines: list[str]) -> None:
        self.lines = lines

    def iter_lines(self) -> Iterator[str]:
        for line in self.lines:
            yield line

    async def aiter_lines(self) -> AsyncIterator[str]:
        for line in self.lines:
            yield line


def logging_object(model: str) -> Logging:
    messages: list[dict[str, JsonValue]] = []
    return Logging(
        model=model,
        messages=messages,
        stream=True,
        call_type="agenerate_content_stream",
        start_time=datetime.datetime.now(datetime.UTC),
        litellm_call_id="call-1",
        function_id="function-1",
    )


def native_provider(provider: str) -> LlmProviders:
    if provider == "gemini":
        return LlmProviders.GEMINI
    return LlmProviders.VERTEX_AI


def provider_config(model: str, provider: LlmProviders) -> GenAiConfig:
    selected = ProviderConfigManager.get_provider_google_genai_generate_content_config(
        model, provider
    )
    if selected is None:
        raise ValueError(f"no native GenAI config for {model}")
    return selected


def answer(
    provider: str,
    model: str,
    endpoint: str,
    yielded: list[bytes],
    collected: list[bytes],
) -> Output:
    return {
        "provider": provider,
        "model": model,
        "endpoint": endpoint,
        "yielded": [base64.b64encode(chunk).decode("ascii") for chunk in yielded],
        "collected": [base64.b64encode(chunk).decode("ascii") for chunk in collected],
        "yielded_matches_collected": yielded == collected,
    }


def sync_events(input_value: Input) -> Output:
    litellm_metadata: dict[str, JsonValue] = {}
    request_body: dict[str, JsonValue] = {}
    iterator = GoogleGenAIGenerateContentStreamingIterator(
        response=FakeResponse(input_value["lines"]),
        model=input_value["model"],
        logging_obj=logging_object(input_value["model"]),
        generate_content_provider_config=provider_config(
            input_value["model"], native_provider(input_value["provider"])
        ),
        litellm_metadata=litellm_metadata,
        custom_llm_provider=input_value["provider"],
        request_body=request_body,
    )
    stream: Iterator[bytes] = iterator
    yielded = list(stream)
    return answer(
        iterator.custom_llm_provider,
        iterator.model,
        iterator.endpoint_type.name,
        yielded,
        iterator.collected_chunks,
    )


async def async_events(input_value: Input) -> Output:
    litellm_metadata: dict[str, JsonValue] = {}
    request_body: dict[str, JsonValue] = {}
    iterator = AsyncGoogleGenAIGenerateContentStreamingIterator(
        response=FakeResponse(input_value["lines"]),
        model=input_value["model"],
        logging_obj=logging_object(input_value["model"]),
        generate_content_provider_config=provider_config(
            input_value["model"], native_provider(input_value["provider"])
        ),
        litellm_metadata=litellm_metadata,
        custom_llm_provider=input_value["provider"],
        request_body=request_body,
    )
    stream: AsyncIterator[bytes] = iterator
    yielded = [chunk async for chunk in stream]
    return answer(
        iterator.custom_llm_provider,
        iterator.model,
        iterator.endpoint_type.name,
        yielded,
        iterator.collected_chunks,
    )


def run(input_value: Input) -> Output:
    if input_value["mode"] == "sync":
        return sync_events(input_value)
    return asyncio.run(async_events(input_value))
