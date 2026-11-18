import asyncio
from collections.abc import AsyncIterator
from typing import Final, NotRequired, TypeAlias, TypedDict

from litellm.llms.sagemaker.common_utils import AWSEventStreamDecoder
from litellm.types.utils import GenericStreamingChunk, StreamingChatCompletionChunk
from pydantic import JsonValue, TypeAdapter

from .types import DecodeStream

Chunk: TypeAlias = GenericStreamingChunk | StreamingChatCompletionChunk  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L434

DECODE_STREAM: Final = TypeAdapter(DecodeStream)
CHUNKS: Final = TypeAdapter(Chunk)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2057
JSON: Final = TypeAdapter(JsonValue)


class FunctionView(TypedDict):
    name: str | None
    arguments: str


class ToolCallView(TypedDict):
    index: int
    function: FunctionView
    id: NotRequired[str | None]
    type: NotRequired[str | None]


class DeltaView(TypedDict, total=False):
    content: str | None
    role: str | None
    tool_calls: list[ToolCallView] | None


class ChoiceView(TypedDict):
    index: int
    finish_reason: str | None
    delta: DeltaView


class ChatView(TypedDict):
    id: str
    created: int
    model: str
    object: str
    choices: list[ChoiceView]


class TokenView(TypedDict):
    text: str
    is_finished: bool
    finish_reason: str
    index: int


CHAT: Final = TypeAdapter(ChatView)
TOKEN: Final = TypeAdapter(TokenView)


def tool_calls_value(calls: list[ToolCallView]) -> JsonValue:
    return [
        {
            "index": call["index"],
            "id": call.get("id"),
            "type": call.get("type"),
            "function": {
                "name": call["function"]["name"],
                "arguments": call["function"]["arguments"],
            },
        }
        for call in calls
    ]


def choice_value(choice: ChoiceView) -> JsonValue:
    calls = choice["delta"].get("tool_calls")
    return {
        "index": choice["index"],
        "finish_reason": choice["finish_reason"],
        "content": choice["delta"].get("content"),
        "role": choice["delta"].get("role"),
        "tool_calls": None if calls is None else tool_calls_value(calls),
    }


def chat_value(value: JsonValue) -> JsonValue:
    view = CHAT.validate_python(value)
    choices: list[JsonValue] = [choice_value(choice) for choice in view["choices"]]
    return {
        "kind": "stream",
        "id": view["id"],
        "created": view["created"],
        "model": view["model"],
        "object": view["object"],
        "choices": choices,
    }


def token_value(value: JsonValue) -> JsonValue:
    view = TOKEN.validate_python(value)
    return {
        "kind": "generic",
        "text": view["text"],
        "is_finished": view["is_finished"],
        "finish_reason": view["finish_reason"],
        "usage": None,
        "index": view["index"],
    }


def chunk_value(item: Chunk | None, *, is_messages_api: bool) -> JsonValue:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L434
    if item is None:
        return {"kind": "none"}
    value = JSON.validate_json(CHUNKS.dump_json(item))  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2057
    if is_messages_api:
        return chat_value(value)
    return token_value(value)


async def chunk_source(chunks: list[bytes]) -> AsyncIterator[bytes]:
    for chunk in chunks:
        yield chunk


# an error that escapes the iterator ends the stream, as the native decoder reports it
def collect_sync(
    decoder: AWSEventStreamDecoder, chunks: list[bytes], *, is_messages_api: bool
) -> tuple[list[JsonValue], bool]:
    values: list[JsonValue] = []
    events = decoder.iter_bytes(iter(chunks))  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/sagemaker/common_utils.py#L92
    try:
        while True:
            item = next(events)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/sagemaker/common_utils.py#L92
            value = chunk_value(item, is_messages_api=is_messages_api)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/sagemaker/common_utils.py#L92
            values.append(value)
    except StopIteration:
        return values, False
    except Exception:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/sagemaker/common_utils.py#L99-L136
        return values, True


async def collect_async(
    decoder: AWSEventStreamDecoder, chunks: list[bytes], *, is_messages_api: bool
) -> tuple[list[JsonValue], bool]:
    values: list[JsonValue] = []
    try:
        async for item in decoder.aiter_bytes(chunk_source(chunks)):  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/sagemaker/common_utils.py#L137
            value = chunk_value(item, is_messages_api=is_messages_api)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/sagemaker/common_utils.py#L137
            values.append(value)
    except Exception:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/sagemaker/common_utils.py#L146-L190
        return values, True
    return values, False


def decode_stream(input_value: dict[str, JsonValue]) -> JsonValue:
    case = DECODE_STREAM.validate_python(input_value)
    chunks = [bytes.fromhex(text) for text in case["chunks"]]
    decoder = AWSEventStreamDecoder(case["model"], is_messages_api=case["is_messages_api"])
    if case["sync"]:
        values, failed = collect_sync(decoder, chunks, is_messages_api=case["is_messages_api"])
    else:
        values, failed = asyncio.run(
            collect_async(decoder, chunks, is_messages_api=case["is_messages_api"])
        )
    failure: JsonValue = {"status": None} if failed else None
    return {"items": values, "failure": failure}
