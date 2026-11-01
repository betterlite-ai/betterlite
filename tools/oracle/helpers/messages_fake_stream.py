import asyncio
from typing import TypeAlias, cast

from litellm.llms.anthropic.pass_through.messages.fake_stream_iterator import (
    FakeAnthropicMessagesStreamIterator,
)
from litellm.types.llms.anthropic_messages.anthropic_response import AnthropicMessagesResponse

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def run(input_value: dict[str, Json]) -> list[str]:
    response = cast(AnthropicMessagesResponse, input_value["response"])
    iterator = FakeAnthropicMessagesStreamIterator(response)
    chunks: list[bytes] = []
    use_async = False
    while True:
        try:
            chunk = asyncio.run(iterator.__anext__()) if use_async else next(iterator)
        except (StopIteration, StopAsyncIteration):
            break
        chunks.append(chunk)
        use_async = not use_async
    return [chunk.decode() for chunk in chunks]
