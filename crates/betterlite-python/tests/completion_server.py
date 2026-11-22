import importlib
import json
import threading
from collections.abc import AsyncIterator, Awaitable, Iterator
from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from types import TracebackType
from typing import ClassVar, Final, Protocol, Self, TypeAlias, runtime_checkable

from pydantic import JsonValue, TypeAdapter
from typing_extensions import TypedDict, override

from betterlite._completion import adapters, arguments
from betterlite._completion.adapters import AdapterEntry
from betterlite._completion.arguments import SessionLike
from betterlite._completion.stream import CompletionOutcome, CustomStreamWrapper
from betterlite.types.utils import Message, ModelResponse

HOST = "127.0.0.1"
OK = 200
BAD_REQUEST = 400
UNAUTHORIZED = 401
NOT_FOUND = 404
RATE_LIMITED = 429
BROKEN = 500
GOOD_KEY = "sk-test"
BAD_KEY = "sk-denied"
RATE_LIMITED_KEY = "sk-rate"
CONTENT = "hello"
STREAM_PARTS = ("a", "b", "c")
GOOD_MODEL = "gpt-4o"
RATE_LIMITED_MODEL = "rate-limited"
UNKNOWN_MODEL = "bad-model"
BROKEN_MODEL = "broken"
CREATED = 1700000000
COMPLETION_ID = "chatcmpl-1"
USAGE_PATH = "/v1/responses/input_tokens"
MODELS_PATH = "/v1/models/"
CHAT_PATH = "/v1/chat/completions"
USAGE: dict[str, JsonValue] = {
    "prompt_tokens": 2, "completion_tokens": 1, "total_tokens": 3,
}


class CompletionRequest(TypedDict, total=False):
    model: str
    messages: list[dict[str, JsonValue]]
    stream: bool
    response_format: dict[str, JsonValue]
    max_tokens: int
    user: str


class RecordedCall(TypedDict):
    path: str
    headers: dict[str, str]
    body: CompletionRequest


REQUEST: TypeAdapter[CompletionRequest] = TypeAdapter(CompletionRequest)


def completion_payload(model: str) -> dict[str, JsonValue]:
    message: dict[str, JsonValue] = {"role": "assistant", "content": CONTENT}
    choice: dict[str, JsonValue] = {
        "index": 0, "message": message, "finish_reason": "stop",
    }
    return {
        "id": COMPLETION_ID, "object": "chat.completion", "created": CREATED,
        "model": model, "choices": [choice], "usage": USAGE,
    }


def stream_payload(model: str, content: str | None, finish: str | None) -> dict[str, JsonValue]:
    delta: dict[str, JsonValue] = {} if content is None else {"content": content}
    return {
        "id": COMPLETION_ID, "object": "chat.completion.chunk", "created": CREATED,
        "model": model,
        "choices": [{"index": 0, "delta": delta, "finish_reason": finish}],
    }


