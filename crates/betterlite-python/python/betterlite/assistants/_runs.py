import json
from collections.abc import AsyncIterator, Awaitable, Callable, Coroutine, Iterator, Mapping
from types import TracebackType
from typing import Final, Protocol, Self, TypeAlias, TypeVar, runtime_checkable

from pydantic import JsonValue

from betterlite._completion.errors import UnsupportedCallArgument, raise_native_error
from betterlite._endpoints.payload import CallPayload, CallValue, to_call_payload
from betterlite._endpoints.settings import read_settings
from betterlite._native import (
    NativeAssistantRunError,
    NativeAssistantRunSession,
    NativeAssistantStream,
    NativeError,
)
from betterlite.assistants._records import DROPPED, JSON, PARAMS, SETTINGS, param_value
from betterlite.types.utils.base import ResponseObject

CONNECTION_PARAMS: Final = (*PARAMS, "extra_body")
# run_thread sends the request fields; the stream call sets stream itself
RUN_FIELDS: Final = ("additional_instructions", "instructions", "metadata", "model", "tools")
MESSAGE_NAMED: Final = ("attachments", "metadata")


class AssistantStreamEvent(ResponseObject):
    event: str
    data: JsonValue


class Message(ResponseObject):
    id: str
    assistant_id: str | None = None
    attachments: list[JsonValue] | None = None
    completed_at: int | None = None
    content: list[JsonValue]
    created_at: int
    incomplete_at: int | None = None
    incomplete_details: JsonValue = None
    metadata: dict[str, str] | None = None
    object: str
    role: str
    run_id: str | None = None
    status: str | None = None
    thread_id: str


class MessagePage(ResponseObject):
    data: list[Message]
    first_id: str | None = None
    has_more: bool | None = None
    last_id: str | None = None
    object: str | None = None


class Run(ResponseObject):
    id: str
    assistant_id: str
    cancelled_at: int | None = None
    completed_at: int | None = None
    created_at: int
    expires_at: int | None = None
    failed_at: int | None = None
    incomplete_details: JsonValue = None
    instructions: str
    last_error: JsonValue = None
    max_completion_tokens: int | None = None
    max_prompt_tokens: int | None = None
    metadata: dict[str, str] | None = None
    model: str
    object: str
    parallel_tool_calls: bool
    required_action: JsonValue = None
    response_format: JsonValue = None
    started_at: int | None = None
    status: str
    thread_id: str
    tool_choice: JsonValue = None
    tools: list[JsonValue]
    truncation_strategy: JsonValue = None
    usage: JsonValue = None
    temperature: float | None = None
    top_p: float | None = None


@runtime_checkable
class EventHandler(Protocol):
    def on_event(self, event: AssistantStreamEvent) -> None: ...


RecordT = TypeVar("RecordT", bound=ResponseObject)
SyncMethod: TypeAlias = Callable[[NativeAssistantRunSession, str, list[bytes]], str]
AsyncMethod: TypeAlias = Callable[[NativeAssistantRunSession, str, list[bytes]], Awaitable[str]]


def checked_handler(value: EventHandler | CallValue) -> EventHandler | None:
    if value is None:
        return None
    match value:
        case EventHandler() as handler:
            return handler
        case object():
            raise UnsupportedCallArgument("event_handler")


# OpenAI sends the event name beside its payload; a bare payload is named "payload" here.
def payload_event(value: JsonValue) -> AssistantStreamEvent:
    match value:
        case dict() as record if "event" in record:
            return AssistantStreamEvent.model_validate(record)
        case object() as payload:
            wrapped: dict[str, JsonValue] = {"event": "payload", "data": payload}
            return AssistantStreamEvent.model_validate(wrapped)


def stream_event(text: str, handler: EventHandler | None) -> AssistantStreamEvent:
    event = payload_event(JSON.validate_json(text))
    if handler is not None:
        handler.on_event(event)
    return event


def call_values(
    provider: CallValue, thread_id: CallValue, arguments: Mapping[str, CallValue],
) -> dict[str, CallValue]:
    values: dict[str, CallValue] = {
        "custom_llm_provider": provider,
        "thread_id": thread_id,
        "params": {
            name: param_value(name, arguments[name])
            for name in CONNECTION_PARAMS
            if name in arguments
        },
    }
    values.update({name: arguments.get(name) for name in DROPPED})
    return values


