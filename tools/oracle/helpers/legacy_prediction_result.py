import asyncio
from typing import Literal, TypedDict

import litellm
from litellm.llms.vertex_ai.vertex_ai_non_gemini import TextStreamer

# an unmapped model prints a provider banner on stdout, which carries the oracle's JSON line
litellm.suppress_debug_info = True

Step = Literal["sync", "async"]


class Input(TypedDict):
    text: str
    steps: list[Step]


class Answer(TypedDict):
    words: list[str]
    asyncWords: list[str]
    trace: list[str]


def sync_next(streamer: TextStreamer) -> str:
    try:
        word = next(streamer)
    except StopIteration:
        return "StopIteration"
    return word


async def async_next(streamer: TextStreamer) -> str:
    try:
        word = await anext(streamer)
    except StopAsyncIteration:
        return "StopAsyncIteration"
    return word


async def traced(streamer: TextStreamer, steps: list[Step]) -> list[str]:
    trace: list[str] = []
    for step in steps:
        if step == "sync":
            trace.append(sync_next(streamer))
        else:
            trace.append(await async_next(streamer))
    return trace


async def collected_async(streamer: TextStreamer) -> list[str]:
    return [word async for word in streamer]


def run(input_value: Input) -> Answer:
    words = list(TextStreamer(input_value["text"]))
    async_words = asyncio.run(collected_async(TextStreamer(input_value["text"])))
    trace = asyncio.run(traced(TextStreamer(input_value["text"]), input_value["steps"]))
    return {"words": words, "asyncWords": async_words, "trace": trace}