class CompletionServer(ThreadingHTTPServer):
    daemon_threads = True
    calls: list[RecordedCall]

    def __init__(self) -> None:
        super().__init__((HOST, 0), _Handler)
        self.calls = []
        _Handler.owner = self
        self.thread = threading.Thread(target=self.serve_forever, daemon=True)
        self.thread.start()

    @property
    def url(self) -> str:
        return f"http://{HOST}:{self.server_port}/v1"

    def record(self, path: str, headers: dict[str, str], body: CompletionRequest) -> None:
        self.calls.append({"path": path, "headers": headers, "body": body})

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
    owner: ClassVar["CompletionServer"]

    def do_GET(self) -> None:
        self.owner.record(self.path, dict(self.headers.items()), {})
        if self.headers.get("Authorization") != f"Bearer {GOOD_KEY}":
            self.reply_error(UNAUTHORIZED, "invalid api key")
            return
        if not self.path.startswith(MODELS_PATH):
            self.send_error(NOT_FOUND)
            return
        model = self.path.removeprefix(MODELS_PATH)
        if model != GOOD_MODEL:
            self.reply_error(NOT_FOUND, f"model {model} does not exist")
            return
        self.reply(OK, {"id": model, "object": "model"})

    def do_POST(self) -> None:
        declared = self.headers.get("Content-Length")
        raw = b"{}" if declared is None else self.rfile.read(int(declared))
        body = REQUEST.validate_json(raw)
        self.owner.record(self.path, dict(self.headers.items()), body)
        if self.path == USAGE_PATH:
            self.reply_error(NOT_FOUND, "no token counting endpoint")
            return
        authorized = self.headers.get("Authorization")
        if authorized == f"Bearer {RATE_LIMITED_KEY}":
            self.reply_error(RATE_LIMITED, "rate limit reached")
            return
        if authorized != f"Bearer {GOOD_KEY}":
            self.reply_error(UNAUTHORIZED, "invalid api key")
            return
        model = body.get("model")
        if model is None:
            self.reply_error(BAD_REQUEST, "model is required")
            return
        match model:
            case str() if model == UNKNOWN_MODEL:
                self.reply_error(NOT_FOUND, "model does not exist")
            case str() if model == BROKEN_MODEL:
                self.reply_error(BROKEN, "provider failed")
            case str() if model == RATE_LIMITED_MODEL:
                self.reply_error(RATE_LIMITED, "rate limit reached")
            case str() if body.get("stream") is True:
                self.stream(model)
            case str():
                self.reply(OK, completion_payload(model))

    def reply_error(self, status: int, message: str) -> None:
        payload: dict[str, JsonValue] = {
            "error": {"message": message, "type": "invalid_request_error"},
        }
        self.reply(status, payload)

    def reply(self, status: int, payload: dict[str, JsonValue]) -> None:
        encoded = json.dumps(payload).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)

    def stream(self, model: str) -> None:
        self.send_response(OK)
        self.send_header("Content-Type", "text/event-stream")
        self.end_headers()
        for part in STREAM_PARTS:
            frame = stream_payload(model, part, None)
            self.wfile.write(f"data: {json.dumps(frame)}\n\n".encode())
            self.wfile.flush()
        finish = stream_payload(model, None, "stop")
        self.wfile.write(f"data: {json.dumps(finish)}\n\n".encode())
        self.wfile.write(b"data: [DONE]\n\n")
        self.wfile.flush()


class ChunkDelta(TypedDict, total=False):
    content: str
    role: str


class StreamChoice(TypedDict):
    index: int
    delta: ChunkDelta
    finish_reason: str | None


class ChunkView(TypedDict):
    choices: list[StreamChoice]


CHUNK_VIEW: Final[TypeAdapter[ChunkView]] = TypeAdapter(ChunkView)
DROPPED: Final[frozenset[str]] = frozenset({"id", "created"})
StreamParts: TypeAlias = list[tuple[int, str | None, str | None, str | None]]
CallArgument: TypeAlias = JsonValue | SessionLike
OwnResult: TypeAlias = CompletionOutcome | Awaitable[CompletionOutcome] | None


@runtime_checkable
class Dumpable(Protocol):
    def model_dump(self) -> dict[str, JsonValue]: ...


@runtime_checkable
class StreamOf(Protocol):
    def __iter__(self) -> Iterator[Dumpable]: ...


@runtime_checkable
class AsyncStreamOf(Protocol):
    def __aiter__(self) -> AsyncIterator[Dumpable]: ...


RefResult: TypeAlias = Dumpable | StreamOf | AsyncStreamOf | None


@runtime_checkable
class ErrorFields(Protocol):
    message: str
    status_code: int
    llm_provider: str
    model: str


class CountView(Protocol):
    total_tokens: int
    request_model: str
    model_used: str
    tokenizer_type: str


class SyncCall(Protocol):
    def __call__(self, *args: CallArgument, **kwargs: CallArgument) -> RefResult: ...


class AsyncCall(Protocol):
    def __call__(
        self, *args: CallArgument, **kwargs: CallArgument
    ) -> Awaitable[RefResult]: ...


class CountCall(Protocol):
    def __call__(self, *args: CallArgument, **kwargs: CallArgument) -> Awaitable[CountView]: ...


