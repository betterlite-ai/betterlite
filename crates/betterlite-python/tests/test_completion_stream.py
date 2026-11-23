import asyncio
from typing import Final

import pytest
from completion_server import (
    GOOD_KEY,
    GOOD_MODEL,
    STREAM_PARTS,
    CompletionServer,
    StreamParts,
    chunk_parts,
    litellm_async_stream_parts,
    litellm_stream_parts,
    ours_async_stream_parts,
    ours_stream_parts,
    stream_response,
)
from pydantic import JsonValue

from betterlite._completion.completion import settle_outcome
from betterlite.main import completion
from betterlite.types.utils import Message

MESSAGES: Final[list[dict[str, JsonValue] | Message]] = [{"role": "user", "content": "hi"}]
EXPECTED_CHUNKS: Final = len(STREAM_PARTS) + 1
FIRST_ROLE: Final = "assistant"
STOP_REASON: Final = "stop"
FIRST_PART: Final = 0


def test_stream_matches_litellm() -> None:
    with CompletionServer() as server:
        outcome = completion(
            model=GOOD_MODEL, messages=MESSAGES, api_base=server.url, api_key=GOOD_KEY,
            stream=True,
        )
        ours = ours_stream_parts(stream_response(outcome))
        reference = litellm_stream_parts(
            model=GOOD_MODEL, messages=MESSAGES, api_base=server.url, api_key=GOOD_KEY,
            stream=True,
        )
    assert ours == reference
    assert len(ours) == EXPECTED_CHUNKS
    assert ours[FIRST_PART][2] == FIRST_ROLE
    assert [part[1] for part in ours[:-1]] == list(STREAM_PARTS)
    assert ours[-1][1] is None
    assert ours[-1][3] == STOP_REASON


def test_stream_stops_at_the_end() -> None:
    with CompletionServer() as server:
        stream = stream_response(
            completion(
                model=GOOD_MODEL, messages=MESSAGES, api_base=server.url, api_key=GOOD_KEY,
                stream=True,
            )
        )
        assert len(ours_stream_parts(stream)) == EXPECTED_CHUNKS
        with pytest.raises(StopIteration):
            next(stream)


def test_stream_close_twice() -> None:
    with CompletionServer() as server:
        stream = stream_response(
            completion(
                model=GOOD_MODEL, messages=MESSAGES, api_base=server.url, api_key=GOOD_KEY,
                stream=True,
            )
        )
        first = next(stream)
        assert chunk_parts(first.model_dump())[1] == STREAM_PARTS[FIRST_PART]
        stream.close()
        stream.close()
        assert stream.closed
        with pytest.raises(StopIteration):
            next(stream)


async def collect_async(server_url: str) -> tuple[StreamParts, StreamParts]:
    outcome = await settle_outcome(
        completion(
            model=GOOD_MODEL, messages=MESSAGES, api_base=server_url, api_key=GOOD_KEY,
            stream=True, acompletion=True,
        )
    )
    ours = await ours_async_stream_parts(stream_response(outcome))
    reference = await litellm_async_stream_parts(
        model=GOOD_MODEL, messages=MESSAGES, api_base=server_url, api_key=GOOD_KEY,
        stream=True,
    )
    return ours, reference


def test_async_stream_matches() -> None:
    with CompletionServer() as server:
        ours, reference = asyncio.run(collect_async(server.url))
    assert ours == reference
    assert len(ours) == EXPECTED_CHUNKS
    assert ours[FIRST_PART][2] == FIRST_ROLE
    assert ours[-1][3] == STOP_REASON


async def close_async(server_url: str) -> None:
    outcome = await settle_outcome(
        completion(
            model=GOOD_MODEL, messages=MESSAGES, api_base=server_url, api_key=GOOD_KEY,
            stream=True, acompletion=True,
        )
    )
    stream = stream_response(outcome)
    first = await anext(stream)
    assert chunk_parts(first.model_dump())[1] == STREAM_PARTS[FIRST_PART]
    await stream.aclose()
    await stream.aclose()
    assert stream.closed
    with pytest.raises(StopAsyncIteration):
        await anext(stream)


def test_async_stream_closes() -> None:
    with CompletionServer() as server:
        asyncio.run(close_async(server.url))
