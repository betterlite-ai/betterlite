from typing import Literal, Protocol, TypedDict

from litellm.llms.a2a.chat.streaming_iterator import A2AModelResponseIterator
from litellm.llms.a2a.common_utils import A2AError
from litellm.types.utils import GenericStreamingChunk, ModelResponseStream
from pydantic import JsonValue, TypeAdapter

from .support import Body, error_view


class ChunkView(TypedDict):
    text: str
    is_finished: bool
    finish_reason: str
    index: int


class StreamCase(TypedDict):
    chunk: Body


class StreamInput(TypedDict):
    op: Literal["stream"]
    chunks: list[StreamCase]


class StreamParser(Protocol):
    def chunk_parser(self, chunk: Body) -> GenericStreamingChunk | ModelResponseStream: ...


CHUNK_VIEW: TypeAdapter[ChunkView] = TypeAdapter(ChunkView)


def chunk_fields(value: Body) -> JsonValue:
    parser: StreamParser = A2AModelResponseIterator(iter(()), sync_stream=False)
    found = CHUNK_VIEW.validate_python(parser.chunk_parser(value))  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/a2a/chat/streaming_iterator.py#L34
    return {
        "chunk": {
            "text": found["text"],
            "is_finished": found["is_finished"],
            "finish_reason": found["finish_reason"],
            "index": found["index"],
        }
    }


def stream_chunks(input_value: StreamInput) -> JsonValue:
    chunks: list[JsonValue] = []
    for case in input_value["chunks"]:
        try:
            chunks.append(chunk_fields(case["chunk"]))
        except A2AError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/a2a/common_utils.py#L18
            chunks.append(error_view(error))
    return {"chunks": chunks}
