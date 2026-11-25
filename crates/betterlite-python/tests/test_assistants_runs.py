import asyncio
import importlib
import inspect
import json
import threading
from collections.abc import Callable, Coroutine, Iterator, Mapping
from dataclasses import dataclass, field
from functools import partial
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Final, Protocol, TypeAlias
from urllib.parse import urlsplit

import litellm
import openai
import pytest
from assistant_server import (
    ASSISTANT_ID,
    AUTHORIZATION,
    BAD_KEY,
    BEARER,
    CREATED,
    GOOD_KEY,
    HOST,
    NOT_FOUND,
    OK,
    RATE_LIMITED,
    RATE_LIMITED_BODY,
    RATE_LIMITED_KEY,
    THREAD_ID,
    UNAUTHORIZED,
    RecordedCall,
)
from openai import AssistantEventHandler, AsyncAssistantEventHandler
from openai.lib.streaming import AssistantStreamManager, AsyncAssistantStreamManager
from openai.types.beta import AssistantStreamEvent as OpenAIStreamEvent
from pydantic import JsonValue
from test_assistants_records import (
    JSON,
    LITELLM,
    LITELLM_ASYNC,
    THEIR_ERRORS,
    CallArgument,
    answer_of,
    dump_pair,
    flagged_run,
    page_of,
    page_view,
    same_call,
    shape,
)
from typing_extensions import override, runtime_checkable

import betterlite
import betterlite.exceptions as errors
from betterlite import assistants as package
from betterlite._completion.errors import UnsupportedCallArgumentError
from betterlite.assistants import _runs, _utils, main

BAD_REQUEST_STATUS: Final = 400
RATE_LIMIT_STATUS: Final = 429
FAKE_CLIENT: Final = "openai.OpenAI"
FAKE_HANDLER: Final = "handler"
RUN_ID: Final = "run_1"
MESSAGE_ID: Final = "msg_1"
JSON_TYPE: Final = "application/json"
EVENT_TYPE: Final = "text/event-stream"
MESSAGES_PATH: Final = f"/v1/threads/{THREAD_ID}/messages"
BARE_MESSAGES_PATH: Final = "/v1/threads/thread_bare/messages"
RUNS_PATH: Final = f"/v1/threads/{THREAD_ID}/runs"
RUN_PATH: Final = f"{RUNS_PATH}/{RUN_ID}"
NO_CALLS: Final = 0
FIRST_READ_CALLS: Final = 1
REREAD_CALLS: Final = 2
STREAM_CALLS: Final = 3
DENIED_BODY: Final[dict[str, JsonValue]] = {
    "error": {
        "message": f"Incorrect API key provided: {BAD_KEY}.", "type": "invalid_request_error",
        "param": None, "code": "invalid_api_key",
    },
}
ATTACHMENTS: Final[list[JsonValue]] = [
    {"file_id": "file_1", "tools": [{"type": "code_interpreter"}]},
]
MESSAGE: Final[dict[str, JsonValue]] = {
    "id": MESSAGE_ID, "assistant_id": ASSISTANT_ID,
    "attachments": ATTACHMENTS, "completed_at": CREATED + 1,
    "content": [{"type": "text", "text": {"value": "hello", "annotations": []}}],
    "created_at": CREATED, "incomplete_at": None,
    "incomplete_details": None, "metadata": {"k": "v"},
    "object": "thread.message", "role": "user",
    "run_id": None, "status": "completed",
    "thread_id": THREAD_ID,
}
MESSAGE_BARE: Final[dict[str, JsonValue]] = {
    name: value for name, value in MESSAGE.items() if name != "status"
}
MESSAGE_STARTED: Final[dict[str, JsonValue]] = {
    **MESSAGE, "completed_at": None, "content": [], "status": "in_progress",
}
MESSAGE_DELTA: Final[dict[str, JsonValue]] = {
    "id": MESSAGE_ID,
    "object": "thread.message.delta",
    "delta": {
        "role": "assistant",
        "content": [{"index": 0, "type": "text", "text": {"value": "hello", "annotations": []}}],
    },
}
MESSAGE_PAGE: Final[dict[str, JsonValue]] = {
    "object": "list", "data": [MESSAGE], "first_id": MESSAGE_ID,
    "last_id": MESSAGE_ID, "has_more": False,
}
RUN: Final[dict[str, JsonValue]] = {
    "id": RUN_ID, "assistant_id": ASSISTANT_ID, "cancelled_at": None,
    "completed_at": CREATED + 2, "created_at": CREATED, "expires_at": None,
    "failed_at": None, "incomplete_details": None, "instructions": "be brief",
    "last_error": None, "max_completion_tokens": 100, "max_prompt_tokens": 200,
    "metadata": {"k": "v"}, "model": "gpt-4o-mini", "object": "thread.run",
    "parallel_tool_calls": True, "required_action": None, "response_format": "auto",
    "started_at": CREATED + 1, "status": "completed", "thread_id": THREAD_ID,
    "tool_choice": "auto", "tools": [{"type": "code_interpreter"}],
    "truncation_strategy": {"type": "auto", "last_messages": None},
    "usage": {"completion_tokens": 1, "prompt_tokens": 2, "total_tokens": 3},
    "temperature": 0.5, "top_p": 0.75,
}
RUN_QUEUED: Final[dict[str, JsonValue]] = {
    **RUN, "completed_at": None, "started_at": None, "status": "queued", "usage": None,
}
STREAM_EVENTS: Final[list[tuple[str, JsonValue]]] = [
    ("thread.run.created", RUN_QUEUED), ("thread.message.created", MESSAGE_STARTED),
    ("thread.message.delta", MESSAGE_DELTA), ("thread.message.completed", MESSAGE),
    ("thread.run.completed", RUN),
]
EVENT_NAMES: Final[list[str]] = [name for name, _ in STREAM_EVENTS]
ROUTES: Final[dict[tuple[str, str], dict[str, JsonValue]]] = {
    ("GET", MESSAGES_PATH): MESSAGE_PAGE,
    ("GET", BARE_MESSAGES_PATH): {
        "data": [MESSAGE], "first_id": MESSAGE_ID, "last_id": MESSAGE_ID,
    },
    ("POST", MESSAGES_PATH): MESSAGE,
    ("POST", BARE_MESSAGES_PATH): MESSAGE_BARE,
    ("POST", RUNS_PATH): RUN_QUEUED,
    ("GET", RUN_PATH): RUN,
}
RUN_OPTIONS: Final[dict[str, JsonValue]] = {
    "additional_instructions": "be kind", "instructions": "be brief", "metadata": {"k": "v"},
    "model": "gpt-4o-mini", "tools": [{"type": "code_interpreter"}],
}
MESSAGE_ARGUMENTS: Final[dict[str, JsonValue]] = {
    "attachments": ATTACHMENTS, "metadata": {"k": "v"},
}
BARE_STREAM_ARGUMENTS: Final[dict[str, str]] = {
    "custom_llm_provider": "openai",
    "thread_id": THREAD_ID,
    "assistant_id": ASSISTANT_ID,
}
BEDROCK_STREAM_ARGUMENTS: Final[dict[str, str]] = {
    "custom_llm_provider": "bedrock",
    "thread_id": THREAD_ID,
    "assistant_id": ASSISTANT_ID,
}
View: TypeAlias = dict[str, JsonValue]


