from collections.abc import Iterator
from typing import Final, TypedDict

from litellm.types.utils import ModelResponseStream
from pydantic import JsonValue, TypeAdapter

from .views import CHAT, ChunkHandler
from .wire import ChunkCall, Object

NO_STREAM: Final[Iterator[str]] = iter(())
HANDLER: ChunkHandler = CHAT.get_model_response_iterator(
    NO_STREAM, sync_stream=True, json_mode=False
)


class ChunkDelta(TypedDict, total=False):
    role: str | None
    content: str | None


class ChunkChoice(TypedDict):
    index: int | None
    finish_reason: str | None
    delta: ChunkDelta


class ChunkRecord(TypedDict):
    id: str | None
    object: str | None
    created: int | None
    model: str | None
    choices: list[ChunkChoice]
    provider_specific_fields: Object | None


CHUNK_RECORD: TypeAdapter[ChunkRecord] = TypeAdapter(ChunkRecord)


def chunk_choice_value(choice: ChunkChoice) -> JsonValue:
    return {
        "index": choice["index"],
        "finish_reason": choice["finish_reason"],
        "role": choice["delta"].get("role"),
        "content": choice["delta"].get("content"),
    }


def chunk_value(call: ChunkCall) -> JsonValue:
    parsed: ModelResponseStream = HANDLER.chunk_parser(call["chunk"])
    record = CHUNK_RECORD.validate_json(parsed.model_dump_json())
    return {
        "id": record["id"],
        "object": record["object"],
        "created": record["created"],
        "model": record["model"],
        "choices": [chunk_choice_value(choice) for choice in record["choices"]],
        "provider_specific_fields": record["provider_specific_fields"],
    }
