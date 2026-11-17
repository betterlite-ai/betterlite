from collections.abc import AsyncGenerator, AsyncIterator
from typing import (
    Final,
    Literal,
    NotRequired,
    Protocol,
    TypeAlias,
    TypedDict,
    runtime_checkable,
)

from litellm.proxy._types import UserAPIKeyAuth
from litellm.types.llms.openai import (
    ErrorEvent,
    OutputTextDeltaEvent,
    ReasoningSummaryTextDeltaEvent,
    ResponseCompletedEvent,
    ResponseCreatedEvent,
)
from litellm.types.llms.openai import (
    FunctionCallArgumentsDeltaEvent as ArgsDeltaEvent,
)
from litellm.types.utils import ModelResponseStream
from pydantic import JsonValue, TypeAdapter

RESPONSE_KEY: Final = "_model_armor_response"
STATUS_KEY: Final = "_model_armor_status"
APPLIED_KEY: Final = "applied_guardrails"


class Input(TypedDict):
    surface: Literal["chat", "anthropic", "responses"]
    chunks: list[JsonValue]
    mask_response_content: bool
    fail_on_error: bool
    sanitize_error_detail: bool
    answer_status: int
    answer_body: str


ResponseEvent: TypeAlias = (
    ResponseCreatedEvent
    | ResponseCompletedEvent
    | OutputTextDeltaEvent
    | ReasoningSummaryTextDeltaEvent
    | ArgsDeltaEvent
)
OutputItem: TypeAlias = ModelResponseStream | ResponseEvent | ErrorEvent | str | bytes


class ChatDelta(TypedDict):
    role: NotRequired[str | None]
    content: NotRequired[JsonValue]


class ChatChoice(TypedDict):
    index: NotRequired[JsonValue]
    finish_reason: NotRequired[JsonValue]
    delta: ChatDelta


class ChatChunk(TypedDict):
    id: JsonValue
    object: JsonValue
    model: JsonValue
    created: JsonValue
    choices: list[ChatChoice]


class EventError(TypedDict):
    type: str
    code: str
    message: str
    param: JsonValue


class EventChunk(TypedDict):
    type: str
    delta: NotRequired[JsonValue]
    item_id: NotRequired[JsonValue]
    output_index: NotRequired[JsonValue]
    content_index: NotRequired[JsonValue]
    summary_index: NotRequired[JsonValue]


class ErrorChunk(TypedDict):
    type: str
    error: EventError


class StartMessage(TypedDict):
    id: str


class ModelResponseData(TypedDict, total=False):
    text: str | None


class ScanBody(TypedDict, total=False):
    modelResponseData: ModelResponseData


class FrameDelta(TypedDict):
    type: NotRequired[str]
    text: NotRequired[str]
    stop_reason: NotRequired[str | None]


class FrameBlock(TypedDict):
    type: NotRequired[str]


class FrameError(TypedDict):
    type: NotRequired[str]
    message: NotRequired[str]


class FrameData(TypedDict):
    type: NotRequired[str]
    message: NotRequired[StartMessage]
    delta: NotRequired[FrameDelta]
    content_block: NotRequired[FrameBlock]
    error: NotRequired[FrameError]


@runtime_checkable
class Hook(Protocol):
    def async_post_call_streaming_iterator_hook(  # ast-grep-ignore: no-long-names
        self,
        user_api_key_dict: UserAPIKeyAuth,
        response: AsyncIterator[OutputItem],
        request_data: dict[str, JsonValue],
    ) -> AsyncGenerator[OutputItem, None]: ...


class UnknownEventError(Exception):
    pass


ITEMS: Final[TypeAdapter[OutputItem]] = TypeAdapter(OutputItem)
SCAN: Final[TypeAdapter[ScanBody]] = TypeAdapter(ScanBody)
CHAT_IN: Final[TypeAdapter[ModelResponseStream]] = TypeAdapter(ModelResponseStream)
CREATED: Final[TypeAdapter[ResponseCreatedEvent]] = TypeAdapter(ResponseCreatedEvent)
COMPLETED: Final[TypeAdapter[ResponseCompletedEvent]] = TypeAdapter(ResponseCompletedEvent)
TEXT_DELTA: Final[TypeAdapter[OutputTextDeltaEvent]] = TypeAdapter(OutputTextDeltaEvent)
REASONING_DELTA: Final[TypeAdapter[ReasoningSummaryTextDeltaEvent]] = TypeAdapter(
    ReasoningSummaryTextDeltaEvent
)
ARGS_DELTA: Final[TypeAdapter[ArgsDeltaEvent]] = TypeAdapter(ArgsDeltaEvent)
STR: Final[TypeAdapter[str]] = TypeAdapter(str)
CHAT_OUT: Final[TypeAdapter[ChatChunk]] = TypeAdapter(ChatChunk)
EVENT_OUT: Final[TypeAdapter[EventChunk]] = TypeAdapter(EventChunk)
ERROR_OUT: Final[TypeAdapter[ErrorChunk]] = TypeAdapter(ErrorChunk)
FRAME: Final[TypeAdapter[FrameData | None]] = TypeAdapter(FrameData | None)
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
BUCKET: Final[TypeAdapter[dict[str, JsonValue] | None]] = TypeAdapter(
    dict[str, JsonValue] | None
)


