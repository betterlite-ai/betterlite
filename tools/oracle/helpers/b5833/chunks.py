from typing import TypedDict

from litellm.llms.groq.chat.transformation import GroqChatCompletionStreamingHandler
from litellm.types.utils import ModelResponseStream
from pydantic import JsonValue, TypeAdapter

from .views import ChunkParser
from .wire import Call, Object

HANDLER: ChunkParser = GroqChatCompletionStreamingHandler(
    streaming_response=iter(()),
    sync_stream=True,
    json_mode=False,
)


class ChunkDelta(TypedDict, total=False):
    content: str | None
    reasoning_content: str | None


class ChunkChoice(TypedDict, total=False):
    delta: ChunkDelta


class ChunkRecord(TypedDict, total=False):
    choices: list[ChunkChoice]


CHUNK_RECORD: TypeAdapter[ChunkRecord] = TypeAdapter(ChunkRecord)


def chunk_value(call: Call) -> JsonValue:
    parsed: ModelResponseStream = HANDLER.chunk_parser(call["chunk"])
    record = CHUNK_RECORD.validate_json(parsed.model_dump_json())
    delta: ChunkDelta = {}
    choices = record.get("choices")
    if choices:
        inner = choices[0].get("delta")
        if inner is not None:
            delta = inner
    value: Object = {
        "content": delta.get("content"),
        "reasoning_content": delta.get("reasoning_content"),
    }
    return value
