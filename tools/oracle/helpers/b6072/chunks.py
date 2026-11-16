from collections.abc import Iterator
from typing import Final, NotRequired, TypedDict

from litellm.types.utils import ModelResponseStream
from pydantic import JsonValue, TypeAdapter

from .views import CHAT, ChunkHandler
from .wire import ChunkCall, Object

NO_STREAM: Final[Iterator[str]] = iter(())
HANDLER: ChunkHandler = CHAT.get_model_response_iterator(NO_STREAM, sync_stream=True, json_mode=False)


class ChunkDelta(TypedDict, total=False):
    role: str | None
    content: str | None


class ChunkChoice(TypedDict, total=False):
    index: int | None
    finish_reason: str | None
    delta: ChunkDelta


class ChunkRecord(TypedDict):
    choices: list[ChunkChoice]
    id: NotRequired[str | None]
    object: NotRequired[str | None]
    created: NotRequired[int | None]
    model: NotRequired[str | None]
    provider_specific_fields: NotRequired[Object | None]


CHUNK_RECORD: TypeAdapter[ChunkRecord] = TypeAdapter(ChunkRecord)


def chunk_delta(choice: ChunkChoice) -> ChunkDelta:
    given = choice.get("delta")
    if given is None:
        return {}
    return given


def chunk_choice_value(choice: ChunkChoice) -> JsonValue:
    delta = chunk_delta(choice)
    index = choice.get("index")
    finish_reason = choice.get("finish_reason")
    return {
        "index": index,
        "finish_reason": finish_reason,
        "role": delta.get("role"),
        "content": delta.get("content"),
    }


def chunk_value(call: ChunkCall) -> JsonValue:
    parsed: ModelResponseStream = HANDLER.chunk_parser(call["chunk"])
    record = CHUNK_RECORD.validate_json(parsed.model_dump_json())
    return {
        "id": record.get("id"),
        "object": record.get("object"),
        "created": record.get("created"),
        "model": record.get("model"),
        "choices": [chunk_choice_value(choice) for choice in record["choices"]],
        "provider_specific_fields": record.get("provider_specific_fields"),
    }
