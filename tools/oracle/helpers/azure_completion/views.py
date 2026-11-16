from typing import cast

import httpx
from litellm.llms.azure.common_utils import AzureOpenAIError
from pydantic import JsonValue

from .fixture import Fixture, Recorder
from .types import (
    CHUNK,
    CREATED_FLOOR,
    AsyncStream,
    ChunkView,
    ErrorView,
    ReplyChoiceView,
    ReplyView,
    SyncStream,
    UsageView,
)


def report(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/completion/handler.py#L128-L140
    fixture: Fixture,
    recorder: Recorder,
    outcome: str,
    reply: JsonValue,
    chunks: JsonValue,
    error: JsonValue,
) -> JsonValue:
    return {
        "outcome": outcome,
        "requests": fixture.requests,
        "pre": recorder.pre,
        "post": recorder.post,
        "reply": reply,
        "chunks": chunks,
        "error": error,
    }


def error_view(error: AzureOpenAIError) -> JsonValue:
    view = cast(ErrorView, error)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L46-L63
    return {
        "status": view.status_code,
        "message": view.message,
        "retry_after": header_value(view.headers, "retry-after"),
    }


def header_value(headers: httpx.Headers | dict[str, str] | None, name: str) -> str | None:
    if headers is None:
        return None
    return headers.get(name)


def detail_value(usage: UsageView) -> JsonValue:
    details = usage.get("completion_tokens_details")
    if details is None:
        return None
    return details.get("reasoning_tokens")


def usage_reasoning(usage: UsageView) -> JsonValue:
    direct = usage.get("reasoning_tokens")
    if direct is not None:
        return direct
    return detail_value(usage)


def usage_view(usage: UsageView | None) -> JsonValue:
    if usage is None:
        return None
    prompt_tokens = usage.get("prompt_tokens")
    if prompt_tokens is None:
        prompt_tokens = 0
    completion_tokens = usage.get("completion_tokens")
    if completion_tokens is None:
        completion_tokens = 0
    total_tokens = usage.get("total_tokens")
    if total_tokens is None:
        total_tokens = 0
    return {
        "prompt_tokens": prompt_tokens,
        "completion_tokens": completion_tokens,
        "total_tokens": total_tokens,
        "reasoning_tokens": usage_reasoning(usage),
        "queue_time": usage.get("queue_time"),
        "details": usage.get("completion_tokens_details") is not None,
    }


def choice_view(choice: ReplyChoiceView) -> JsonValue:
    return {
        "index": choice["index"],
        "finish_reason": choice["finish_reason"],
        "content": choice["message"].get("content"),
        "role": choice["message"]["role"],
        "tool_calls": choice["message"].get("tool_calls"),
        "function_call": choice["message"].get("function_call"),
    }


def reply_view(view: ReplyView) -> JsonValue:
    return {
        "id": view["id"],
        "model": view["model"],
        "created_is_current": view["created"] > CREATED_FLOOR,
        "choices": [choice_view(choice) for choice in view["choices"]],
        "usage": usage_view(view["usage"]),
    }


def chunk_view(view: ChunkView) -> JsonValue:
    choice = view["choices"][0]
    return {
        "content": choice["delta"].get("content"),
        "role": choice["delta"].get("role"),
        "finish_reason": choice["finish_reason"],
        "index": choice["index"],
        "model": view["model"],
        "created_is_current": view["created"] > CREATED_FLOOR,
    }


def sync_chunk(stream: SyncStream) -> JsonValue:
    try:
        chunk = next(stream)
    except StopIteration:
        return None
    return chunk_view(CHUNK.validate_json(chunk.model_dump_json()))


async def next_chunk(stream: AsyncStream) -> JsonValue:
    try:
        chunk = await anext(stream)
    except StopAsyncIteration:
        return None
    return chunk_view(CHUNK.validate_json(chunk.model_dump_json()))


def sync_chunks(stream: SyncStream) -> list[JsonValue]:
    chunks: list[JsonValue] = []
    chunk = sync_chunk(stream)
    while chunk is not None:
        chunks.append(chunk)
        chunk = sync_chunk(stream)
    return chunks


async def async_chunks(stream: AsyncStream) -> list[JsonValue]:
    chunks: list[JsonValue] = []
    chunk = await next_chunk(stream)
    while chunk is not None:
        chunks.append(chunk)
        chunk = await next_chunk(stream)
    return chunks
