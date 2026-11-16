import asyncio
from collections.abc import AsyncGenerator, AsyncIterator
from contextlib import aclosing
from typing import (
    Final,
    Literal,
    NotRequired,
    Protocol,
    TypeAlias,
    TypedDict,
    runtime_checkable,
)

import httpx
import litellm
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.guardrails.guardrail_hooks.model_armor.model_armor import (
    ModelArmorGuardrail,
)
from litellm.types.guardrails import GuardrailEventHooks
from litellm.types.llms.openai import (
    ErrorEvent,
    FunctionCallArgumentsDeltaEvent as ArgsDeltaEvent,
    OutputTextDeltaEvent,
    ReasoningSummaryTextDeltaEvent,
    ResponseCompletedEvent,
    ResponseCreatedEvent,
)
from litellm.types.utils import ModelResponseStream
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter

# The Anthropic replay's unknown-model banner would corrupt the helper's JSON on stdout.
litellm.suppress_debug_info = True

GUARDRAIL_NAME: Final = "model-armor"
TEMPLATE_ID: Final = "t"
PROJECT_ID: Final = "p"
LOCATION: Final = "us-central1"
RESPONSE_KEY: Final = "_model_armor_response"
STATUS_KEY: Final = "_model_armor_status"
APPLIED_KEY: Final = "applied_guardrails"


class ChatCase(TypedDict):
    surface: Literal["chat"]
    chunks: list[JsonValue]
    mask_response_content: bool
    fail_on_error: bool
    sanitize_error_detail: bool
    answer_status: int
    answer_body: str


class AnthropicCase(TypedDict):
    surface: Literal["anthropic"]
    chunks: list[str]
    mask_response_content: bool
    fail_on_error: bool
    sanitize_error_detail: bool
    answer_status: int
    answer_body: str


class ResponsesCase(TypedDict):
    surface: Literal["responses"]
    chunks: list[JsonValue]
    mask_response_content: bool
    fail_on_error: bool
    sanitize_error_detail: bool
    answer_status: int
    answer_body: str


Input: TypeAlias = ChatCase | AnthropicCase | ResponsesCase

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


ARBITRARY: Final = ConfigDict(arbitrary_types_allowed=True)


class UnknownEventError(Exception):
    pass


class TokenProvider:
    def __init__(self, access_token: str, project_id: str) -> None:
        self.access_token = access_token
        self.project_id = project_id

    async def __call__(self) -> tuple[str, str]:
        return (self.access_token, self.project_id)


CHAT_IN: Final[TypeAdapter[ModelResponseStream]] = TypeAdapter(ModelResponseStream)
CREATED: Final[TypeAdapter[ResponseCreatedEvent]] = TypeAdapter(ResponseCreatedEvent)
COMPLETED: Final[TypeAdapter[ResponseCompletedEvent]] = TypeAdapter(ResponseCompletedEvent)
TEXT_DELTA: Final[TypeAdapter[OutputTextDeltaEvent]] = TypeAdapter(OutputTextDeltaEvent)
REASONING_DELTA: Final[TypeAdapter[ReasoningSummaryTextDeltaEvent]] = TypeAdapter(
    ReasoningSummaryTextDeltaEvent
)
ARGS_DELTA: Final[TypeAdapter[ArgsDeltaEvent]] = TypeAdapter(ArgsDeltaEvent)
CHAT_OUT: Final[TypeAdapter[ChatChunk]] = TypeAdapter(ChatChunk)
EVENT_OUT: Final[TypeAdapter[EventChunk]] = TypeAdapter(EventChunk)
ERROR_OUT: Final[TypeAdapter[ErrorChunk]] = TypeAdapter(ErrorChunk)
FRAME: Final[TypeAdapter[FrameData | None]] = TypeAdapter(FrameData | None)
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
BUCKET: Final[TypeAdapter[dict[str, JsonValue] | None]] = TypeAdapter(
    dict[str, JsonValue] | None
)