def response_event(payload: JsonValue) -> ResponseEvent:
    match payload:
        case {"type": "response.created"}:
            return CREATED.validate_python(payload)
        case {"type": "response.completed"}:
            return COMPLETED.validate_python(payload)
        case {"type": "response.output_text.delta"}:
            return TEXT_DELTA.validate_python(payload)
        case {"type": "response.reasoning_summary_text.delta"}:
            return REASONING_DELTA.validate_python(payload)
        case {"type": "response.function_call_arguments.delta"}:
            return ARGS_DELTA.validate_python(payload)
        case None | bool() | int() | float() | str() | list() | dict():
            raise UnknownEventError


def build_items(request: Input) -> list[OutputItem]:
    items: list[OutputItem] = []
    match request["surface"]:
        case "chat":
            items = [CHAT_IN.validate_python(chunk) for chunk in request["chunks"]]
        case "responses":
            items = [response_event(chunk) for chunk in request["chunks"]]
        case "anthropic":
            items = [STR.validate_python(chunk) for chunk in request["chunks"]]
    return items


async def feed(items: list[OutputItem]) -> AsyncIterator[OutputItem]:
    for item in items:
        yield item


def chat_view(chunk: ChatChunk) -> JsonValue:
    choices: list[JsonValue] = [
        {
            "index": choice.get("index"),
            "finish_reason": choice.get("finish_reason"),
            "role": choice["delta"].get("role"),
            "content": choice["delta"].get("content"),
        }
        for choice in chunk["choices"]
    ]
    return {
        "kind": "chat",
        "id": chunk["id"],
        "object": chunk["object"],
        "model": chunk["model"],
        "created": chunk["created"],
        "choices": choices,
    }


def event_view(event: EventChunk) -> JsonValue:
    return {
        "kind": "event",
        "type": event["type"],
        "delta": event.get("delta"),
        "item_id": event.get("item_id"),
        "output_index": event.get("output_index"),
        "content_index": event.get("content_index"),
        "summary_index": event.get("summary_index"),
    }


def error_view(chunk: ErrorChunk) -> JsonValue:
    error = chunk["error"]
    return {
        "kind": "error_item",
        "error_type": error["type"],
        "code": error["code"],
        "message": error["message"],
        "param": error["param"],
    }


def frame_body(data: FrameData | None) -> JsonValue:
    if data is None:
        return {}
    message = data.get("message")
    delta = data.get("delta")
    block = data.get("content_block")
    error = data.get("error")
    return {
        "type": data.get("type"),
        "message_id": None if message is None else message["id"],
        "delta_type": None if delta is None else delta.get("type"),
        "text": None if delta is None else delta.get("text"),
        "stop_reason": None if delta is None else delta.get("stop_reason"),
        "block_type": None if block is None else block.get("type"),
        "error_type": None if error is None else error.get("type"),
        "error_message": None if error is None else error.get("message"),
    }


def frame_view(frame: str) -> JsonValue:
    event: str | None = None
    data: JsonValue = None
    for line in frame.split("\n"):
        if line.startswith("event:"):
            event = line[len("event:") :].strip()
        if line.startswith("data:"):
            data = JSON.validate_json(line[len("data:") :].strip())
    return {"kind": "sse", "event": event, "data": frame_body(FRAME.validate_python(data))}


def project(item: OutputItem) -> JsonValue:
    payload = JSON.validate_json(ITEMS.dump_json(item))
    match payload:
        case str():
            return frame_view(payload)
        case {"type": "error"}:
            return error_view(ERROR_OUT.validate_python(payload))
        case {"type": str(kind)} if kind.startswith("response."):
            return event_view(EVENT_OUT.validate_python(payload))
        case None | bool() | int() | float() | list() | dict():
            return chat_view(CHAT_OUT.validate_python(payload))


def metadata_view(request_data: dict[str, JsonValue]) -> JsonValue:
    bucket = BUCKET.validate_python(request_data.get("metadata"))
    if bucket is None:
        return {"response": None, "status": None, "applied": None}
    return {
        "response": bucket.get(RESPONSE_KEY),
        "status": bucket.get(STATUS_KEY),
        "applied": bucket.get(APPLIED_KEY),
    }