def event_text() -> bytes:
    chunks = [f"event: {name}\ndata: {json.dumps(data)}\n\n" for name, data in STREAM_EVENTS]
    chunks.append("event: done\ndata: [DONE]\n\n")
    return "".join(chunks).encode()


def is_stream(body: JsonValue) -> bool:
    match body:
        case {"stream": True}:
            return True
        case object():
            return False


class RunServer(ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self) -> None:
        super().__init__((HOST, 0), RunHandler)
        self.calls: list[RecordedCall] = []
        self.thread = threading.Thread(target=self.serve_forever, daemon=True)
        self.thread.start()

    @property
    def url(self) -> str:
        return f"http://{HOST}:{self.server_port}/v1"

    def connection(self, key: str = GOOD_KEY) -> dict[str, str]:
        return {"api_key": key, "api_base": self.url}

    def stream_arguments(self, key: str = GOOD_KEY) -> dict[str, str]:
        return {**BARE_STREAM_ARGUMENTS, **self.connection(key)}


@pytest.fixture(autouse=False)
def server() -> Iterator[RunServer]:
    running = RunServer()
    try:
        yield running
    finally:
        running.shutdown()
        running.server_close()
        running.thread.join()


class RunHandler(BaseHTTPRequestHandler):
    def do_GET(self) -> None:
        self.serve(None)

    def do_POST(self) -> None:
        self.serve(self.body())

    def serve(self, body: JsonValue) -> None:
        match self.server:
            case RunServer() as owner:
                owner.calls.append(
                    {
                        "method": self.command,
                        "path": self.path,
                        "headers": dict(self.headers.items()),
                        "body": body,
                    }
                )
            case object():
                raise AssertionError("the handler runs outside a RunServer")
        key = self.headers.get(AUTHORIZATION)
        if key == f"{BEARER}{RATE_LIMITED_KEY}":
            self.respond(RATE_LIMITED, JSON_TYPE, json.dumps(RATE_LIMITED_BODY).encode())
            return
        if key != f"{BEARER}{GOOD_KEY}":
            self.respond(UNAUTHORIZED, JSON_TYPE, json.dumps(DENIED_BODY).encode())
            return
        path = urlsplit(self.path).path
        if self.command == "POST" and path == RUNS_PATH and is_stream(body):
            self.respond(OK, EVENT_TYPE, event_text())
            return
        found = ROUTES.get((self.command, path))
        if found is None:
            self.send_error(NOT_FOUND)
            return
        self.respond(OK, JSON_TYPE, json.dumps(found).encode())

    def body(self) -> JsonValue:
        declared = self.headers.get("Content-Length")
        if declared is None:
            return None
        return JSON.validate_json(self.rfile.read(int(declared)))

    def respond(self, status: int, content_type: str, encoded: bytes) -> None:
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)


class StreamCall(Protocol):
    def __call__(
        self,
        *,
        event_handler: AssistantEventHandler | CallArgument = None,
        **kwargs: CallArgument,
    ) -> AssistantStreamManager[AssistantEventHandler]: ...


