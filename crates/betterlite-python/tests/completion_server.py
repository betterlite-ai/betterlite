import importlib
import json
import threading
from collections.abc import AsyncIterator, Awaitable, Callable, Coroutine, Iterator
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from types import TracebackType
from typing import ClassVar, Final, Protocol, Self, TypeAlias, runtime_checkable

from pydantic import BaseModel, JsonValue, TypeAdapter
from typing_extensions import TypedDict

from betterlite._completion.acompletion import acompletion
from betterlite._completion.completion import completion
from betterlite._completion.stream import CompletionOutcome, CustomStreamWrapper
from betterlite.types.utils import ModelResponse

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
TOKEN_COUNT_PATH = "/v1/responses/input_tokens"
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


def error_payload(message: str) -> dict[str, JsonValue]:
    return {"error": {"message": message, "type": "invalid_request_error"}}


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
    choice: dict[str, JsonValue] = {
        "index": 0, "delta": delta, "finish_reason": finish,
    }
    return {
        "id": COMPLETION_ID, "object": "chat.completion.chunk", "created": CREATED,
        "model": model, "choices": [choice],
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

    def __enter__(self) -> Self:
        return self

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

    def log_message(self, _format: str, *_args: str) -> None:
        return

    def do_GET(self) -> None:
        self.owner.record(self.path, dict(self.headers.items()), {})
        if self.headers.get("Authorization") != f"Bearer {GOOD_KEY}":
            self.reply(UNAUTHORIZED, error_payload("invalid api key"))
            return
        if not self.path.startswith(MODELS_PATH):
            self.send_error(NOT_FOUND)
            return
        model = self.path.removeprefix(MODELS_PATH)
        if model != GOOD_MODEL:
            self.reply(NOT_FOUND, error_payload(f"model {model} does not exist"))
            return
        self.reply(OK, {"id": model, "object": "model"})

    def do_POST(self) -> None:
        declared = self.headers.get("Content-Length")
        raw = b"{}" if declared is None else self.rfile.read(int(declared))
        body = REQUEST.validate_json(raw)
        self.owner.record(self.path, dict(self.headers.items()), body)
        if self.path == TOKEN_COUNT_PATH:
            self.reply(NOT_FOUND, error_payload("no token counting endpoint"))
            return
        authorized = self.headers.get("Authorization")
        if authorized == f"Bearer {RATE_LIMITED_KEY}":
            self.reply(RATE_LIMITED, error_payload("rate limit reached"))
            return
        if authorized != f"Bearer {GOOD_KEY}":
            self.reply(UNAUTHORIZED, error_payload("invalid api key"))
            return
        model = body.get("model")
        match model:
            case None:
                self.reply(BAD_REQUEST, error_payload("model is required"))
            case str() if model == UNKNOWN_MODEL:
                self.reply(NOT_FOUND, error_payload("model does not exist"))
            case str() if model == BROKEN_MODEL:
                self.reply(BROKEN, error_payload("provider failed"))
            case str() if model == RATE_LIMITED_MODEL:
                self.reply(RATE_LIMITED, error_payload("rate limit reached"))
            case str() if body.get("stream") is True:
                self.stream(model)
            case str():
                self.reply(OK, completion_payload(model))
            case _:
                self.reply(400, error_payload("model must be a string"))

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


class Dumpable(Protocol):
    def model_dump(self) -> dict[str, JsonValue]: ...


class StreamOf(Protocol):
    def __iter__(self) -> Iterator[Dumpable]: ...


class AsyncStreamOf(Protocol):
    def __aiter__(self) -> AsyncIterator[Dumpable]: ...


@runtime_checkable
class ErrorFields(Protocol):
    message: str
    status_code: int
    llm_provider: str
    model: str


@runtime_checkable
class ErrorStatus(Protocol):
    status_code: int


class TokenCountView(Protocol):
    total_tokens: int
    request_model: str
    model_used: str
    tokenizer_type: str


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

CallArgument: TypeAlias = (
    JsonValue | Dumpable | BaseException | type[BaseModel] | Callable[..., object]
)


class LitellmCompletion(Protocol):
    def __call__(
        self, *args: CallArgument, **kwargs: CallArgument
    ) -> Dumpable | StreamOf: ...


class LitellmAsyncCompletion(Protocol):
    def __call__(
        self, *args: CallArgument, **kwargs: CallArgument
    ) -> Awaitable[Dumpable | StreamOf]: ...


class LitellmSyncRetry(Protocol):
    def __call__(
        self, *args: CallArgument, **kwargs: CallArgument
    ) -> Dumpable | StreamOf: ...


class LitellmAsyncRetry(Protocol):
    def __call__(
        self, *args: CallArgument, **kwargs: CallArgument
    ) -> Awaitable[Dumpable | StreamOf]: ...


class LitellmCreate(Protocol):
    def create(
        self, messages: list[dict[str, JsonValue]], model: str | None = None,
        **kwargs: JsonValue,
    ) -> object: ...


class LitellmCompletionsFactory(Protocol):
    def __call__(self, params: dict[str, JsonValue], router_obj: object) -> LitellmCreate: ...


class LitellmAsyncCreate(Protocol):
    def create(
        self, messages: list[dict[str, JsonValue]], model: str | None = None,
        **kwargs: JsonValue,
    ) -> Coroutine[object, object, object]: ...


class LitellmAsyncCompletionsFactory(Protocol):
    def __call__(
        self, params: dict[str, JsonValue], router_obj: object
    ) -> LitellmAsyncCreate: ...


class LitellmConfigCompletion(Protocol):
    def __call__(self, **kwargs: CallArgument) -> object: ...


class LitellmAdapterCompletion(Protocol):
    def __call__(self, *, adapter_id: str, **kwargs: CallArgument) -> object: ...


class LitellmAsyncAdapterCompletion(Protocol):
    def __call__(
        self, *, adapter_id: str, **kwargs: CallArgument
    ) -> Awaitable[object]: ...


@runtime_checkable
class LitellmModule(Protocol):
    completion: LitellmCompletion
    acompletion: LitellmAsyncCompletion
    completion_with_retries: LitellmSyncRetry
    acompletion_with_retries: LitellmAsyncRetry
    Completions: LitellmCompletionsFactory
    AsyncCompletions: LitellmAsyncCompletionsFactory
    ModelResponse: type[BaseModel]
    config_completion: LitellmConfigCompletion
    adapter_completion: LitellmAdapterCompletion
    aadapter_completion: LitellmAsyncAdapterCompletion


@runtime_checkable
class LitellmUtils(Protocol):
    def check_valid_key(self, model: str, api_key: str) -> bool: ...
    def valid_model(self, model: str) -> None: ...
    def acreate(
        self, *args: CallArgument, **kwargs: CallArgument
    ) -> Awaitable[Dumpable | StreamOf]: ...


@runtime_checkable
class LitellmMain(Protocol):
    def acount_tokens(
        self, model: str, messages: list[dict[str, JsonValue]], **kwargs: JsonValue
    ) -> Coroutine[object, object, TokenCountView]: ...


@runtime_checkable
class OpenaiModule(Protocol):
    APIError: type[BaseException]


def litellm_module() -> LitellmModule:
    # litellm is a runtime dependency of the check, not a typed one pyright can resolve
    module = importlib.import_module("litellm")
    assert isinstance(module, LitellmModule)
    return module


def litellm_utils() -> LitellmUtils:
    module = importlib.import_module("litellm.utils")
    assert isinstance(module, LitellmUtils)
    return module


def litellm_main() -> LitellmMain:
    module = importlib.import_module("litellm.main")
    assert isinstance(module, LitellmMain)
    return module


def openai_module() -> OpenaiModule:
    module = importlib.import_module("openai")
    assert isinstance(module, OpenaiModule)
    return module


def normalized(dump: dict[str, JsonValue]) -> dict[str, JsonValue]:
    return {name: value for name, value in dump.items() if name not in DROPPED}


def chunk_parts(
    dump: dict[str, JsonValue],
) -> tuple[int, str | None, str | None, str | None]:
    view = CHUNK_VIEW.validate_python(dump)
    choice = view["choices"][0]
    delta = choice["delta"]
    return (choice["index"], delta.get("content"), delta.get("role"), choice["finish_reason"])


def error_view(error: BaseException) -> tuple[str, str, int, str, str]:
    assert isinstance(error, ErrorFields)
    return (
        type(error).__name__, error.message, error.status_code, error.llm_provider, error.model,
    )


def error_kind(error: BaseException) -> tuple[str, int]:
    assert isinstance(error, ErrorStatus)
    return (type(error).__name__, error.status_code)


def header_of(call_record: RecordedCall, name: str) -> str | None:
    lowered = name.lower()
    for header, value in call_record["headers"].items():
        if header.lower() == lowered:
            return value
    return None


def raised(call: Callable[[], object], expected: type[BaseException]) -> BaseException:
    # the error itself is the comparison, so the test names no class for the reference side
    try:
        call()
    except expected as error:
        return error
    raise AssertionError("the call did not raise an error")


def model_response(
    outcome: CompletionOutcome | Awaitable[CompletionOutcome] | None,
) -> ModelResponse:
    match outcome:
        case ModelResponse():
            return outcome
        case CustomStreamWrapper():
            raise AssertionError("a plain call answered with a stream")
        case object():
            raise AssertionError("a plain call answered with an awaitable or nothing")


def stream_response(
    outcome: CompletionOutcome | Awaitable[CompletionOutcome] | None,
) -> CustomStreamWrapper:
    match outcome:
        case CustomStreamWrapper():
            return outcome
        case ModelResponse():
            raise AssertionError("a streaming call answered with a response")
        case object():
            raise AssertionError("a streaming call answered with an awaitable or nothing")


def litellm_plain(**kwargs: CallArgument) -> dict[str, JsonValue]:
    result = litellm_module().completion(**kwargs)
    assert isinstance(result, Dumpable)
    return result.model_dump()


async def litellm_async_plain(**kwargs: CallArgument) -> dict[str, JsonValue]:
    result = await litellm_module().acompletion(**kwargs)
    assert isinstance(result, Dumpable)
    return result.model_dump()


def litellm_stream_parts(**kwargs: CallArgument) -> StreamParts:
    result = litellm_module().completion(**kwargs)
    assert isinstance(result, StreamOf)
    return [chunk_parts(chunk.model_dump()) for chunk in result]


async def litellm_async_stream_parts(**kwargs: CallArgument) -> StreamParts:
    result = await litellm_module().acompletion(**kwargs)
    assert isinstance(result, AsyncStreamOf)
    parts: StreamParts = []
    async for chunk in result:
        parts.append(chunk_parts(chunk.model_dump()))
    return parts


def ours_stream_parts(stream: CustomStreamWrapper) -> StreamParts:
    return [chunk_parts(chunk.model_dump()) for chunk in stream]


async def ours_async_stream_parts(stream: CustomStreamWrapper) -> StreamParts:
    parts: StreamParts = []
    async for chunk in stream:
        parts.append(chunk_parts(chunk.model_dump()))
    return parts


# the loose view of a call the tests drive with Python objects: completion's own **kwargs are JSON
CompletionCall: TypeAlias = Callable[..., CompletionOutcome | Awaitable[CompletionOutcome]]
AsyncCompletionCall: TypeAlias = Callable[..., Awaitable[CompletionOutcome]]
COMPLETION: Final[CompletionCall] = completion
ACOMPLETION: Final[AsyncCompletionCall] = acompletion


async def await_outcome(awaitable: Awaitable[CompletionOutcome]) -> CompletionOutcome:
    return await awaitable
