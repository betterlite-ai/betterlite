import asyncio
import importlib
import inspect
import json
import threading
from collections.abc import Callable, Coroutine
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from types import TracebackType
from typing import Final, Protocol, TypeAlias
from urllib.parse import urlsplit

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
from typing_extensions import override

import betterlite.exceptions as errors
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
DENIED_BODY: Final[dict[str, JsonValue]] = {
    "error": {
        "message": f"Incorrect API key provided: {BAD_KEY}.",
        "type": "invalid_request_error",
        "param": None,
        "code": "invalid_api_key",
    },
}
ATTACHMENTS: Final[list[JsonValue]] = [{"file_id": "file_1", "tools": [{"type": "code_interpreter"}]}]
MESSAGE: Final[dict[str, JsonValue]] = {
    "id": MESSAGE_ID,
    "assistant_id": ASSISTANT_ID,
    "attachments": ATTACHMENTS,
    "completed_at": CREATED + 1,
    "content": [{"type": "text", "text": {"value": "hello", "annotations": []}}],
    "created_at": CREATED,
    "incomplete_at": None,
    "incomplete_details": None,
    "metadata": {"k": "v"},
    "object": "thread.message",
    "role": "user",
    "run_id": None,
    "status": "completed",
    "thread_id": THREAD_ID,
}
MESSAGE_BARE: Final[dict[str, JsonValue]] = {
    name: value for name, value in MESSAGE.items() if name != "status"
}
MESSAGE_STARTED: Final[dict[str, JsonValue]] = {
    **MESSAGE,
    "completed_at": None,
    "content": [],
    "status": "in_progress",
}
MESSAGE_DELTA: Final[dict[str, JsonValue]] = {
    "id": MESSAGE_ID,
    "object": "thread.message.delta",
    "delta": {
        "role": "assistant",
        "content": [
            {"index": 0, "type": "text", "text": {"value": "hello", "annotations": []}},
        ],
    },
}
MESSAGE_PAGE: Final[dict[str, JsonValue]] = {
    "object": "list",
    "data": [MESSAGE],
    "first_id": MESSAGE_ID,
    "last_id": MESSAGE_ID,
    "has_more": False,
}
RUN: Final[dict[str, JsonValue]] = {
    "id": RUN_ID,
    "assistant_id": ASSISTANT_ID,
    "cancelled_at": None,
    "completed_at": CREATED + 2,
    "created_at": CREATED,
    "expires_at": None,
    "failed_at": None,
    "incomplete_details": None,
    "instructions": "be brief",
    "last_error": None,
    "max_completion_tokens": 100,
    "max_prompt_tokens": 200,
    "metadata": {"k": "v"},
    "model": "gpt-4o-mini",
    "object": "thread.run",
    "parallel_tool_calls": True,
    "required_action": None,
    "response_format": "auto",
    "started_at": CREATED + 1,
    "status": "completed",
    "thread_id": THREAD_ID,
    "tool_choice": "auto",
    "tools": [{"type": "code_interpreter"}],
    "truncation_strategy": {"type": "auto", "last_messages": None},
    "usage": {"completion_tokens": 1, "prompt_tokens": 2, "total_tokens": 3},
    "temperature": 0.5,
    "top_p": 0.75,
}
RUN_QUEUED: Final[dict[str, JsonValue]] = {
    **RUN,
    "completed_at": None,
    "started_at": None,
    "status": "queued",
    "usage": None,
}
STREAM_EVENTS: Final[list[tuple[str, JsonValue]]] = [
    ("thread.run.created", RUN_QUEUED),
    ("thread.message.created", MESSAGE_STARTED),
    ("thread.message.delta", MESSAGE_DELTA),
    ("thread.message.completed", MESSAGE),
    ("thread.run.completed", RUN),
]
EVENT_NAMES: Final[list[str]] = [name for name, _ in STREAM_EVENTS]
ROUTES: Final[dict[tuple[str, str], dict[str, JsonValue]]] = {
    ("GET", MESSAGES_PATH): MESSAGE_PAGE,
    ("POST", MESSAGES_PATH): MESSAGE,
    ("POST", BARE_MESSAGES_PATH): MESSAGE_BARE,
    ("POST", RUNS_PATH): RUN_QUEUED,
    ("GET", RUN_PATH): RUN,
}
RUN_OPTIONS: Final[dict[str, JsonValue]] = {
    "additional_instructions": "be kind",
    "instructions": "be brief",
    "metadata": {"k": "v"},
    "model": "gpt-4o-mini",
    "tools": [{"type": "code_interpreter"}],
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

    @override
    def __exit__(
        self,
        _exc_type: type[BaseException] | None,
        _exc_value: BaseException | None,
        _traceback: TracebackType | None,
    ) -> None:
        self.shutdown()
        self.server_close()
        self.thread.join()


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
        self, *, event_handler: AssistantEventHandler | None = None, **kwargs: CallArgument
    ) -> AssistantStreamManager[AssistantEventHandler]: ...