def message_payload(
    provider: str, thread_id: str, role: CallValue, content: CallValue,
    arguments: Mapping[str, CallValue],
) -> CallPayload:
    values = call_values(provider, thread_id, arguments)
    data: dict[str, CallValue] = {"role": role, "content": content}
    data.update({name: arguments[name] for name in MESSAGE_NAMED if name in arguments})
    values["data"] = data
    values["drop_params"] = read_settings(("drop_params",))["drop_params"] is True
    return to_call_payload(values)


def run_payload(arguments: Mapping[str, CallValue]) -> CallPayload:
    # litellm's stream wrapper forwards stream to run_thread, which sets stream and rejects it
    if "stream" in arguments:
        raise TypeError("run_thread() got multiple values for keyword argument 'stream'")
    values = call_values(arguments["custom_llm_provider"], arguments["thread_id"], arguments)
    values["assistant_id"] = arguments["assistant_id"]
    values.update({name: arguments[name] for name in RUN_FIELDS if name in arguments})
    return to_call_payload(values)


def run_session() -> NativeAssistantRunSession:
    settings = read_settings(SETTINGS)
    return NativeAssistantRunSession(json.dumps(settings))


def validated_answer(model: type[RecordT], method: SyncMethod, payload: CallPayload) -> RecordT:
    session = run_session()
    try:
        text = method(session, payload.text, payload.blobs)
    except NativeAssistantRunError as failure:
        raise_native_error(NativeError(str(failure)))
    return model.model_validate(JSON.validate_json(text))


async def validated_aanswer(
    model: type[RecordT], method: AsyncMethod, payload: CallPayload,
) -> RecordT:
    session = run_session()
    try:
        text = await method(session, payload.text, payload.blobs)
    except NativeAssistantRunError as failure:
        raise_native_error(NativeError(str(failure)))
    return model.model_validate(JSON.validate_json(text))


# litellm reports a message its provider left without a status as completed
def completed_message(message: Message) -> Message:
    if message.status is None:
        message.status = "completed"
    return message


# a read that fails raises the class the native error payload names, like every other call
def next_event(events: NativeAssistantStream) -> str | None:
    try:
        return events.next_event()
    except NativeAssistantRunError as failure:
        raise_native_error(NativeError(str(failure)))


async def anext_event(events: NativeAssistantStream) -> str | None:
    try:
        return await events.anext_event()
    except NativeAssistantRunError as failure:
        raise_native_error(NativeError(str(failure)))


class AssistantStream:
    def __init__(self, events: NativeAssistantStream, handler: EventHandler | None) -> None:
        self._events = events
        self._handler = handler

    def __enter__(self) -> Self:
        return self

    def __exit__(
        self, _exc_type: type[BaseException] | None, _exc_value: BaseException | None,
        _traceback: TracebackType | None,
    ) -> None:
        self._events.close()

    def __iter__(self) -> Iterator[AssistantStreamEvent]:
        return self

    def __next__(self) -> AssistantStreamEvent:
        text = next_event(self._events)
        if text is None:
            raise StopIteration
        return stream_event(text, self._handler)


class AsyncAssistantStream:
    def __init__(self, events: NativeAssistantStream, handler: EventHandler | None) -> None:
        self._events = events
        self._handler = handler

    async def __aenter__(self) -> Self:
        return self

    async def __aexit__(
        self, _exc_type: type[BaseException] | None, _exc_value: BaseException | None,
        _traceback: TracebackType | None,
    ) -> None:
        await self._events.aclose()

    def __aiter__(self) -> AsyncIterator[AssistantStreamEvent]:
        return self

    async def __anext__(self) -> AssistantStreamEvent:
        text = await anext_event(self._events)
        if text is None:
            raise StopAsyncIteration
        return stream_event(text, self._handler)


def open_stream(payload: CallPayload, *, asynchronous: bool) -> NativeAssistantStream:
    session = run_session()
    try:
        return session.run_stream(payload.text, payload.blobs, asynchronous)
    except NativeAssistantRunError as failure:
        raise_native_error(NativeError(str(failure)))


# run_thread answers the run, either stream, or the coroutine the arun_thread flag asks for
RunAnswer: TypeAlias = (
    Run | AssistantStream | AsyncAssistantStream | Coroutine[None, None, Run]
)
MessageAnswer: TypeAlias = Message | Coroutine[None, None, Message]
MessagePageAnswer: TypeAlias = MessagePage | Coroutine[None, None, MessagePage]
StreamAnswer: TypeAlias = AssistantStream | AsyncAssistantStream
