import asyncio
from collections.abc import AsyncIterator, Iterator
from typing import Protocol, TypedDict, cast

import httpx
from litellm.llms.langgraph.chat.sse_iterator import LangGraphSSEStreamIterator
from litellm.types.utils import Delta, ModelResponseStream
from pydantic import JsonValue


class StreamCase(TypedDict):
    model: str
    lines: list[str]


class Input(TypedDict):
    cases: list[StreamCase]


# StreamingChoices assigns these in __init__ without declaring them, so mypy reads them as Any
class ChunkChoice(Protocol):
    index: int
    finish_reason: str | None
    delta: Delta


def iterator(case: StreamCase) -> LangGraphSSEStreamIterator:
    body = "\n".join(case["lines"])
    return LangGraphSSEStreamIterator(
        response=httpx.Response(200, content=body.encode()), model=case["model"]
    )


def chunk_view(chunk: ModelResponseStream) -> JsonValue:
    choice = cast(ChunkChoice, chunk.choices[0])  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2006
    delta = choice.delta
    return {
        "created": chunk.created,
        "model": chunk.model,
        "object": chunk.object,
        "choices": [
            {
                "index": choice.index,
                "finish_reason": choice.finish_reason,
                "delta": {"content": delta.content, "role": delta.role},
            }
        ],
    }


def sync_chunks(case: StreamCase) -> list[JsonValue]:
    stream = cast("Iterator[ModelResponseStream]", iterator(case))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/langgraph/chat/sse_iterator.py#L34
    return [chunk_view(chunk) for chunk in stream]


async def async_chunks(case: StreamCase) -> list[JsonValue]:
    stream = cast("AsyncIterator[ModelResponseStream]", iterator(case))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/langgraph/chat/sse_iterator.py#L39
    return [chunk_view(chunk) async for chunk in stream]


def run(input_value: Input) -> JsonValue:
    return {
        "cases": [
            {"sync": sync_chunks(case), "async": asyncio.run(async_chunks(case))}
            for case in input_value["cases"]
        ]
    }
