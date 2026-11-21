import asyncio
import json
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from types import TracebackType
from typing import ClassVar, Self, TypedDict

import pytest
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

from betterlite import _native

HOST = "127.0.0.1"
OK = 200
DENIED = 401
NOT_FOUND = 404
BAD_REQUEST = 400
SLOW = 0.5
TICK = 0.01
SERIAL_LIMIT = SLOW * 2
COUNTER_FLOOR = 5
PARALLEL = 2

COMPLETION = (
    '{"id": "chatcmpl-1", "object": "chat.completion", "created": 1700000000,'
    ' "model": "gpt-4o", "choices": [{"index": 0,'
    ' "message": {"role": "assistant", "content": "hello"}, "finish_reason": "stop"}]}'
)
MODELS = '{"object": "list", "data": [{"id": "gpt-4o", "object": "model"}]}'
ERROR_BODY = b'{"error": {"message": "denied", "type": "invalid_request_error"}}'
STREAM_CONTENTS = ["a", "b", "c"]


class ChatBody(TypedDict, total=False):
    stream: bool


class Message(TypedDict):
    role: str
    content: str


class Choice(TypedDict):
    message: Message


class Completion(TypedDict):
    model: str
    choices: list[Choice]


class Delta(TypedDict, total=False):
    content: str


class StreamChoice(TypedDict, total=False):
    delta: Delta


class Chunk(TypedDict):
    choices: list[StreamChoice]


class TokenCount(TypedDict):
    total_tokens: int
    request_model: str
    model_used: str
    tokenizer_type: str


class ErrorPayload(TypedDict):
    type: str
    message: str
    status_code: int | None
    llm_provider: str | None
    model: str | None


class Recorder:
    def __init__(self, *, delay: float = 0.0, status: int = OK) -> None:
        self.delay = delay
        self.status = status
        self.active = 0
        self.max_active = 0
        self.lock = threading.Lock()


class Server(ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self, recorder: Recorder) -> None:
        super().__init__((HOST, 0), _Handler)
        self.recorder = recorder
        self.thread = threading.Thread(target=self.serve_forever, daemon=True)
        self.thread.start()
        _Handler.recording = self

    @property
    def url(self) -> str:
        return f"http://{HOST}:{self.server_port}/v1"

    @override
    def __enter__(self) -> Self:
        return self

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


class _Handler(BaseHTTPRequestHandler):
    recording: ClassVar[Server | None] = None

    def do_POST(self) -> None:
        recorder = self._server().recorder
        length = int(self.headers.get("Content-Length") or "0")
        raw = self.rfile.read(length)
        body = TypeAdapter(ChatBody).validate_json(raw if raw else b"{}")
        with recorder.lock:
            recorder.active += 1
            recorder.max_active = max(recorder.max_active, recorder.active)
        try:
            time.sleep(recorder.delay)
            if recorder.status != OK:
                self._reply(recorder.status, ERROR_BODY)
            elif body.get("stream", False):
                self._stream()
            else:
                self._reply(OK, COMPLETION.encode())
        finally:
            with recorder.lock:
                recorder.active -= 1

    def do_GET(self) -> None:
        recorder = self._server().recorder
        if recorder.status != OK:
            self._reply(recorder.status, ERROR_BODY)
        elif self.path.endswith("/gpt-4o"):
            self._reply(OK, MODELS.encode())
        else:
            self._reply(NOT_FOUND, ERROR_BODY)

    def _server(self) -> Server:
        server = _Handler.recording
        if server is None:
            raise RuntimeError("the handler is not on a Server")
        return server

    def _reply(self, status: int, payload: bytes) -> None:
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def _stream(self) -> None:
        self.send_response(OK)
        self.send_header("Content-Type", "text/event-stream")
        self.end_headers()
        for content in STREAM_CONTENTS:
            frame = {
                "id": "chatcmpl-1",
                "object": "chat.completion.chunk",
                "created": 1700000000,
                "model": "gpt-4o",
                "choices": [{"index": 0, "delta": {"content": content}, "finish_reason": None}],
            }
            self.wfile.write(f"data: {json.dumps(frame)}\n\n".encode())
            self.wfile.flush()
        self.wfile.write(b"data: [DONE]\n\n")
        self.wfile.flush()


class Progress:
    def __init__(self) -> None:
        self.count = 0
        self.stopped = threading.Event()

    def run_until_stopped(self) -> None:
        while not self.stopped.is_set():
            self.count += 1
            time.sleep(TICK)

    def stop(self) -> None:
        self.stopped.set()


def session_for(server: Server) -> _native.NativeSession:
    environment = {"OPENAI_API_BASE": server.url, "OPENAI_API_KEY": "sk-test"}
    return _native.NativeSession(json.dumps({"environment": environment}))


def completion_request(*, stream: bool = False) -> str:
    arguments: dict[str, JsonValue] = {
        "model": "gpt-4o",
        "messages": [{"role": "user", "content": "hi"}],
    }
    if stream:
        arguments["stream"] = True
    return json.dumps({"arguments": arguments, "settings": {}})


