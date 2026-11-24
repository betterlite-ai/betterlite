import json
from collections.abc import AsyncIterator, Awaitable, Callable, Coroutine, Iterator, Mapping
from types import TracebackType
from typing import Final, Protocol, TypeAlias, TypeVar, runtime_checkable

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
from betterlite.assistants._records import (
    DROPPED,
    JSON,
    PARAMS,
    SETTINGS,
    named_arguments,
    param_value,
)
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
    has_more: bool
    last_id: str | None = None
    object: str

    def __iter__(self) -> Iterator[Message]:
        return iter(self.data)


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
StreamMethod: TypeAlias = Callable[
    [NativeAssistantRunSession, str, list[bytes]], NativeAssistantStream
]


def checked_handler(value: EventHandler | CallValue) -> EventHandler | None:
    if value is None:
        return None
    match value:
        case EventHandler() as handler:
            return handler
        case object():
            raise UnsupportedCallArgument("event_handler")


def stream_event(text: str) -> AssistantStreamEvent:
    value = JSON.validate_json(text)
    match value:
        case dict() as record if "event" in record:
            return AssistantStreamEvent.model_validate(record)
        case object() as payload:
            return AssistantStreamEvent.model_validate({"event": "payload", "data": payload})


def call_values(
    provider: str, thread_id: str, arguments: Mapping[str, CallValue],
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
    values.update({"role": role, "content": content})
    values["drop_params"] = read_settings(("drop_params",))["drop_params"] is True
    values.update({name: arguments[name] for name in MESSAGE_NAMED if name in arguments})
    return to_call_payload(values)


def run_payload(
    provider: str, thread_id: str, assistant_id: str,
    arguments: Mapping[str, CallValue],
) -> CallPayload:
    values = call_values(provider, thread_id, arguments)
    values["assistant_id"] = assistant_id
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

    def __enter__(self) -> AssistantStream:
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
        event = stream_event(text)
        if self._handler is not None:
            self._handler.on_event(event)
        return event


class AsyncAssistantStream:
    def __init__(self, events: NativeAssistantStream, handler: EventHandler | None) -> None:
        self._events = events
        self._handler = handler

    async def __aenter__(self) -> AsyncAssistantStream:
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
        event = stream_event(text)
        if self._handler is not None:
            self._handler.on_event(event)
        return event


def stream_answer(
    method: StreamMethod, payload: CallPayload, handler: EventHandler | None,
) -> AssistantStream:
    session = run_session()
    try:
        events = method(session, payload.text, payload.blobs)
    except NativeAssistantRunError as failure:
        raise_native_error(NativeError(str(failure)))
    return AssistantStream(events, handler)


def async_stream_answer(
    method: StreamMethod, payload: CallPayload, handler: EventHandler | None,
) -> AsyncAssistantStream:
    session = run_session()
    try:
        events = method(session, payload.text, payload.blobs)
    except NativeAssistantRunError as failure:
        raise_native_error(NativeError(str(failure)))
    return AsyncAssistantStream(events, handler)


RunAnswer: TypeAlias = Run | Coroutine[None, None, Run] | AssistantStream | AsyncAssistantStream
MessageAnswer: TypeAlias = Message | Coroutine[None, None, Message]
PageAnswer: TypeAlias = MessagePage | Coroutine[None, None, MessagePage]


def add_message(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/assistants/main.py#L884
    custom_llm_provider: str, thread_id: str, role: CallValue, content: CallValue,
    attachments: CallValue = None, metadata: CallValue = None, client: CallValue = None,
    **kwargs: CallValue,
) -> MessageAnswer:
    flag = kwargs.pop("a_add_message", None)
    arguments = named_arguments(kwargs, {
        "attachments": attachments,
        "metadata": metadata,
        "client": client,
    })
    if flag is True:
        return a_add_message(custom_llm_provider, thread_id, role, content, **arguments)
    payload = message_payload(custom_llm_provider, thread_id, role, content, arguments)
    answer = validated_answer(Message, NativeAssistantRunSession.add_message, payload)
    return completed_message(answer)


async def a_add_message(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/assistants/main.py#L841
    custom_llm_provider: str, thread_id: str, role: CallValue, content: CallValue,
    attachments: CallValue = None, metadata: CallValue = None, client: CallValue = None,
    **kwargs: CallValue,
) -> Message:
    arguments = named_arguments(kwargs, {
        "attachments": attachments,
        "metadata": metadata,
        "client": client,
    })
    payload = message_payload(custom_llm_provider, thread_id, role, content, arguments)
    answer = await validated_aanswer(Message, NativeAssistantRunSession.a_add_message, payload)
    return completed_message(answer)


def get_messages(
    custom_llm_provider: str, thread_id: str, client: CallValue = None,
    **kwargs: CallValue,
) -> PageAnswer:
    flag = kwargs.pop("aget_messages", None)
    arguments = named_arguments(kwargs, {"client": client})
    if flag is True:
        return aget_messages(custom_llm_provider, thread_id, **arguments)
    payload = to_call_payload(call_values(custom_llm_provider, thread_id, arguments))
    return validated_answer(MessagePage, NativeAssistantRunSession.get_messages, payload)


async def aget_messages(
    custom_llm_provider: str, thread_id: str, client: CallValue = None,
    **kwargs: CallValue,
) -> MessagePage:
    arguments = named_arguments(kwargs, {"client": client})
    payload = to_call_payload(call_values(custom_llm_provider, thread_id, arguments))
    return await validated_aanswer(MessagePage, NativeAssistantRunSession.aget_messages, payload)


def run_thread(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/assistants/main.py#L1231
    custom_llm_provider: str, thread_id: str, assistant_id: str,
    additional_instructions: CallValue = None, instructions: CallValue = None,
    metadata: CallValue = None, model: CallValue = None, stream: CallValue = None,
    tools: CallValue = None, client: CallValue = None,
    event_handler: EventHandler | CallValue = None, **kwargs: CallValue,
) -> RunAnswer:
    arun = kwargs.pop("arun_thread", None)
    arguments = named_arguments(kwargs, {
        "additional_instructions": additional_instructions,
        "instructions": instructions,
        "metadata": metadata,
        "model": model,
        "stream": stream,
        "tools": tools,
        "client": client,
    })
    payload = run_payload(custom_llm_provider, thread_id, assistant_id, arguments)
    if arguments.get("stream") is True:
        handler = checked_handler(event_handler)
        if arun is True:
            return async_stream_answer(
                NativeAssistantRunSession.arun_thread_stream, payload, handler)
        return stream_answer(NativeAssistantRunSession.run_thread_stream, payload, handler)
    if arun is True:
        return validated_aanswer(Run, NativeAssistantRunSession.arun_thread, payload)
    return validated_answer(Run, NativeAssistantRunSession.run_thread, payload)


def run_thread_stream(
    *, event_handler: EventHandler | CallValue = None, **kwargs: CallValue,
) -> AssistantStream:
    answer = run_thread(stream=True, event_handler=event_handler, **kwargs)
    match answer:
        case AssistantStream() as stream:
            return stream
        case object():
            raise TypeError("run_thread answered no stream")


def arun_thread_stream(
    *, event_handler: EventHandler | CallValue = None, **kwargs: CallValue,
) -> AsyncAssistantStream:
    kwargs["arun_thread"] = True
    answer = run_thread(stream=True, event_handler=event_handler, **kwargs)
    match answer:
        case AsyncAssistantStream() as stream:
            return stream
        case object():
            raise TypeError("run_thread answered no async stream")


async def arun_thread(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/assistants/main.py#L1179
    custom_llm_provider: str, thread_id: str, assistant_id: str,
    additional_instructions: CallValue = None, instructions: CallValue = None,
    metadata: CallValue = None, model: CallValue = None, stream: CallValue = None,
    tools: CallValue = None, client: CallValue = None, **kwargs: CallValue,
) -> Run | AsyncAssistantStream:
    arguments = named_arguments(kwargs, {
        "additional_instructions": additional_instructions,
        "instructions": instructions,
        "metadata": metadata,
        "model": model,
        "stream": stream,
        "tools": tools,
        "client": client,
    })
    payload = run_payload(custom_llm_provider, thread_id, assistant_id, arguments)
    if arguments.get("stream") is True:
        return async_stream_answer(NativeAssistantRunSession.arun_thread_stream, payload, None)
    return await validated_aanswer(Run, NativeAssistantRunSession.arun_thread, payload)