class AsyncStreamCall(Protocol):
    def __call__(
        self,
        *,
        event_handler: AsyncAssistantEventHandler | CallArgument = None,
        **kwargs: CallArgument,
    ) -> AsyncAssistantStreamManager[AsyncAssistantEventHandler]: ...


class LiteLLMStreams(Protocol):
    def __getattr__(self, name: str) -> StreamCall: ...


class LiteLLMAsyncStreams(Protocol):
    def __getattr__(self, name: str) -> AsyncStreamCall: ...


class UtilsCall(Protocol):
    def __call__(self, **kwargs: JsonValue) -> dict[str, JsonValue]: ...


class LiteLLMUtils(Protocol):
    def __getattr__(self, name: str) -> UtilsCall: ...


@runtime_checkable
class MessageCarrier(Protocol):
    message: str


STREAMS: LiteLLMStreams = importlib.import_module("litellm")
ASYNC_STREAMS: LiteLLMAsyncStreams = importlib.import_module("litellm")
THEIR_UTILS: LiteLLMUtils = importlib.import_module("litellm.assistants.utils")
EXPORTED_NAMES: Final[tuple[str, ...]] = (
    "Assistant", "AssistantDeleted", "AssistantPage", "AssistantStream",
    "AssistantStreamEvent", "AsyncAssistantStream", "EventHandler", "Message",
    "MessagePage", "Run", "a_add_message", "acreate_assistants", "acreate_thread",
    "add_message", "adelete_assistant", "aget_assistants", "aget_messages", "aget_thread",
    "arun_thread", "arun_thread_stream", "create_assistants", "create_thread",
    "delete_assistant", "get_assistants", "get_messages", "get_thread", "run_thread",
    "run_thread_stream",
)  # fmt: skip
LITELLM_FUNCTIONS: Final[tuple[str, ...]] = (
    "a_add_message", "acreate_assistants", "acreate_thread", "add_message",
    "adelete_assistant", "aget_assistants", "aget_messages", "aget_thread", "arun_thread",
    "arun_thread_stream", "create_assistants", "create_thread", "delete_assistant",
    "get_assistants", "get_messages", "get_thread", "run_thread", "run_thread_stream",
)  # fmt: skip


@dataclass
class OurHandler(_runs.EventHandler):
    seen: list[_runs.AssistantStreamEvent] = field(default_factory=list)

    @override
    def on_event(self, event: _runs.AssistantStreamEvent) -> None:
        self.seen.append(event)


class ReferenceHandler(AssistantEventHandler):
    def __init__(self) -> None:
        super().__init__()
        self.names: list[str] = []

    @override
    def on_event(self, event: OpenAIStreamEvent) -> None:
        self.names.append(event.event)


def matched_calls(server: RunServer, count: int) -> None:
    recorded = list(server.calls)
    server.calls.clear()
    assert len(recorded) == count * 2
    for reference, ours in zip(recorded[:count], recorded[count:], strict=True):
        same_call(reference, ours)


def run_of(answer: _runs.RunAnswer) -> _runs.Run:
    match answer:
        case _runs.Run() as run:
            return run
        case object():
            raise AssertionError("the call answered no run")


def sync_stream_of(answer: _runs.RunAnswer) -> main.AssistantStream:
    match answer:
        case main.AssistantStream() as stream:
            return stream
        case object():
            raise AssertionError("the call answered no sync stream")


def async_stream_of(answer: _runs.RunAnswer) -> main.AsyncAssistantStream:
    match answer:
        case main.AsyncAssistantStream() as stream:
            return stream
        case object():
            raise AssertionError("the call answered no async stream")


def reference_events(manager: AssistantStreamManager[AssistantEventHandler]) -> list[View]:
    with manager as handler:
        return [
            {"event": event.event, "data": JSON.validate_json(event.data.model_dump_json())}
            for event in handler
        ]


def our_events(stream: main.AssistantStream) -> list[View]:
    with stream as events:
        return [{"event": event.event, "data": event.data} for event in events]


async def reference_async_events(
    manager: AsyncAssistantStreamManager[AsyncAssistantEventHandler],
) -> list[View]:
    async with manager as handler:
        return [
            {"event": event.event, "data": JSON.validate_json(event.data.model_dump_json())}
            async for event in handler
        ]


async def our_async_events(stream: main.AsyncAssistantStream) -> list[View]:
    async with stream as events:
        return [{"event": event.event, "data": event.data} async for event in events]


def reference_first(manager: AssistantStreamManager[AssistantEventHandler]) -> View:
    with manager as handler:
        first = next(iter(handler))
        data = JSON.validate_json(first.data.model_dump_json())
        return {"event": first.event, "data": data}


def our_first(stream: main.AssistantStream) -> View:
    with stream as events:
        first = next(events)
        return {"event": first.event, "data": first.data}


async def our_async_first(stream: main.AsyncAssistantStream) -> View:
    async with stream as events:
        first = await anext(events)
        return {"event": first.event, "data": first.data}


def closed_after_first(stream: main.AssistantStream) -> _runs.AssistantStreamEvent | None:
    with stream as events:
        assert next(events).event == EVENT_NAMES[0]
    return next(stream, None)


async def async_closed_after_first(
    stream: main.AsyncAssistantStream,
) -> _runs.AssistantStreamEvent | None:
    async with stream as events:
        assert (await anext(events)).event == EVENT_NAMES[0]
    return await anext(stream, None)