class AsyncStreamCall(Protocol):
    def __call__(
        self, *, event_handler: AsyncAssistantEventHandler | None = None, **kwargs: CallArgument
    ) -> AsyncAssistantStreamManager[AsyncAssistantEventHandler]: ...


class LiteLLMStreams(Protocol):
    def __getattr__(self, name: str) -> StreamCall: ...


class LiteLLMAsyncStreams(Protocol):
    def __getattr__(self, name: str) -> AsyncStreamCall: ...


class UtilsCall(Protocol):
    def __call__(self, **kwargs: JsonValue) -> dict[str, JsonValue]: ...


class LiteLLMUtils(Protocol):
    def __getattr__(self, name: str) -> UtilsCall: ...


STREAMS: LiteLLMStreams = importlib.import_module("litellm")
ASYNC_STREAMS: LiteLLMAsyncStreams = importlib.import_module("litellm")
THEIR_UTILS: LiteLLMUtils = importlib.import_module("litellm.assistants.utils")
SignatureSource: TypeAlias = Callable[..., object]
SIGNATURES: Final[dict[str, tuple[SignatureSource, SignatureSource]]] = {
    "add_message": (main.add_message, LITELLM.add_message),
    "a_add_message": (main.a_add_message, LITELLM.a_add_message),
    "get_messages": (main.get_messages, LITELLM.get_messages),
    "aget_messages": (main.aget_messages, LITELLM.aget_messages),
    "run_thread": (main.run_thread, LITELLM.run_thread),
    "arun_thread": (main.arun_thread, LITELLM.arun_thread),
    "run_thread_stream": (main.run_thread_stream, LITELLM.run_thread_stream),
    "arun_thread_stream": (main.arun_thread_stream, LITELLM.arun_thread_stream),
    "get_optional_params_add_message": (
        _utils.get_optional_params_add_message,
        THEIR_UTILS.get_optional_params_add_message,
    ),
    "get_optional_params_image_gen": (
        _utils.get_optional_params_image_gen,
        THEIR_UTILS.get_optional_params_image_gen,
    ),
}
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


class OurHandler:
    def __init__(self) -> None:
        self.seen: list[_runs.AssistantStreamEvent] = []

    def on_event(self, event: _runs.AssistantStreamEvent) -> None:
        self.seen.append(event)


class ReferenceHandler(AssistantEventHandler):
    def __init__(self) -> None:
        super().__init__()
        self.names: list[str] = []

    @override
    def on_event(self, event: OpenAIStreamEvent) -> None:
        self.names.append(event.event)


def connection(server: RunServer, key: str = GOOD_KEY) -> dict[str, str]:
    return {"api_key": key, "api_base": server.url}


def stream_arguments(server: RunServer, key: str = GOOD_KEY) -> dict[str, str]:
    return {
        "custom_llm_provider": "openai",
        "thread_id": THREAD_ID,
        "assistant_id": ASSISTANT_ID,
        **connection(server, key),
    }


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