class Router(Protocol):
    def completion(
        self,
        model: str | None,
        messages: list[dict[str, JsonValue] | Message],
        **params: JsonValue,
    ) -> CompletionOutcome: ...

    def acompletion(
        self,
        model: str | None,
        messages: list[dict[str, JsonValue] | Message],
        **params: JsonValue,
    ) -> Awaitable[CompletionOutcome]: ...


class SyncCreate(Protocol):
    def create(
        self,
        messages: list[dict[str, JsonValue] | Message],
        model: str | None = None,
        **kwargs: JsonValue,
    ) -> CompletionOutcome: ...


class AsyncCreate(Protocol):
    def create(
        self,
        messages: list[dict[str, JsonValue] | Message],
        model: str | None = None,
        **kwargs: JsonValue,
    ) -> Awaitable[CompletionOutcome]: ...


class SyncFactory(Protocol):
    def __call__(
        self, params: dict[str, JsonValue], router_obj: Router | None
    ) -> SyncCreate: ...


class AsyncFactory(Protocol):
    def __call__(
        self, params: dict[str, JsonValue], router_obj: Router | None
    ) -> AsyncCreate: ...


class ApiErrorCall(Protocol):
    def __call__(
        self, message: str, request: JsonValue, *, body: JsonValue
    ) -> BaseException: ...


class LiteLLMSync(Protocol):
    def __getattr__(self, name: str) -> SyncCall: ...


class LiteLLMAsync(Protocol):
    def __getattr__(self, name: str) -> AsyncCall: ...


class ReferenceClient(Protocol):
    params: dict[str, JsonValue]


class ClientFactory(Protocol):
    def __call__(
        self,
        *,
        api_key: JsonValue = None,
        organization: JsonValue = None,
        base_url: JsonValue = None,
        timeout: JsonValue = 600,
        max_retries: JsonValue = None,
    ) -> ReferenceClient: ...


class LiteLLMClient(Protocol):
    def __getattr__(self, name: str) -> ClientFactory: ...


class LiteLLMCounts(Protocol):
    def __getattr__(self, name: str) -> CountCall: ...


class LiteLLMClasses(Protocol):
    def __getattr__(self, name: str) -> SyncFactory: ...


class LiteLLMAsyncClasses(Protocol):
    def __getattr__(self, name: str) -> AsyncFactory: ...


class AsyncChatView(Protocol):
    params: dict[str, JsonValue]
    completions: AsyncCreate


class ChatFactory(Protocol):
    def __call__(
        self, params: dict[str, JsonValue], router_obj: Router | None
    ) -> AsyncChatView: ...


class LiteLLMChats(Protocol):
    def __getattr__(self, name: str) -> ChatFactory: ...


class OpenaiModule(Protocol):
    def __getattr__(self, name: str) -> ApiErrorCall: ...


LITELLM: LiteLLMSync = importlib.import_module("litellm")
LITELLM_ASYNC: LiteLLMAsync = importlib.import_module("litellm")
LITELLM_CLASSES: LiteLLMClasses = importlib.import_module("litellm")
LITELLM_ASYNC_CLASSES: LiteLLMAsyncClasses = importlib.import_module("litellm")
LITELLM_CHAT: LiteLLMChats = importlib.import_module("litellm")
LITELLM_CLIENT: LiteLLMClient = importlib.import_module("litellm")
LITELLM_COUNTS: LiteLLMCounts = importlib.import_module("litellm.main")
OPENAI: OpenaiModule = importlib.import_module("openai")


class FakeAdapter:
    def __init__(self) -> None:
        self.input_calls = 0
        self.output_calls = 0
        self.stream_calls = 0

    def translate_completion_input_params(  # ast-grep-ignore: no-long-names
        self, *, kwargs: dict[str, JsonValue]
    ) -> dict[str, JsonValue]:
        self.input_calls += 1
        return {**kwargs, "model": GOOD_MODEL}

    def translate_completion_output_params(  # ast-grep-ignore: no-long-names
        self, *, response: ModelResponse
    ) -> ModelResponse:
        self.output_calls += 1
        return response

    def translate_completion_output_params_streaming(  # ast-grep-ignore: no-long-names
        self, *, completion_stream: CustomStreamWrapper
    ) -> CompletionOutcome | None:
        self.stream_calls += 1
        return completion_stream