class Recorder(httpx.MockTransport):
    def __init__(self, status: int, body: str) -> None:
        super().__init__(self.handle)
        self.status = status
        self.body = body
        self.requests: list[httpx.Request] = []

    def handle(self, request: httpx.Request) -> httpx.Response:
        self.requests.append(request)
        return httpx.Response(self.status, text=self.body, request=request)


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
    match request["surface"]:
        case "chat":
            chat: list[OutputItem] = [
                CHAT_IN.validate_python(chunk) for chunk in request["chunks"]
            ]
            return chat
        case "responses":
            responses: list[OutputItem] = [
                response_event(chunk) for chunk in request["chunks"]
            ]
            return responses
        case "anthropic":
            texts: list[OutputItem] = []
            texts.extend(request["chunks"])
            return texts


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
    match item:
        case ModelResponseStream():
            return chat_view(CHAT_OUT.validate_json(item.model_dump_json()))
        case str():
            return frame_view(item)
        case bytes():
            return frame_view(item.decode("utf-8"))
        case ErrorEvent():
            return error_view(ERROR_OUT.validate_json(item.model_dump_json()))
        case BaseModel():
            return event_view(EVENT_OUT.validate_json(item.model_dump_json()))


def metadata_view(request_data: dict[str, JsonValue]) -> JsonValue:
    bucket = BUCKET.validate_python(request_data.get("metadata"))
    if bucket is None:
        return {"response": None, "status": None, "applied": None}
    return {
        "response": bucket.get(RESPONSE_KEY),
        "status": bucket.get(STATUS_KEY),
        "applied": bucket.get(APPLIED_KEY),
    }


def scanned(recorder: Recorder) -> JsonValue:
    if not recorder.requests:
        return None
    body = JSON.validate_json(recorder.requests[0].content)
    match body:
        case {"modelResponseData": {"text": text}}:
            return text
        case None | bool() | int() | float() | str() | list() | dict():
            return None


async def run_case(hook: Hook, recorder: Recorder, request: Input) -> JsonValue:
    guardrails: list[JsonValue] = [GUARDRAIL_NAME]
    metadata: dict[str, JsonValue] = {"guardrails": guardrails}
    request_data: dict[str, JsonValue] = {"metadata": metadata}
    items = build_items(request)
    chunks: list[JsonValue] = []
    streamed = hook.async_post_call_streaming_iterator_hook(
        UserAPIKeyAuth(), feed(items), request_data
    )
    async with aclosing(streamed) as stream:
        async for item in stream:
            chunks.append(project(item))
    return {
        "items": chunks,
        "scanned": scanned(recorder),
        "metadata": metadata_view(request_data),
    }


def build_guardrail(request: Input, recorder: Recorder) -> ModelArmorGuardrail:
    return ModelArmorGuardrail(
        template_id=TEMPLATE_ID,
        project_id=PROJECT_ID,
        location=LOCATION,
        access_token_provider=TokenProvider("credential", PROJECT_ID),
        async_handler=AsyncHTTPHandler(transport=recorder),
        sanitize_error_detail=request["sanitize_error_detail"],
        guardrail_name=GUARDRAIL_NAME,
        event_hook=[GuardrailEventHooks.post_call],
        mask_response_content=request["mask_response_content"],
        fail_on_error=request["fail_on_error"],
    )


def as_hook(guardrail: ModelArmorGuardrail) -> Hook:
    adapter: Final[TypeAdapter[Hook]] = TypeAdapter[Hook](Hook, config=ARBITRARY)
    return adapter.validate_python(guardrail)


def run(input_value: Input) -> JsonValue:
    recorder = Recorder(input_value["answer_status"], input_value["answer_body"])
    guardrail = as_hook(build_guardrail(input_value, recorder))
    return asyncio.run(run_case(guardrail, recorder, input_value))