def flagged_run_answer(answer: _runs.RunAnswer) -> _runs.Run:
    match answer:
        case Coroutine() as coroutine:
            return asyncio.run(coroutine)
        case object():
            raise AssertionError("the flag did not answer a coroutine")


def reference_view(event: OpenAIStreamEvent) -> View:
    return {"event": event.event, "data": JSON.validate_json(event.data.model_dump_json())}


def our_view(event: _runs.AssistantStreamEvent) -> View:
    return {"event": event.event, "data": event.data}


def reference_events(manager: AssistantStreamManager[AssistantEventHandler]) -> list[View]:
    with manager as handler:
        return [reference_view(event) for event in handler]


def our_events(stream: _runs.AssistantStream) -> list[View]:
    with stream as events:
        return [our_view(event) for event in events]


async def reference_async_events(
    manager: AsyncAssistantStreamManager[AsyncAssistantEventHandler],
) -> list[View]:
    async with manager as handler:
        return [reference_view(event) async for event in handler]


async def our_async_events(stream: _runs.AsyncAssistantStream) -> list[View]:
    async with stream as events:
        return [our_view(event) async for event in events]


def reference_first(manager: AssistantStreamManager[AssistantEventHandler]) -> View:
    with manager as handler:
        return reference_view(next(iter(handler)))


def our_first(stream: _runs.AssistantStream) -> View:
    with stream as events:
        return our_view(next(events))


async def our_async_first(stream: _runs.AsyncAssistantStream) -> View:
    async with stream as events:
        return our_view(await anext(events))


def closed_after_first(stream: _runs.AssistantStream) -> _runs.AssistantStreamEvent | None:
    with stream as events:
        assert next(events).event == EVENT_NAMES[0]
    return next(stream, None)


async def async_closed_after_first(
    stream: _runs.AsyncAssistantStream,
) -> _runs.AssistantStreamEvent | None:
    async with stream as events:
        assert (await anext(events)).event == EVENT_NAMES[0]
    return await anext(stream, None)


def message_arguments() -> dict[str, JsonValue]:
    return {"attachments": ATTACHMENTS, "metadata": {"k": "v"}}


def test_signature_matches_litellm() -> None:
    for name, (ours, reference) in SIGNATURES.items():
        assert shape(inspect.signature(ours)) == shape(inspect.signature(reference)), name
        assert inspect.iscoroutinefunction(ours) == inspect.iscoroutinefunction(reference), name


def test_signature_stream_helpers_return_managers() -> None:
    assert not inspect.iscoroutinefunction(main.arun_thread_stream)
    assert not inspect.iscoroutinefunction(main.run_thread_stream)
    with RunServer() as server:
        sync_stream = main.run_thread_stream(**stream_arguments(server))
        async_stream = main.arun_thread_stream(**stream_arguments(server))
        assert isinstance(sync_stream, main.AssistantStream)
        assert isinstance(async_stream, main.AsyncAssistantStream)
        assert not server.calls
        with sync_stream as entered:
            assert entered is sync_stream


def test_signature_refuses_client_keyword() -> None:
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
        main.run_thread_stream(**stream_arguments_without_server(), client=FAKE_CLIENT)
    with pytest.raises(UnsupportedCallArgumentError, match="client"):
        main.arun_thread_stream(**stream_arguments_without_server(), client=FAKE_CLIENT)


def stream_arguments_without_server() -> dict[str, str]:
    return {"custom_llm_provider": "openai", "thread_id": THREAD_ID, "assistant_id": ASSISTANT_ID}


def test_calls_add_message_matches_litellm() -> None:
    with RunServer() as server:
        args = connection(server)
        reference = answer_of(
            LITELLM.add_message("openai", THREAD_ID, "user", "hello", **message_arguments(), **args)
        )
        ours = answer_of(
            main.add_message("openai", THREAD_ID, "user", "hello", **message_arguments(), **args)
        )
        matched_calls(server, 1)
        dump_pair(ours, reference)
        reference_plain = answer_of(LITELLM.add_message("openai", THREAD_ID, "user", "hello", **args))
        ours_plain = answer_of(main.add_message("openai", THREAD_ID, "user", "hello", **args))
        matched_calls(server, 1)
        dump_pair(ours_plain, reference_plain)