def test_signature_matches_litellm() -> None:
    assert shape(inspect.signature(main.add_message)) == shape(
        inspect.signature(LITELLM.add_message))
    assert shape(inspect.signature(main.a_add_message)) == shape(
        inspect.signature(LITELLM.a_add_message))
    assert shape(inspect.signature(main.get_messages)) == shape(
        inspect.signature(LITELLM.get_messages))
    assert shape(inspect.signature(main.aget_messages)) == shape(
        inspect.signature(LITELLM.aget_messages))
    assert shape(inspect.signature(main.run_thread)) == shape(
        inspect.signature(LITELLM.run_thread))
    assert shape(inspect.signature(main.arun_thread)) == shape(
        inspect.signature(LITELLM.arun_thread))
    assert shape(inspect.signature(main.run_thread_stream)) == shape(
        inspect.signature(LITELLM.run_thread_stream))
    assert shape(inspect.signature(main.arun_thread_stream)) == shape(
        inspect.signature(LITELLM.arun_thread_stream))
    assert shape(inspect.signature(_utils.get_optional_params_add_message)) == shape(
        inspect.signature(THEIR_UTILS.get_optional_params_add_message))
    assert shape(inspect.signature(_utils.get_optional_params_image_gen)) == shape(
        inspect.signature(THEIR_UTILS.get_optional_params_image_gen))
    assert inspect.iscoroutinefunction(main.a_add_message)
    assert inspect.iscoroutinefunction(main.aget_messages)
    assert inspect.iscoroutinefunction(main.arun_thread)
    assert not inspect.iscoroutinefunction(main.run_thread)
    assert not inspect.iscoroutinefunction(main.run_thread_stream)
    assert not inspect.iscoroutinefunction(main.arun_thread_stream)


def test_signature_stream_helpers(server: RunServer) -> None:
    sync_stream = main.run_thread_stream(**server.stream_arguments())
    async_stream = main.arun_thread_stream(**server.stream_arguments())
    flagged = main.run_thread_stream(arun_thread=True, **server.stream_arguments())
    assert sync_stream_of(sync_stream) is sync_stream
    assert async_stream_of(async_stream) is async_stream
    assert async_stream_of(flagged) is flagged
    assert not server.calls
    with sync_stream_of(sync_stream) as entered:
        assert entered is sync_stream


def test_signature_refuses_client() -> None:
    with pytest.raises(UnsupportedCallArgumentError, match="client"):
        main.add_message("openai", THREAD_ID, "user", "hi", client=FAKE_CLIENT)
    with pytest.raises(UnsupportedCallArgumentError, match="client"):
        asyncio.run(main.a_add_message("openai", THREAD_ID, "user", "hi", client=FAKE_CLIENT))
    with pytest.raises(UnsupportedCallArgumentError, match="client"):
        main.get_messages("openai", THREAD_ID, client=FAKE_CLIENT)
    with pytest.raises(UnsupportedCallArgumentError, match="client"):
        asyncio.run(main.aget_messages("openai", THREAD_ID, client=FAKE_CLIENT))
    with pytest.raises(UnsupportedCallArgumentError, match="client"):
        main.run_thread("openai", THREAD_ID, ASSISTANT_ID, client=FAKE_CLIENT)
    with pytest.raises(UnsupportedCallArgumentError, match="client"):
        asyncio.run(main.arun_thread("openai", THREAD_ID, ASSISTANT_ID, client=FAKE_CLIENT))
    with pytest.raises(UnsupportedCallArgumentError, match="client"):
        main.run_thread_stream(**BARE_STREAM_ARGUMENTS, client=FAKE_CLIENT)
    with pytest.raises(UnsupportedCallArgumentError, match="client"):
        main.arun_thread_stream(**BARE_STREAM_ARGUMENTS, client=FAKE_CLIENT)


def compare_add_message(server: RunServer, arguments: Mapping[str, JsonValue]) -> None:
    reference = answer_of(LITELLM.add_message("openai", THREAD_ID, "user", "hello", **arguments))
    ours = answer_of(main.add_message("openai", THREAD_ID, "user", "hello", **arguments))
    matched_calls(server, 1)
    dump_pair(ours, reference)


def compare_run_thread(server: RunServer, arguments: Mapping[str, JsonValue]) -> _runs.Run:
    reference = answer_of(LITELLM.run_thread("openai", THREAD_ID, ASSISTANT_ID, **arguments))
    ours = run_of(main.run_thread("openai", THREAD_ID, ASSISTANT_ID, **arguments))
    matched_calls(server, 2)
    dump_pair(ours, reference)
    return ours


def test_calls_add_message(server: RunServer) -> None:
    args = server.connection()
    cases: list[dict[str, JsonValue]] = [MESSAGE_ARGUMENTS, {}]
    for extra in cases:
        merged: dict[str, JsonValue] = {**extra, **args}
        compare_add_message(server, merged)


def test_calls_completed_status(server: RunServer) -> None:
    args = server.connection()
    reference = answer_of(LITELLM.add_message("openai", "thread_bare", "user", "hello", **args))
    ours = answer_of(main.add_message("openai", "thread_bare", "user", "hello", **args))
    matched_calls(server, 1)
    dump_pair(ours, reference)
    assert JSON.validate_json(ours.model_dump_json()) == MESSAGE


