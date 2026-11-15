from typing import Final, TypedDict

from litellm.llms.groq.chat.transformation import GroqChatCompletionStreamingHandler
from litellm.llms.openai.common_utils import OpenAIError
from litellm.types.utils import ModelResponseStream
from pydantic import BaseModel, JsonValue, TypeAdapter

from .views import ARBITRARY, ChunkParser
from .wire import Call, Object

HANDLER: ChunkParser = TypeAdapter(ChunkParser, config=ARBITRARY).validate_python(
    GroqChatCompletionStreamingHandler(
        streaming_response=iter(()),
        sync_stream=True,
        json_mode=False,
    )
)
# LiteLLM's class carries Any in its constructor, so the catch names it through a typed alias
CHUNK_ERROR: Final[type[Exception]] = OpenAIError


class Failure(BaseModel):
    status_code: int | None


class ChunkDelta(TypedDict, total=False):
    content: str | None
    reasoning_content: str | None


class ChunkChoice(TypedDict, total=False):
    delta: ChunkDelta


class ChunkRecord(TypedDict, total=False):
    choices: list[ChunkChoice]


CHUNK_RECORD: TypeAdapter[ChunkRecord] = TypeAdapter(ChunkRecord)


def chunk_value(call: Call) -> JsonValue:
    # an error chunk becomes an item, so the case compares the parsed chunks beside the status
    try:
        parsed: ModelResponseStream = HANDLER.chunk_parser(call["chunk"])
    except CHUNK_ERROR as error:
        record = Failure.model_validate(error, from_attributes=True)
        failure: Object = {"error": {"status": record.status_code}}
        return failure
    choices = CHUNK_RECORD.validate_json(parsed.model_dump_json()).get("choices")
    inner = None if not choices else choices[0].get("delta")
    delta: ChunkDelta = {} if inner is None else inner
    value: Object = {
        "content": delta.get("content"),
        "reasoning_content": delta.get("reasoning_content"),
    }
    return value