def test_calls_add_message_without_status_is_completed() -> None:
    with RunServer() as server:
        args = connection(server)
        reference = answer_of(LITELLM.add_message("openai", "thread_bare", "user", "hello", **args))
        ours = answer_of(main.add_message("openai", "thread_bare", "user", "hello", **args))
        matched_calls(server, 1)
        dump_pair(ours, reference)
        assert JSON.validate_json(ours.model_dump_json()) == MESSAGE
        reference_async = answer_of(
            asyncio.run(LITELLM_ASYNC.a_add_message("openai", "thread_bare", "user", "hello", **args))
        )
        ours_async = answer_of(
            asyncio.run(main.a_add_message("openai", "thread_bare", "user", "hello", **args))
        )
        matched_calls(server, 1)
        dump_pair(ours_async, reference_async)


def test_calls_get_messages_matches_litellm() -> None:
    with RunServer() as server:
        args = connection(server)
        reference = page_of(LITELLM.get_messages("openai", THREAD_ID, **args))
        ours = page_of(main.get_messages("openai", THREAD_ID, **args))
        matched_calls(server, 1)
        assert page_view(ours) == page_view(reference)
        assert page_view(ours)["data"] == [MESSAGE]
        assert [message.id for message in main.MessagePage.model_validate(MESSAGE_PAGE)] == [MESSAGE_ID]


def test_calls_run_thread_matches_litellm() -> None:
    with RunServer() as server:
        args = connection(server)
        reference = answer_of(LITELLM.run_thread("openai", THREAD_ID, ASSISTANT_ID, **RUN_OPTIONS, **args))
        ours = run_of(main.run_thread("openai", THREAD_ID, ASSISTANT_ID, **RUN_OPTIONS, **args))
        matched_calls(server, 2)
        dump_pair(ours, reference)
        assert JSON.validate_json(ours.model_dump_json()) == RUN
        reference_plain = answer_of(LITELLM.run_thread("openai", THREAD_ID, ASSISTANT_ID, **args))
        ours_plain = run_of(main.run_thread("openai", THREAD_ID, ASSISTANT_ID, **args))
        matched_calls(server, 2)
        dump_pair(ours_plain, reference_plain)


def test_calls_run_thread_ignores_handler_without_stream() -> None:
    with RunServer() as server:
        args = connection(server)
        handler = OurHandler()
        ours = run_of(
            main.run_thread("openai", THREAD_ID, ASSISTANT_ID, event_handler=handler, **args)
        )
        assert ours.id == RUN_ID
        assert not handler.seen
        for call in server.calls:
            assert "event_handler" not in json.dumps(call.get("body"))


def test_calls_async_forms_match_litellm() -> None:
    with RunServer() as server:
        args = connection(server)
        reference_added = answer_of(
            asyncio.run(
                LITELLM_ASYNC.a_add_message("openai", THREAD_ID, "user", "hello", **message_arguments(), **args)
            )
        )
        ours_added = answer_of(
            asyncio.run(main.a_add_message("openai", THREAD_ID, "user", "hello", **message_arguments(), **args))
        )
        matched_calls(server, 1)
        dump_pair(ours_added, reference_added)
        flagged_added = answer_of(
            flagged_run(main.add_message("openai", THREAD_ID, "user", "hello", a_add_message=True, **args))
        )
        assert server.calls.pop()["method"] == "POST"
        dump_pair(flagged_added, reference_added)
        reference_page = page_of(asyncio.run(LITELLM_ASYNC.aget_messages("openai", THREAD_ID, **args)))
        ours_page = page_of(asyncio.run(main.aget_messages("openai", THREAD_ID, **args)))
        matched_calls(server, 1)
        assert page_view(ours_page) == page_view(reference_page)
        flagged_page = page_of(flagged_run(main.get_messages("openai", THREAD_ID, aget_messages=True, **args)))
        server.calls.clear()
        assert page_view(flagged_page) == page_view(reference_page)
        reference_run = answer_of(
            asyncio.run(LITELLM_ASYNC.arun_thread("openai", THREAD_ID, ASSISTANT_ID, **RUN_OPTIONS, **args))
        )
        ours_run = asyncio.run(main.arun_thread("openai", THREAD_ID, ASSISTANT_ID, **RUN_OPTIONS, **args))
        matched_calls(server, 2)
        dump_pair(run_of(ours_run), reference_run)
        flagged = flagged_run_answer(
            main.run_thread("openai", THREAD_ID, ASSISTANT_ID, arun_thread=True, **args)
        )
        server.calls.clear()
        assert flagged.id == RUN_ID