def test_calls_get_messages(server: RunServer) -> None:
    args = server.connection()
    reference = page_of(LITELLM.get_messages("openai", THREAD_ID, **args))
    ours = page_of(main.get_messages("openai", THREAD_ID, **args))
    matched_calls(server, 1)
    assert page_view(ours) == page_view(reference)
    assert page_view(ours)["data"] == [MESSAGE]
    page = main.MessagePage.model_validate(MESSAGE_PAGE)
    assert [message.id for message in page.data] == [MESSAGE_ID]
    reference_bare = page_of(LITELLM.get_messages("openai", "thread_bare", **args))
    ours_bare = page_of(main.get_messages("openai", "thread_bare", **args))
    matched_calls(server, 1)
    assert page_view(ours_bare) == page_view(reference_bare)


def test_calls_run_thread(server: RunServer) -> None:
    args = server.connection()
    merged: dict[str, JsonValue] = {**RUN_OPTIONS, **args}
    run = compare_run_thread(server, merged)
    assert JSON.validate_json(run.model_dump_json()) == RUN
    compare_run_thread(server, args)


def test_calls_module_globals(server: RunServer, monkeypatch: pytest.MonkeyPatch) -> None:
    # ast-grep-ignore: no-string-mocking
    monkeypatch.setattr(betterlite, "api_key", GOOD_KEY)
    # ast-grep-ignore: no-string-mocking
    monkeypatch.setattr(betterlite, "api_base", server.url)
    # ast-grep-ignore: no-string-mocking
    monkeypatch.setattr(litellm, "api_key", GOOD_KEY)
    # ast-grep-ignore: no-string-mocking
    monkeypatch.setattr(litellm, "api_base", server.url)
    compare_add_message(server, {})
    compare_run_thread(server, {})


def test_calls_handler_no_stream(server: RunServer) -> None:
    args = server.connection()
    handler = OurHandler()
    ours = run_of(
        main.run_thread("openai", THREAD_ID, ASSISTANT_ID, event_handler=handler, **args)
    )
    assert ours.id == RUN_ID
    assert not handler.seen
    for call in server.calls:
        assert "event_handler" not in json.dumps(call.get("body"))


def test_calls_async_forms(server: RunServer) -> None:
    args = server.connection()
    reference_added = answer_of(
        asyncio.run(
            LITELLM_ASYNC.a_add_message(
                "openai", THREAD_ID, "user", "hello", **MESSAGE_ARGUMENTS, **args
            )
        )
    )
    ours_added = answer_of(
        asyncio.run(
            main.a_add_message("openai", THREAD_ID, "user", "hello", **MESSAGE_ARGUMENTS, **args)
        )
    )
    matched_calls(server, 1)
    dump_pair(ours_added, reference_added)
    flagged_added = answer_of(
        flagged_run(
            main.add_message("openai", THREAD_ID, "user", "hello", a_add_message=True, **args)
        )
    )
    assert server.calls.pop()["method"] == "POST"
    dump_pair(flagged_added, reference_added)
    reference_page = page_of(
        asyncio.run(LITELLM_ASYNC.aget_messages("openai", THREAD_ID, **args))
    )
    ours_page = page_of(asyncio.run(main.aget_messages("openai", THREAD_ID, **args)))
    matched_calls(server, 1)
    assert page_view(ours_page) == page_view(reference_page)
    flagged_page = page_of(
        flagged_run(main.get_messages("openai", THREAD_ID, aget_messages=True, **args))
    )
    server.calls.clear()
    assert page_view(flagged_page) == page_view(reference_page)
    reference_run = answer_of(
        asyncio.run(
            LITELLM_ASYNC.arun_thread("openai", THREAD_ID, ASSISTANT_ID, **RUN_OPTIONS, **args)
        )
    )
    ours_run = asyncio.run(
        main.arun_thread("openai", THREAD_ID, ASSISTANT_ID, **RUN_OPTIONS, **args)
    )
    matched_calls(server, 2)
    dump_pair(run_of(ours_run), reference_run)
    flagged = main.run_thread("openai", THREAD_ID, ASSISTANT_ID, arun_thread=True, **args)
    match flagged:
        case Coroutine() as pending:
            assert run_of(asyncio.run(pending)).id == RUN_ID
        case object():
            raise AssertionError("the flag did not answer a coroutine")
    server.calls.clear()


def test_stream_events(server: RunServer) -> None:
    theirs = ReferenceHandler()
    reference = reference_events(
        STREAMS.run_thread_stream(event_handler=theirs, **server.stream_arguments())
    )
    mine = OurHandler()
    opened = main.run_thread_stream(event_handler=mine, **server.stream_arguments())
    ours = our_events(sync_stream_of(opened))
    matched_calls(server, 1)
    assert ours == reference
    assert [view["event"] for view in ours] == EVENT_NAMES
    assert ours[-1]["data"] == RUN
    assert theirs.names == EVENT_NAMES
    assert [event.event for event in mine.seen] == EVENT_NAMES
    assert [{"event": event.event, "data": event.data} for event in mine.seen] == ours