def response_text(result: str | _native.NativeChatStream) -> str:
    if isinstance(result, _native.NativeChatStream):
        raise AssertionError("a plain call answered with a stream")
    return result


def contents_of(chunks: list[str]) -> list[str]:
    contents: list[str] = []
    for text in chunks:
        chunk = TypeAdapter(Chunk).validate_json(text)
        for choice in chunk["choices"]:
            delta = choice.get("delta")
            if delta is None:
                continue
            content = delta.get("content")
            if content is not None:
                contents.append(content)
    return contents


def drain_blocking(stream: _native.NativeChatStream) -> list[str]:
    chunks: list[str] = []
    while True:
        chunk = stream.next_chunk()
        if chunk is None:
            break
        chunks.append(chunk)
    return chunks


async def drain_async(stream: _native.NativeChatStream) -> list[str]:
    chunks: list[str] = []
    while True:
        chunk = await stream.anext_chunk()
        if chunk is None:
            break
        chunks.append(chunk)
    return chunks


def test_native_completion_blocking_returns_and_releases_the_gil() -> None:
    recorder = Recorder(delay=SLOW)
    with Server(recorder) as server:
        session = session_for(server)
        progress = Progress()
        worker = threading.Thread(target=progress.run_until_stopped, daemon=True)
        worker.start()
        try:
            text = response_text(session.completion(completion_request()))
        finally:
            progress.stop()
            worker.join()
    payload = TypeAdapter(Completion).validate_json(text)
    assert payload["choices"][0]["message"]["content"] == "hello"
    assert progress.count > COUNTER_FLOOR


def test_native_completion_async_call_runs_alongside_a_second_call() -> None:
    recorder = Recorder(delay=SLOW)
    with Server(recorder) as server:
        session = session_for(server)

        async def both() -> list[str]:
            first_call = asyncio.ensure_future(session.acompletion(completion_request()))
            second_call = asyncio.ensure_future(session.acompletion(completion_request()))
            first = await first_call
            second = await second_call
            return [response_text(first), response_text(second)]

        started = time.monotonic()
        results = asyncio.run(both())
        elapsed = time.monotonic() - started
    assert len(results) == PARALLEL
    assert all(TypeAdapter(Completion).validate_json(text)["model"] == "gpt-4o" for text in results)
    assert recorder.max_active == PARALLEL
    assert elapsed < SERIAL_LIMIT


def test_native_stream_blocking_gives_chunks_then_none() -> None:
    with Server(Recorder()) as server:
        session = session_for(server)
        answer = session.completion(completion_request(stream=True))
        assert isinstance(answer, _native.NativeChatStream)
        chunks = drain_blocking(answer)
        assert contents_of(chunks) == STREAM_CONTENTS
        answer.close()
        answer.close()
        assert answer.next_chunk() is None


def test_native_stream_async_gives_chunks_then_none() -> None:
    with Server(Recorder()) as server:
        session = session_for(server)

        async def consume() -> list[str]:
            answer = await session.acompletion(completion_request(stream=True))
            assert isinstance(answer, _native.NativeChatStream)
            chunks = await drain_async(answer)
            await answer.aclose()
            await answer.aclose()
            assert await answer.anext_chunk() is None
            return chunks

        chunks = asyncio.run(consume())
    assert contents_of(chunks) == STREAM_CONTENTS


def test_native_checks_count_tokens_counts_messages() -> None:
    with Server(Recorder()) as server:
        session = session_for(server)
        request = json.dumps(
            {
                "model": "gpt-4o",
                "messages": [{"role": "user", "content": "hello there"}],
                "system": "be brief",
            }
        )
        counted = TypeAdapter(TokenCount).validate_json(session.count_tokens(request))
    assert counted["total_tokens"] > 0
    assert counted["request_model"] == "gpt-4o"
    assert counted["model_used"] == "gpt-4o"
    assert counted["tokenizer_type"] == "local_tokenizer"


def test_native_checks_check_valid_key_follows_the_answer() -> None:
    request = json.dumps({"model": "gpt-4o", "api_key": "sk-test"})
    with Server(Recorder()) as server:
        accepted = session_for(server).check_valid_key(request)
    assert accepted
    denied = Recorder(status=DENIED)
    with Server(denied) as server:
        rejected = session_for(server).check_valid_key(request)
    assert not rejected


def test_native_checks_valid_model_raises_bad_request_on_failure() -> None:
    request = json.dumps({"model": "gpt-4o"})
    with Server(Recorder()) as server:
        session_for(server).valid_model(request)
    absent = Server(Recorder(status=NOT_FOUND))
    with absent, pytest.raises(_native.NativeError) as raised:
        session_for(absent).valid_model(request)
    payload = TypeAdapter(ErrorPayload).validate_json(str(raised.value))
    assert payload["type"] == "BadRequestError"
    assert payload["status_code"] == BAD_REQUEST
    assert payload["model"] == "gpt-4o"
    assert payload["llm_provider"] == ""