def test_stream_events_match_litellm() -> None:
    with RunServer() as server:
        theirs = ReferenceHandler()
        reference = reference_events(STREAMS.run_thread_stream(event_handler=theirs, **stream_arguments(server)))
        mine = OurHandler()
        ours = our_events(main.run_thread_stream(event_handler=mine, **stream_arguments(server)))
        matched_calls(server, 1)
        assert ours == reference
        assert [view["event"] for view in ours] == EVENT_NAMES
        assert ours[-1]["data"] == RUN
        assert theirs.names == EVENT_NAMES
        assert [event.event for event in mine.seen] == EVENT_NAMES
        assert [our_view(event) for event in mine.seen] == ours


def test_stream_events_without_handler_match_litellm() -> None:
    with RunServer() as server:
        reference = reference_events(STREAMS.run_thread_stream(**stream_arguments(server)))
        ours = our_events(main.run_thread_stream(**stream_arguments(server)))
        matched_calls(server, 1)
        assert ours == reference


def test_stream_async_events_match_litellm() -> None:
    with RunServer() as server:
        reference = asyncio.run(
            reference_async_events(ASYNC_STREAMS.arun_thread_stream(**stream_arguments(server)))
        )
        handler = OurHandler()
        ours = asyncio.run(
            our_async_events(main.arun_thread_stream(event_handler=handler, **stream_arguments(server)))
        )
        matched_calls(server, 1)
        assert ours == reference
        assert [view["event"] for view in ours] == EVENT_NAMES
        assert [our_view(event) for event in handler.seen] == ours
        sync_events = our_events(main.run_thread_stream(**stream_arguments(server)))
        server.calls.clear()
        assert sync_events == ours


def test_stream_run_thread_flags_return_streams() -> None:
    with RunServer() as server:
        args = connection(server)
        sync_stream = main.run_thread("openai", THREAD_ID, ASSISTANT_ID, stream=True, **args)
        assert isinstance(sync_stream, main.AssistantStream)
        assert [view["event"] for view in our_events(sync_stream)] == EVENT_NAMES
        flagged = main.run_thread("openai", THREAD_ID, ASSISTANT_ID, stream=True, arun_thread=True, **args)
        assert isinstance(flagged, main.AsyncAssistantStream)
        assert [view["event"] for view in asyncio.run(our_async_events(flagged))] == EVENT_NAMES
        awaited = asyncio.run(main.arun_thread("openai", THREAD_ID, ASSISTANT_ID, stream=True, **args))
        assert isinstance(awaited, main.AsyncAssistantStream)
        assert [view["event"] for view in asyncio.run(our_async_events(awaited))] == EVENT_NAMES
        assert len(server.calls) == 3


def test_stream_closes_on_exit() -> None:
    with RunServer() as server:
        stream = main.run_thread_stream(**stream_arguments(server))
        assert closed_after_first(stream) is None
        assert len(server.calls) == 1
        async_stream = main.arun_thread_stream(**stream_arguments(server))
        assert asyncio.run(async_closed_after_first(async_stream)) is None
        assert len(server.calls) == 2
        raising = main.run_thread_stream(**stream_arguments(server))
        with pytest.raises(RuntimeError, match="stop"):
            with raising:
                raise RuntimeError("stop")
        assert next(raising, None) is None
        assert len(server.calls) == 2