def test_stream_no_handler(server: RunServer) -> None:
    reference = reference_events(STREAMS.run_thread_stream(**server.stream_arguments()))
    opened = main.run_thread_stream(**server.stream_arguments())
    ours = our_events(sync_stream_of(opened))
    matched_calls(server, 1)
    assert ours == reference


def test_stream_async_events(server: RunServer) -> None:
    reference = asyncio.run(
        reference_async_events(ASYNC_STREAMS.arun_thread_stream(**server.stream_arguments()))
    )
    handler = OurHandler()
    ours = asyncio.run(
        our_async_events(
            main.arun_thread_stream(event_handler=handler, **server.stream_arguments())
        )
    )
    matched_calls(server, 1)
    assert ours == reference
    assert [view["event"] for view in ours] == EVENT_NAMES
    assert [{"event": event.event, "data": event.data} for event in handler.seen] == ours
    sync_events = our_events(sync_stream_of(main.run_thread_stream(**server.stream_arguments())))
    server.calls.clear()
    assert sync_events == ours


def test_stream_run_thread_flags(server: RunServer) -> None:
    args = server.connection()
    sync_stream = sync_stream_of(
        main.run_thread("openai", THREAD_ID, ASSISTANT_ID, stream=True, **args)
    )
    flagged = async_stream_of(
        main.run_thread(
            "openai", THREAD_ID, ASSISTANT_ID, stream=True, arun_thread=True, **args
        )
    )
    awaited = async_stream_of(
        asyncio.run(main.arun_thread("openai", THREAD_ID, ASSISTANT_ID, stream=True, **args))
    )
    names = [
        [view["event"] for view in our_events(sync_stream)],
        [view["event"] for view in asyncio.run(our_async_events(flagged))],
        [view["event"] for view in asyncio.run(our_async_events(awaited))],
    ]
    assert names == [EVENT_NAMES] * STREAM_CALLS
    assert len(server.calls) == STREAM_CALLS


def test_stream_closes_on_exit(server: RunServer) -> None:
    stream = sync_stream_of(main.run_thread_stream(**server.stream_arguments()))
    assert closed_after_first(stream) is None
    assert len(server.calls) == FIRST_READ_CALLS
    async_stream = main.arun_thread_stream(**server.stream_arguments())
    assert asyncio.run(async_closed_after_first(async_stream)) is None
    assert len(server.calls) == REREAD_CALLS
    raising = sync_stream_of(main.run_thread_stream(**server.stream_arguments()))
    with pytest.raises(RuntimeError, match="stop"), raising:
        raise RuntimeError("stop")
    assert next(raising, None) is None
    assert len(server.calls) == REREAD_CALLS


def test_stream_first_read_waits(server: RunServer) -> None:
    stream = sync_stream_of(main.run_thread_stream(**server.stream_arguments()))
    assert not server.calls
    expected: View = {"event": EVENT_NAMES[0], "data": STREAM_EVENTS[0][1]}
    assert our_first(stream) == expected
    assert len(server.calls) == FIRST_READ_CALLS
    async_first = asyncio.run(
        our_async_first(main.arun_thread_stream(**server.stream_arguments()))
    )
    assert async_first == expected
    assert reference_first(STREAMS.run_thread_stream(**server.stream_arguments())) == expected


def test_stream_argument_checks() -> None:
    with pytest.raises(UnsupportedCallArgumentError, match="event_handler"):
        main.run_thread_stream(**BARE_STREAM_ARGUMENTS, event_handler=FAKE_HANDLER)
    with pytest.raises(UnsupportedCallArgumentError, match="event_handler"):
        main.arun_thread_stream(**BARE_STREAM_ARGUMENTS, event_handler=FAKE_HANDLER)
    with pytest.raises(UnsupportedCallArgumentError, match="event_handler"):
        main.run_thread(
            "openai", THREAD_ID, ASSISTANT_ID, stream=True, event_handler=FAKE_HANDLER
        )
    with pytest.raises(TypeError, match="stream"):
        main.run_thread_stream(**BARE_STREAM_ARGUMENTS, stream=True)
    with pytest.raises(TypeError, match="stream"):
        STREAMS.run_thread_stream(**BARE_STREAM_ARGUMENTS, stream=True)
    with pytest.raises(KeyError, match="custom_llm_provider"):
        main.run_thread_stream()
    with pytest.raises(KeyError, match="custom_llm_provider"):
        main.arun_thread_stream()


def test_stream_failure_first_read(server: RunServer) -> None:
    bad = server.stream_arguments(BAD_KEY)
    ours = sync_stream_of(main.run_thread_stream(**bad))
    assert len(server.calls) == NO_CALLS
    with pytest.raises(errors.AuthenticationError) as ours_failure:
        our_first(ours)
    with pytest.raises(openai.AuthenticationError) as reference_failure:
        reference_first(STREAMS.run_thread_stream(**bad))
    assert type(ours_failure.value).__name__ == type(reference_failure.value).__name__
    assert ours_failure.value.status_code == UNAUTHORIZED
    assert ours_failure.value.__cause__ is not None
    assert next(ours, None) is None
    with pytest.raises(errors.AuthenticationError):
        asyncio.run(our_async_first(main.arun_thread_stream(**bad)))


