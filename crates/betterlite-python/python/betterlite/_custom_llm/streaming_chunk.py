from pydantic import JsonValue
from typing_extensions import Required, TypedDict

from betterlite.types.llms.openai import ChatCompletionToolCallChunk, ChatCompletionUsageBlock

__all__ = ["GenericStreamingChunk"]


class GenericStreamingChunk(TypedDict, total=False):
    text: Required[str]
    tool_use: ChatCompletionToolCallChunk | None
    is_finished: Required[bool]
    finish_reason: Required[str]
    usage: Required[ChatCompletionUsageBlock | None]
    index: int
    provider_specific_fields: dict[str, JsonValue] | None