def test_stream_waits_for_the_first_read() -> None:
    with RunServer() as server:
        stream = main.run_thread_stream(**stream_arguments(server))
        assert not server.calls
        first = our_first(stream)
        assert first["event"] == EVENT_NAMES[0]
        assert len(server.calls) == 1
        async_first = asyncio.run(our_async_first(main.arun_thread_stream(**stream_arguments(server))))
        assert async_first == first
        reference = reference_first(STREAMS.run_thread_stream(**stream_arguments(server)))
        assert reference == first


def test_stream_handler_must_have_on_event() -> None:
    assert isinstance(OurHandler(), main.EventHandler)
    assert not isinstance(object(), main.EventHandler)
    with pytest.raises(UnsupportedCallArgumentError, match="event_handler"):
        main.run_thread_stream(**stream_arguments_without_server(), event_handler=FAKE_HANDLER)
    with pytest.raises(UnsupportedCallArgumentError, match="event_handler"):
        main.arun_thread_stream(**stream_arguments_without_server(), event_handler=FAKE_HANDLER)
    with pytest.raises(UnsupportedCallArgumentError, match="event_handler"):
        main.run_thread(
            "openai", THREAD_ID, ASSISTANT_ID, stream=True, event_handler=FAKE_HANDLER
        )
    with pytest.raises(TypeError, match="stream"):
        main.run_thread_stream(**stream_arguments_without_server(), stream=True)
    with pytest.raises(TypeError, match="stream"):
        STREAMS.run_thread_stream(**stream_arguments_without_server(), stream=True)


def test_stream_failure_surfaces_on_the_first_read() -> None:
    with RunServer() as server:
        bad = stream_arguments(server, BAD_KEY)
        ours = main.run_thread_stream(**bad)
        assert len(server.calls) == 0
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


def utils_outcome(function: Callable[..., dict[str, JsonValue]], kwargs: dict[str, JsonValue]) -> View:
    try:
        return {"ok": function(**kwargs)}
    except UTILS_FAILURES as failure:
        return {"error": [type(failure).__name__, str(vars(failure).get("message"))]}


UTILS_FAILURES: Final[tuple[type[Exception], ...]] = (
    THEIR_ERRORS.UnsupportedParamsError,
    errors.UnsupportedParamsError,
)
ADD_MESSAGE_CASES: Final[dict[str, dict[str, JsonValue]]] = {
    "openai_everything": {
        "role": "user", "content": "hi", "attachments": ATTACHMENTS, "metadata": {"k": "v"},
        "custom_llm_provider": "openai",
    },
    "openai_nulls_dropped": {
        "role": "user", "content": "hi", "attachments": None, "metadata": None,
        "custom_llm_provider": "openai",
    },
    "openai_extra_key": {
        "role": "assistant", "content": "hi", "attachments": None, "metadata": None,
        "custom_llm_provider": "openai", "extra_flag": True,
    },
    "azure_text": {
        "role": "user", "content": "hi", "attachments": None, "metadata": {"k": "v"},
        "custom_llm_provider": "azure",
    },
    "azure_list_content": {
        "role": "user", "content": [{"type": "text", "text": "hi"}], "attachments": None,
        "metadata": None, "custom_llm_provider": "azure",
    },
    "azure_file_attachment": {
        "role": "user", "content": "hi", "attachments": ATTACHMENTS, "metadata": None,
        "custom_llm_provider": "azure",
    },
    "azure_plain_attachment": {
        "role": "user", "content": "hi", "attachments": [{"text": "x"}], "metadata": None,
        "custom_llm_provider": "azure",
    },
    "azure_attachments_not_a_list": {
        "role": "user", "content": "hi", "attachments": "nope", "metadata": None,
        "custom_llm_provider": "azure",
    },
    "other_provider_extra_key": {
        "role": "user", "content": "hi", "attachments": None, "metadata": None,
        "custom_llm_provider": "bedrock", "extra_flag": 1,
    },
}  # fmt: skip
IMAGE_CASES: Final[dict[str, dict[str, JsonValue]]] = {
    "openai_everything": {
        "n": 2, "quality": "hd", "response_format": "url", "size": "1024x1024",
        "style": "vivid", "user": "u1", "custom_llm_provider": "openai",
    },
    "openai_compatible_provider": {"n": 1, "size": "256x256", "custom_llm_provider": "groq"},
    "azure_extra_key": {"n": 1, "custom_llm_provider": "azure", "extra_flag": "x"},
    "bedrock_size": {"size": "512x768", "custom_llm_provider": "bedrock"},
    "bedrock_size_and_count": {"size": "512x768", "n": 2, "custom_llm_provider": "bedrock"},
    "bedrock_nothing": {"custom_llm_provider": "bedrock", "extra_flag": 1},
    "vertex_count": {"n": 3, "custom_llm_provider": "vertex_ai"},
    "vertex_count_and_size": {"n": 3, "size": "512x512", "custom_llm_provider": "vertex_ai"},
    "unknown_provider": {"n": 3, "size": "512x512", "custom_llm_provider": "unknown"},
    "no_provider": {"n": 3, "extra_flag": 1},
}  # fmt: skip