def error_text(failure: Exception | MessageCarrier) -> str:
    match failure:
        case MessageCarrier() as carrier:
            return carrier.message
        case object():
            return str(failure)


def utils_outcome(function: Callable[[], dict[str, JsonValue]]) -> View:
    try:
        answer = function()
    except UTILS_FAILURES as failure:
        return {"error": [type(failure).__name__, error_text(failure)]}
    return {"ok": answer}


def compared_outcomes(
    ours: Callable[[], dict[str, JsonValue]], reference: Callable[[], dict[str, JsonValue]],
) -> None:
    try:
        for dropped in (False, True):
            litellm.drop_params = dropped
            betterlite.drop_params = dropped
            assert utils_outcome(ours) == utils_outcome(reference)
    finally:
        litellm.drop_params = False
        betterlite.drop_params = False


UTILS_FAILURES: Final[tuple[type[Exception], ...]] = (
    THEIR_ERRORS.UnsupportedParamsError,
    errors.UnsupportedParamsError,
)
# the provider is its own value because it is the one argument the helper types as text
ADD_MESSAGE_CASES: Final[dict[str, tuple[str, dict[str, JsonValue]]]] = {
    "openai_everything": ("openai", {
        "role": "user", "content": "hi", "attachments": ATTACHMENTS, "metadata": {"k": "v"},
    }),
    "openai_extra_key": ("openai", {
        "role": "assistant", "content": "hi", "attachments": None, "metadata": None,
        "extra_flag": True,
    }),
    "azure_text": ("azure", {
        "role": "user", "content": "hi", "attachments": None, "metadata": {"k": "v"},
    }),
    "azure_list_content": ("azure", {
        "role": "user", "content": [{"type": "text", "text": "hi"}], "attachments": None,
        "metadata": None,
    }),
    "azure_plain_attachment": ("azure", {
        "role": "user", "content": "hi", "attachments": [{"text": "x"}], "metadata": None,
    }),
    "azure_attachments_not_a_list": ("azure", {
        "role": "user", "content": "hi", "attachments": "nope", "metadata": None,
    }),
    "other_provider_extra_key": ("bedrock", {
        "role": "user", "content": "hi", "attachments": None, "metadata": None,
        "extra_flag": 1,
    }),
}  # fmt: skip
IMAGE_CASES: Final[dict[str, tuple[str | None, dict[str, JsonValue]]]] = {
    "openai_everything": ("openai", {
        "n": 2, "quality": "hd", "response_format": "url", "size": "1024x1024",
        "style": "vivid", "user": "u1",
    }),
    "openai_compatible_provider": ("groq", {"n": 1, "size": "256x256"}),
    "azure_extra_key": ("azure", {"n": 1, "extra_flag": "x"}),
    "bedrock_size": ("bedrock", {"size": "512x768"}),
    "bedrock_size_and_count": ("bedrock", {"size": "512x768", "n": 2}),
    "bedrock_nothing": ("bedrock", {"extra_flag": 1}),
    "vertex_count": ("vertex_ai", {"n": 3}),
    "vertex_count_and_size": ("vertex_ai", {"n": 3, "size": "512x512"}),
    "unknown_provider": ("unknown", {"n": 3, "size": "512x512"}),
    "no_provider": (None, {"n": 3, "extra_flag": 1}),
}  # fmt: skip


@pytest.mark.parametrize("name", list(ADD_MESSAGE_CASES))
def test_utils_add_message_matches(name: str) -> None:
    provider, case = ADD_MESSAGE_CASES[name]
    compared_outcomes(
        partial(_utils.get_optional_params_add_message, custom_llm_provider=provider, **case),
        partial(THEIR_UTILS.get_optional_params_add_message, custom_llm_provider=provider, **case),
    )


@pytest.mark.parametrize("name", list(IMAGE_CASES))
def test_utils_image_gen_matches(name: str) -> None:
    provider, case = IMAGE_CASES[name]
    compared_outcomes(
        partial(_utils.get_optional_params_image_gen, custom_llm_provider=provider, **case),
        partial(THEIR_UTILS.get_optional_params_image_gen, custom_llm_provider=provider, **case),
    )


def test_utils_both_outcomes() -> None:
    provider, list_case = ADD_MESSAGE_CASES["azure_list_content"]
    rejected = utils_outcome(
        partial(_utils.get_optional_params_add_message, custom_llm_provider=provider, **list_case)
    )
    assert rejected["error"]
    provider, text_case = ADD_MESSAGE_CASES["azure_text"]
    accepted = utils_outcome(
        partial(_utils.get_optional_params_add_message, custom_llm_provider=provider, **text_case)
    )
    assert accepted["ok"] == {"role": "user", "content": "hi", "metadata": {"k": "v"}}


def test_package_exports_names() -> None:
    reference = importlib.import_module("litellm.assistants.main")
    assert package.__all__ == main.__all__
    for name in EXPORTED_NAMES:
        assert name in main.__all__, name
        assert name in dir(main), name
        assert name in dir(package), name
    assert package.add_message is main.add_message
    assert package.Message is main.Message
    for name in LITELLM_FUNCTIONS:
        assert name in dir(reference), name
        assert name in main.__all__, name