@contextmanager
def config_seam(config_file: Path) -> Iterator[None]:
    arguments.config_path = str(config_file)
    try:
        yield
    finally:
        arguments.config_path = None


@contextmanager
def adapter_seam(adapter: FakeAdapter, adapter_id: str) -> Iterator[None]:
    entries: list[AdapterEntry] = [{"id": adapter_id, "adapter": adapter}]
    adapters.adapter_entries = entries
    try:
        yield
    finally:
        adapters.adapter_entries = None


def normalized(dump: dict[str, JsonValue]) -> dict[str, JsonValue]:
    kept: dict[str, JsonValue] = {}
    for name, value in dump.items():
        if name in DROPPED:
            continue
        kept[name] = value
    return kept


def chunk_parts(
    dump: dict[str, JsonValue],
) -> tuple[int, str | None, str | None, str | None]:
    view = CHUNK_VIEW.validate_python(dump)
    choice = view["choices"][0]
    delta = choice["delta"]
    return (choice["index"], delta.get("content"), delta.get("role"), choice["finish_reason"])


def header_of(call_record: RecordedCall, name: str) -> str | None:
    lowered = name.lower()
    for header, value in call_record["headers"].items():
        if header.lower() == lowered:
            return value
    return None


def error_view(error: BaseException | ErrorFields) -> tuple[str, str, int, str, str]:
    match error:
        case ErrorFields():
            return (
                type(error).__name__, error.message, error.status_code,
                error.llm_provider, error.model,
            )
        case object():
            raise AssertionError("the error does not carry the mock fields")


def model_response(outcome: OwnResult) -> ModelResponse:
    match outcome:
        case ModelResponse():
            return outcome
        case CustomStreamWrapper() | Awaitable() | None:
            raise AssertionError("a plain call answered with a stream, an awaitable or nothing")


def stream_response(outcome: OwnResult) -> CustomStreamWrapper:
    match outcome:
        case ModelResponse() | Awaitable() | None:
            raise AssertionError("a streaming call answered with a response or nothing")
        case CustomStreamWrapper():
            return outcome


def dump_of(outcome: OwnResult | RefResult) -> dict[str, JsonValue]:
    match outcome:
        case ModelResponse():
            return outcome.model_dump()
        case CustomStreamWrapper():
            raise AssertionError("the call answered with a stream")
        case Dumpable():
            return outcome.model_dump()
        case object():
            raise AssertionError("the call answered with an awaitable or nothing")


def litellm_plain(**kwargs: CallArgument) -> dict[str, JsonValue]:
    result = LITELLM.completion(**kwargs)
    match result:
        case Dumpable():
            return result.model_dump()
        case object():
            raise AssertionError("the reference call answered with no model response")


async def litellm_async_plain(**kwargs: CallArgument) -> dict[str, JsonValue]:
    result = await LITELLM_ASYNC.acompletion(**kwargs)
    match result:
        case Dumpable():
            return result.model_dump()
        case object():
            raise AssertionError("the reference call answered with no model response")


def litellm_stream_parts(**kwargs: CallArgument) -> StreamParts:
    result = LITELLM.completion(**kwargs)
    match result:
        case Dumpable():
            raise AssertionError("a streaming call answered with a response")
        case StreamOf():
            return [chunk_parts(chunk.model_dump()) for chunk in result]
        case object():
            raise AssertionError("the reference call answered with no stream")


async def litellm_async_stream_parts(**kwargs: CallArgument) -> StreamParts:
    result = await LITELLM_ASYNC.acompletion(**kwargs)
    match result:
        case Dumpable():
            raise AssertionError("a streaming call answered with a response")
        case AsyncStreamOf():
            parts: StreamParts = []
            async for chunk in result:
                parts.append(chunk_parts(chunk.model_dump()))
            return parts
        case object():
            raise AssertionError("the reference call answered with no stream")


def ours_stream_parts(stream: CustomStreamWrapper) -> StreamParts:
    return [
        chunk_parts(chunk.model_dump())
        for chunk in stream
    ]


async def ours_async_stream_parts(stream: CustomStreamWrapper) -> StreamParts:
    return [
        chunk_parts(chunk.model_dump())
        async for chunk in stream
    ]