@pytest.mark.parametrize("name", list(ADD_MESSAGE_CASES))
def test_utils_add_message_params_match_litellm(name: str, monkeypatch: pytest.MonkeyPatch) -> None:
    kwargs = ADD_MESSAGE_CASES[name]
    for dropping in (False, True):
        monkeypatch.setattr("litellm.drop_params", dropping)
        monkeypatch.setattr("betterlite.drop_params", dropping)
        ours = utils_outcome(_utils.get_optional_params_add_message, kwargs)
        reference = utils_outcome(THEIR_UTILS.get_optional_params_add_message, kwargs)
        assert ours == reference


@pytest.mark.parametrize("name", list(IMAGE_CASES))
def test_utils_image_gen_params_match_litellm(name: str, monkeypatch: pytest.MonkeyPatch) -> None:
    kwargs = IMAGE_CASES[name]
    for dropping in (False, True):
        monkeypatch.setattr("litellm.drop_params", dropping)
        monkeypatch.setattr("betterlite.drop_params", dropping)
        ours = utils_outcome(_utils.get_optional_params_image_gen, kwargs)
        reference = utils_outcome(THEIR_UTILS.get_optional_params_image_gen, kwargs)
        assert ours == reference


def test_utils_outcomes_cover_both_branches() -> None:
    refused = utils_outcome(_utils.get_optional_params_add_message, ADD_MESSAGE_CASES["azure_list_content"])
    assert refused["error"]
    accepted = utils_outcome(_utils.get_optional_params_add_message, ADD_MESSAGE_CASES["azure_text"])
    assert accepted["ok"] == {"role": "user", "content": "hi", "metadata": {"k": "v"}}
    sized = utils_outcome(_utils.get_optional_params_image_gen, IMAGE_CASES["bedrock_size"])
    assert sized["ok"] == {"width": 512, "height": 768}


def test_package_main_exports_every_name() -> None:
    package = importlib.import_module("betterlite.assistants")
    reference = vars(importlib.import_module("litellm.assistants.main"))
    for name in EXPORTED_NAMES:
        assert name in main.__all__, name
        assert vars(package)[name] is vars(main)[name], name
    for name in LITELLM_FUNCTIONS:
        assert name in reference, name
        assert name in main.__all__, name


def test_package_utils_module_is_registered() -> None:
    utils = importlib.import_module("betterlite.assistants.utils")
    reference = importlib.import_module("litellm.assistants.utils")
    assert utils is _utils
    for name in ("get_optional_params_add_message", "get_optional_params_image_gen"):
        assert name in vars(utils)
        assert name in vars(reference)


def assert_denied(ours: errors.AuthenticationError, reference: openai.AuthenticationError) -> None:
    assert type(ours).__name__ == type(reference).__name__
    assert ours.status_code == UNAUTHORIZED
    assert "Incorrect API key" in str(ours)
    assert "Incorrect API key" in str(reference)
    assert ours.__cause__ is not None