def test_package_utils_registered() -> None:
    utils = importlib.import_module("betterlite.assistants.utils")
    reference = importlib.import_module("litellm.assistants.utils")
    assert utils is _utils
    for name in ("get_optional_params_add_message", "get_optional_params_image_gen"):
        assert name in dir(utils)
        assert name in dir(reference)


def assert_denied(
    ours: errors.AuthenticationError, reference: openai.AuthenticationError,
) -> None:
    assert type(ours).__name__ == type(reference).__name__
    assert ours.status_code == UNAUTHORIZED
    assert "Incorrect API key" in str(ours)
    assert "Incorrect API key" in str(reference)
    assert ours.__cause__ is not None


def test_error_message_denied(server: RunServer) -> None:
    bad = server.connection(BAD_KEY)
    with pytest.raises(errors.AuthenticationError) as ours:
        main.add_message("openai", THREAD_ID, "user", "hi", **bad)
    with pytest.raises(openai.AuthenticationError) as reference:
        LITELLM.add_message("openai", THREAD_ID, "user", "hi", **bad)
    assert_denied(ours.value, reference.value)


def test_error_list_denied(server: RunServer) -> None:
    bad = server.connection(BAD_KEY)
    with pytest.raises(errors.AuthenticationError) as ours:
        main.get_messages("openai", THREAD_ID, **bad)
    with pytest.raises(openai.AuthenticationError) as reference:
        LITELLM.get_messages("openai", THREAD_ID, **bad)
    assert_denied(ours.value, reference.value)


def test_error_async_denied(server: RunServer) -> None:
    bad = server.connection(BAD_KEY)
    with pytest.raises(errors.AuthenticationError) as ours:
        asyncio.run(main.a_add_message("openai", THREAD_ID, "user", "hi", **bad))
    with pytest.raises(openai.AuthenticationError) as reference:
        asyncio.run(LITELLM_ASYNC.a_add_message("openai", THREAD_ID, "user", "hi", **bad))
    assert_denied(ours.value, reference.value)


def test_error_run_denied(server: RunServer) -> None:
    bad = server.connection(BAD_KEY)
    with pytest.raises(errors.AuthenticationError) as ours:
        main.run_thread("openai", THREAD_ID, ASSISTANT_ID, **bad)
    with pytest.raises(openai.AuthenticationError) as reference:
        LITELLM.run_thread("openai", THREAD_ID, ASSISTANT_ID, **bad)
    assert_denied(ours.value, reference.value)


def test_error_async_run_denied(server: RunServer) -> None:
    bad = server.connection(BAD_KEY)
    with pytest.raises(errors.AuthenticationError) as ours:
        asyncio.run(main.arun_thread("openai", THREAD_ID, ASSISTANT_ID, **bad))
    with pytest.raises(openai.AuthenticationError) as reference:
        asyncio.run(LITELLM_ASYNC.arun_thread("openai", THREAD_ID, ASSISTANT_ID, **bad))
    assert_denied(ours.value, reference.value)


def test_error_rate_limit_class(server: RunServer) -> None:
    limited: dict[str, JsonValue] = {**server.connection(RATE_LIMITED_KEY), "max_retries": 0}
    with pytest.raises(errors.RateLimitError) as ours_message:
        main.add_message("openai", THREAD_ID, "user", "hi", **limited)
    assert ours_message.value.status_code == RATE_LIMIT_STATUS
    with pytest.raises(errors.RateLimitError) as ours_run:
        main.run_thread("openai", THREAD_ID, ASSISTANT_ID, **limited)
    assert ours_run.value.status_code == RATE_LIMIT_STATUS
    assert ours_run.value.__cause__ is not None
    with pytest.raises(openai.RateLimitError):
        LITELLM.run_thread("openai", THREAD_ID, ASSISTANT_ID, **limited)


def test_error_unknown_provider() -> None:
    with pytest.raises(errors.BadRequestError) as ours_message:
        main.add_message("bedrock", THREAD_ID, "user", "hi")
    with pytest.raises(THEIR_ERRORS.BadRequestError) as reference_message:
        LITELLM.add_message("bedrock", THREAD_ID, "user", "hi")
    assert str(ours_message.value) == str(reference_message.value)
    assert ours_message.value.model == "n/a"
    assert ours_message.value.status_code == BAD_REQUEST_STATUS
    with pytest.raises(errors.BadRequestError) as ours_list:
        main.get_messages("bedrock", THREAD_ID)
    with pytest.raises(THEIR_ERRORS.BadRequestError) as reference_list:
        LITELLM.get_messages("bedrock", THREAD_ID)
    assert str(ours_list.value) == str(reference_list.value)
    with pytest.raises(errors.BadRequestError) as ours_run:
        main.run_thread("bedrock", THREAD_ID, ASSISTANT_ID)
    with pytest.raises(THEIR_ERRORS.BadRequestError) as reference_run:
        LITELLM.run_thread("bedrock", THREAD_ID, ASSISTANT_ID)
    assert str(ours_run.value) == str(reference_run.value)
    assert ours_run.value.model == "n/a"
    with pytest.raises(errors.BadRequestError):
        asyncio.run(main.arun_thread("bedrock", THREAD_ID, ASSISTANT_ID))
    with pytest.raises(errors.BadRequestError):
        our_first(sync_stream_of(main.run_thread_stream(**BEDROCK_STREAM_ARGUMENTS)))
