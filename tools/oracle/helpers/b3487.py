import asyncio
import base64
import datetime as dt
from collections.abc import AsyncIterator, Iterator
from typing import Literal, TypeAlias, TypedDict

import httpx
from litellm.google_genai.streaming_iterator import (
    AsyncGoogleGenAIGenerateContentStreamingIterator as AsyncNativeIterator,
)
from litellm.google_genai.streaming_iterator import (
    GoogleGenAIGenerateContentStreamingIterator as NativeSyncIterator,
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


class IteratorArgs(TypedDict):
    response: httpx.Response
    model: str
    logging_obj: Logging
    litellm_metadata: dict[str, JsonValue]
    custom_llm_provider: str
    request_body: dict[str, JsonValue]


NativeIterator: TypeAlias = NativeSyncIterator | AsyncNativeIterator


def logging_object(model: str) -> Logging:
    messages: list[dict[str, JsonValue]] = []
    return Logging(
        model=model,
        messages=messages,
        stream=True,
        call_type="agenerate_content_stream",
        start_time=dt.datetime.now(dt.UTC),
        litellm_call_id="call-1",
        function_id="function-1",
    )


def native_provider(provider: str) -> LlmProviders:
    if provider == "gemini":
        return LlmProviders.GEMINI
    return LlmProviders.VERTEX_AI


def config_of(input_value: Input) -> GenAiConfig:
    model = input_value["model"]
    selected = ProviderConfigManager.get_provider_google_genai_generate_content_config(
        model, native_provider(input_value["provider"])
    )
    if selected is None:
        raise ValueError(f"no native GenAI config for {model}")
    return selected


def response_of(input_value: Input) -> httpx.Response:
    content = "".join(f"{line}\n" for line in input_value["lines"])
    return httpx.Response(200, content=content)


def iterator_args(input_value: Input) -> IteratorArgs:
    return IteratorArgs(
        response=response_of(input_value),
        model=input_value["model"],
        logging_obj=logging_object(input_value["model"]),
        litellm_metadata={},
        custom_llm_provider=input_value["provider"],
        request_body={},
    )


def answer(iterator: NativeIterator, yielded: list[bytes]) -> Output:
    return {
        "provider": iterator.custom_llm_provider,
        "model": iterator.model,
        "endpoint": iterator.endpoint_type.name,
        "yielded": [base64.b64encode(chunk).decode("ascii") for chunk in yielded],
        "collected": [
            base64.b64encode(chunk).decode("ascii") for chunk in iterator.collected_chunks
        ],
        "yielded_matches_collected": yielded == iterator.collected_chunks,
    }


def sync_events(input_value: Input) -> Output:
    iterator = NativeSyncIterator(
        **iterator_args(input_value),
        generate_content_provider_config=config_of(input_value),
    )
    stream: Iterator[bytes] = iterator
    return answer(iterator, list(stream))


async def async_events(input_value: Input) -> Output:
    iterator = AsyncNativeIterator(
        **iterator_args(input_value),
        generate_content_provider_config=config_of(input_value),
    )
    stream: AsyncIterator[bytes] = iterator
    return answer(iterator, [chunk async for chunk in stream])


def run(input_value: Input) -> Output:
    if input_value["mode"] == "sync":
        return sync_events(input_value)
    return asyncio.run(async_events(input_value))
