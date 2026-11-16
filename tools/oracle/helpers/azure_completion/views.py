from typing import Literal, cast

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
    JsonDump,
    ReplyChoiceView,
    ReplyView,
    SyncStream,
    UsageView,
)


def report(fixture: Fixture, recorder: Recorder, outcome: str, **view: JsonValue) -> JsonValue:
    return {
        "outcome": outcome,
        "requests": fixture.requests,
        "pre": recorder.pre,
        "post": recorder.post,
        "reply": view.get("reply"),
        "chunks": view.get("chunks"),
        "error": view.get("error"),
    }


def retry_after(headers: httpx.Headers | dict[str, str] | None) -> str | None:
    if headers is None or "retry-after" not in headers:
        return None
    return headers["retry-after"]


def error_view(error: AzureOpenAIError) -> JsonValue:
    view = cast(ErrorView, error)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L46-L63
    return {
        "status": view.status_code,
        "message": view.message,
        "retry_after": retry_after(view.headers),
    }


def usage_reasoning(usage: UsageView) -> JsonValue:
    direct = usage.get("reasoning_tokens")
    if direct is not None:
        return direct
    details = usage.get("completion_tokens_details")
    if details is None:
        return None
    return details.get("reasoning_tokens")


def usage_count(
    usage: UsageView, name: Literal["prompt_tokens", "completion_tokens", "total_tokens"]
) -> int:
    if name in usage:
        return usage[name]
    return 0


def usage_view(usage: UsageView | None) -> JsonValue:
    if usage is None:
        return None
    return {
        "prompt_tokens": usage_count(usage, "prompt_tokens"),
        "completion_tokens": usage_count(usage, "completion_tokens"),
        "total_tokens": usage_count(usage, "total_tokens"),
        "reasoning_tokens": usage_reasoning(usage),
        "queue_time": usage.get("queue_time"),
        "details": usage.get("completion_tokens_details") is not None,
    }


def choice_view(choice: ReplyChoiceView) -> JsonValue:
    view: dict[str, JsonValue] = {
        "index": choice["index"],
        "finish_reason": choice["finish_reason"],
    }
    for name in ("content", "role", "tool_calls", "function_call"):
        view[name] = choice["message"].get(name)
    return view


def reply_view(view: ReplyView) -> JsonValue:
    created = view["created"] > CREATED_FLOOR
    choices: list[JsonValue] = [choice_view(choice) for choice in view["choices"]]
    return {
        "id": view["id"],
        "model": view["model"],
        "created_is_current": created,
        "choices": choices,
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


async def stream_chunks(
    result: AsyncStream | JsonDump | SyncStream, *, asynchronous: bool
) -> list[JsonValue]:
    if asynchronous:
        return [chunk_view(CHUNK.validate_json(chunk.model_dump_json())) async for chunk in cast(AsyncStream, result)]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/completion/handler.py#L85-L98
    return [chunk_view(CHUNK.validate_json(chunk.model_dump_json())) for chunk in cast(SyncStream, result)]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/completion/handler.py#L114-L125
