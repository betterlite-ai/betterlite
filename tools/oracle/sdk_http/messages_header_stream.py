import asyncio
import json
from collections.abc import AsyncGenerator, AsyncIterator
from typing import TypedDict

import httpx
from litellm.llms.anthropic.pass_through.messages.streaming_iterator import (
    AnthropicMessagesStreamingResponse,
    anthropic_messages_stream_hidden_params,
)
from pydantic import JsonValue as Json
from typing_extensions import override

TIMEOUT_SECONDS = 120.0


class MessagesHeaderStreamCall(TypedDict):
    body: dict[str, Json]
    buffered_provider_output: Json


class ProviderChunks(AsyncIterator[bytes]):
    def __init__(self, chunks: AsyncGenerator[bytes, None], buffered: Json) -> None:
        self.chunks = chunks
        self.has_buffered_provider_output = buffered
        self.closed = False

    @override
    def __aiter__(self) -> "ProviderChunks":
        return self

    @override
    async def __anext__(self) -> bytes:
        return await anext(self.chunks)

    async def aclose(self) -> None:
        self.closed = True


async def passthrough(source: AsyncIterator[bytes]) -> AsyncGenerator[bytes, None]:
    async for chunk in source:
        yield chunk


async def collect(call: MessagesHeaderStreamCall, api_base: str) -> Json:
    async with (
        httpx.AsyncClient(trust_env=False, timeout=TIMEOUT_SECONDS) as client,
        client.stream(
            "POST",
            f"{api_base}/messages",
            json=call["body"],
            headers={"content-type": "application/json"},
        ) as response,
    ):
        hidden = anthropic_messages_stream_hidden_params(response.headers)
        stream = ProviderChunks(
            passthrough(response.aiter_bytes()), call["buffered_provider_output"]
        )
        view = AnthropicMessagesStreamingResponse(
            completion_stream=stream,
            hidden_params=hidden,
        )
        provider_bytes: list[Json] = []
        async for chunk in view:
            provider_bytes.extend(chunk)
        await view.aclose()
        value: Json = {
            "hidden_params": {"additional_headers": dict(hidden["additional_headers"])},
            "provider_bytes": provider_bytes,
            "has_buffered_provider_output": view.has_buffered_provider_output,
            "closed": stream.closed,
        }
        return value


def run_messages_header_stream(call: MessagesHeaderStreamCall, api_base: str) -> str:
    value = asyncio.run(collect(call, api_base))
    outcome: Json = {"messages_header_stream": {"value": value}}
    return json.dumps(outcome)
