import json
import time
from typing import Protocol, TypedDict, cast

from litellm.llms.ollama.completion.transformation import OllamaTextCompletionResponseIterator

from .support import UUID_PATTERN, FrozenClock, Json, StreamInput, UsageRecord, usage_list


class StreamDelta(Protocol):
    content: str | None

    def get(self, key: str) -> str | None: ...


class StreamChoice(Protocol):
    index: int
    finish_reason: str | None
    delta: StreamDelta


class StreamChunk(Protocol):
    id: str
    object: str
    created: int
    model: str | None
    choices: list[StreamChoice]

    def get(self, key: str) -> UsageRecord | None: ...


class DoneUsage(TypedDict):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int


class DoneChunk(TypedDict):
    text: str
    is_finished: bool
    finish_reason: str
    usage: DoneUsage | None


class ChunkParser(Protocol):
    def chunk_parser(self, chunk: dict[str, Json]) -> DoneChunk | StreamChunk: ...


def chunk_value(chunk: StreamChunk) -> Json:
    choices: list[Json] = [
        {
            "index": choice.index,
            "finish_reason": choice.finish_reason,
            "content": choice.delta.content,
            "reasoning_content": choice.delta.get("reasoning_content"),
        }
        for choice in chunk.choices
    ]
    return {
        "id": UUID_PATTERN.sub("<uuid>", chunk.id),
        "object": chunk.object,
        "created": chunk.created,
        "model": chunk.model,
        "usage": usage_list(chunk.get("usage")),
        "choices": choices,
    }


def block_usage(usage: DoneUsage | None) -> Json:
    if usage is None:
        return None
    return [usage["prompt_tokens"], usage["completion_tokens"], usage["total_tokens"]]


def done_value(done: DoneChunk) -> Json:
    return {
        "ok": True,
        "done": True,
        "text": done["text"],
        "is_finished": done["is_finished"],
        "finish_reason": done["finish_reason"],
        "usage": block_usage(done["usage"]),
    }


def stream_step(parser: ChunkParser, line: str) -> Json:
    try:
        parsed = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/ollama/completion/transformation.py#L500
            "dict[str, Json]", json.loads(line)
        )
        result = parser.chunk_parser(parsed)
    except (AttributeError, KeyError, TypeError, ValueError):
        return {"ok": False, "status": None}
    match result:
        case {"text": _}:
            return done_value(cast("DoneChunk", result))  # pyright: ignore[reportUnnecessaryCast]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/ollama/completion/transformation.py#L502
        case _:
            return {"ok": True, "chunk": chunk_value(cast("StreamChunk", result))}  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/ollama/completion/transformation.py#L502


def stream_cases(input_value: StreamInput) -> Json:
    time.time = FrozenClock()  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2135
    iterator = OllamaTextCompletionResponseIterator(
        streaming_response=iter([]), sync_stream=True, json_mode=False
    )
    parser = cast("ChunkParser", iterator)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/ollama/completion/transformation.py#L502
    chunks: list[Json] = [stream_step(parser, line) for line in input_value["lines"]]
    return {"chunks": chunks}