def test_error_class_and_cause_for_message_calls() -> None:
    with RunServer() as server:
        bad = connection(server, BAD_KEY)
        with pytest.raises(errors.AuthenticationError) as ours:
            main.add_message("openai", THREAD_ID, "user", "hi", **bad)
        with pytest.raises(openai.AuthenticationError) as reference:
            LITELLM.add_message("openai", THREAD_ID, "user", "hi", **bad)
        assert_denied(ours.value, reference.value)
        with pytest.raises(errors.AuthenticationError) as ours_list:
            main.get_messages("openai", THREAD_ID, **bad)
        with pytest.raises(openai.AuthenticationError) as reference_list:
            LITELLM.get_messages("openai", THREAD_ID, **bad)
        assert_denied(ours_list.value, reference_list.value)
        with pytest.raises(errors.AuthenticationError) as ours_async:
            asyncio.run(main.a_add_message("openai", THREAD_ID, "user", "hi", **bad))
        with pytest.raises(openai.AuthenticationError) as reference_async:
            asyncio.run(LITELLM_ASYNC.a_add_message("openai", THREAD_ID, "user", "hi", **bad))
        assert_denied(ours_async.value, reference_async.value)
        with pytest.raises(errors.AuthenticationError) as ours_pages:
            asyncio.run(main.aget_messages("openai", THREAD_ID, **bad))
        with pytest.raises(openai.AuthenticationError) as reference_pages:
            asyncio.run(LITELLM_ASYNC.aget_messages("openai", THREAD_ID, **bad))
        assert_denied(ours_pages.value, reference_pages.value)


def test_error_class_and_cause_for_run_calls() -> None:
    with RunServer() as server:
        bad = connection(server, BAD_KEY)
        with pytest.raises(errors.AuthenticationError) as ours:
            main.run_thread("openai", THREAD_ID, ASSISTANT_ID, **bad)
        with pytest.raises(openai.AuthenticationError) as reference:
            LITELLM.run_thread("openai", THREAD_ID, ASSISTANT_ID, **bad)
        assert_denied(ours.value, reference.value)
        with pytest.raises(errors.AuthenticationError) as ours_async:
            asyncio.run(main.arun_thread("openai", THREAD_ID, ASSISTANT_ID, **bad))
        with pytest.raises(openai.AuthenticationError) as reference_async:
            asyncio.run(LITELLM_ASYNC.arun_thread("openai", THREAD_ID, ASSISTANT_ID, **bad))
        assert_denied(ours_async.value, reference_async.value)


def test_error_rate_limit_keeps_its_class() -> None:
    with RunServer() as server:
        limited = {**connection(server, RATE_LIMITED_KEY), "max_retries": 0}
        with pytest.raises(errors.RateLimitError) as ours_message:
            main.add_message("openai", THREAD_ID, "user", "hi", **limited)
        with pytest.raises(openai.RateLimitError):
            LITELLM.add_message("openai", THREAD_ID, "user", "hi", **limited)
        assert ours_message.value.status_code == RATE_LIMIT_STATUS
        with pytest.raises(errors.RateLimitError) as ours_run:
            main.run_thread("openai", THREAD_ID, ASSISTANT_ID, **limited)
        with pytest.raises(openai.RateLimitError):
            LITELLM.run_thread("openai", THREAD_ID, ASSISTANT_ID, **limited)
        assert ours_run.value.status_code == RATE_LIMIT_STATUS
        assert ours_run.value.__cause__ is not None


def test_error_unsupported_provider_is_a_bad_request() -> None:
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
        our_first(main.run_thread_stream(custom_llm_provider="bedrock", thread_id=THREAD_ID, assistant_id=ASSISTANT_ID))


def test_error_unreadable_reply_names_the_field() -> None:
    assert main.Message.model_validate(MESSAGE).id == MESSAGE_ID
    with pytest.raises(ValueError, match="role"):
        main.Message.model_validate({**MESSAGE, "role": None})
