import asyncio
from collections.abc import Awaitable, Iterator
from typing import Literal, Protocol, TypedDict

import litellm
from litellm.llms.vertex_ai.vertex_ai_non_gemini import TextStreamer

# an unmapped model prints a provider banner on stdout, which carries the oracle's JSON line
litellm.suppress_debug_info = True

Step = Literal["sync", "async"]


class WordStream(Protocol):
    def __iter__(self) -> Iterator[str]: ...

    def __next__(self) -> str: ...

    def __anext__(self) -> Awaitable[str]: ...


class Input(TypedDict):
    text: str
    steps: list[Step]


class Answer(TypedDict):
    words: list[str]
    asyncWords: list[str]
    trace: list[str]


def sync_next(streamer: WordStream) -> str:
    try:
        word = next(streamer)
    except StopIteration:
        return "StopIteration"
    return word


async def async_next(streamer: WordStream) -> str:
    try:
        word = await anext(streamer)
    except StopAsyncIteration:
        return "StopAsyncIteration"
    return word


async def traced(streamer: WordStream, steps: list[Step]) -> list[str]:
    trace: list[str] = []
    for step in steps:
        if step == "sync":
            trace.append(sync_next(streamer))
        else:
            trace.append(await async_next(streamer))
    return trace


async def collected_async(streamer: WordStream) -> list[str]:
    words: list[str] = []
    while True:
        try:
            words.append(await anext(streamer))
        except StopAsyncIteration:
            return words


def run(input_value: Input) -> Answer:
    streamer: WordStream = TextStreamer(input_value["text"])
    async_streamer: WordStream = TextStreamer(input_value["text"])
    traced_streamer: WordStream = TextStreamer(input_value["text"])
    return {
        "words": list(streamer),
        "asyncWords": asyncio.run(collected_async(async_streamer)),
        "trace": asyncio.run(traced(traced_streamer, input_value["steps"])),
    }
