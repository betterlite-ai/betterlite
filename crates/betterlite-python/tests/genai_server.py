import importlib
import json
import threading
from collections.abc import AsyncIterator, Awaitable, Iterator
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from types import TracebackType
from typing import ClassVar, Final, Protocol, Self, TypeAlias, TypedDict, runtime_checkable

from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

HOST: Final = "127.0.0.1"
STATUS_OK: Final = 200
STATUS_BAD_REQUEST: Final = 400
STATUS_UNAUTHORIZED: Final = 401
GOOD_KEY: Final = "test-key"
BAD_KEY: Final = "bad-key"
GENERIC_BAD_KEY: Final = "generic-bad-key"
MODEL: Final = "gemini-2.5-flash"
PROVIDER: Final = "gemini"
PREFIXED_MODEL: Final = f"{PROVIDER}/{MODEL}"
API_BASE_SUFFIX: Final = "/v1beta"
GENERATE_PATH: Final = f"{API_BASE_SUFFIX}/models/{MODEL}:generateContent"
STREAM_PATH: Final = f"{API_BASE_SUFFIX}/models/{MODEL}:streamGenerateContent?alt=sse"
CONTENTS: Final[list[JsonValue]] = [
    {"role": "user", "parts": [{"text": "Hi"}]},
]
TOOLS: Final[list[JsonValue]] = [
    {
        "functionDeclarations": [
            {
                "name": "get_weather",
                "description": "Weather for a city",
                "parameters": {"type": "OBJECT", "properties": {"city": {"type": "STRING"}}},
            }
        ]
    }
]
CONFIG: Final[dict[str, JsonValue]] = {"temperature": 1, "maxOutputTokens": 64}
SYSTEM_INSTRUCTION: Final[dict[str, JsonValue]] = {"parts": [{"text": "be brief"}]}
SAFETY_SETTINGS: Final[list[JsonValue]] = [
    {"category": "HARM_CATEGORY_HARASSMENT", "threshold": "BLOCK_NONE"}
]
EXTRA_BODY: Final[dict[str, JsonValue]] = {"labels": {"suite": "parity"}}
RESPONSE: Final[dict[str, JsonValue]] = {
    "candidates": [
        {
            "content": {"parts": [{"text": "Hello from the fake"}], "role": "model"},
            "finishReason": "STOP",
            "index": 0,
        }
    ],
    "usageMetadata": {"promptTokenCount": 3, "candidatesTokenCount": 4, "totalTokenCount": 7},
    "modelVersion": MODEL,
}
EVENTS: Final[list[dict[str, JsonValue]]] = [
    {"candidates": [{"content": {"parts": [{"text": "Hi"}], "role": "model"}, "index": 0}]},
    {
        "candidates": [
            {
                "content": {"parts": [{"text": "lo"}], "role": "model"},
                "finishReason": "STOP",
                "index": 0,
            }
        ],
        "usageMetadata": {"promptTokenCount": 3, "candidatesTokenCount": 2, "totalTokenCount": 5},
    },
]
ERROR_MESSAGE: Final = "API key not valid. Please pass a valid API key."
GENERIC_ERROR_MESSAGE: Final = "Invalid JSON payload received."
API_KEY_HEADER: Final = "x-goog-api-key"

CallArgument: TypeAlias = JsonValue
Request: TypeAlias = dict[str, JsonValue]
REQUEST: Final[TypeAdapter[Request]] = TypeAdapter(dict[str, JsonValue])


class RecordedCall(TypedDict):
    path: str
    headers: dict[str, str]
    body: Request


class GenaiServer(ThreadingHTTPServer):
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
        return f"http://{HOST}:{self.server_port}{API_BASE_SUFFIX}"

    def record(self, path: str, headers: dict[str, str], body: Request) -> None:
        recorded: RecordedCall = {"path": path, "headers": headers, "body": body}
        self.calls.append(recorded)

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
    owner: ClassVar[GenaiServer]

    def do_POST(self) -> None:
        declared = self.headers.get("Content-Length")
        raw = b"{}" if declared is None else self.rfile.read(int(declared))
        body = REQUEST.validate_json(raw)
        self.owner.record(self.path, dict(self.headers.items()), body)
        if self.headers.get(API_KEY_HEADER) == BAD_KEY:
            self.reply_error(STATUS_BAD_REQUEST, ERROR_MESSAGE)
            return
        if self.headers.get(API_KEY_HEADER) == GENERIC_BAD_KEY:
            self.reply_error(STATUS_BAD_REQUEST, GENERIC_ERROR_MESSAGE)
            return
        if self.path == STREAM_PATH:
            self.stream()
            return
        self.reply(STATUS_OK, RESPONSE)

    def reply_error(self, status: int, message: str) -> None:
        payload: dict[str, JsonValue] = {
            "error": {"code": status, "message": message, "status": "INVALID_ARGUMENT"},
        }
        self.reply(status, payload)

    def reply(self, status: int, payload: dict[str, JsonValue]) -> None:
        encoded = json.dumps(payload).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)

    def stream(self) -> None:
        self.send_response(STATUS_OK)
        self.send_header("Content-Type", "text/event-stream")
        self.end_headers()
        for event in EVENTS:
            self.wfile.write(f"data: {json.dumps(event)}\n\n".encode())
            self.wfile.flush()


@runtime_checkable
class Dumpable(Protocol):
    def model_dump(self) -> dict[str, JsonValue]: ...


@runtime_checkable
class ByteStream(Protocol):
    def __iter__(self) -> Iterator[bytes]: ...


@runtime_checkable
class AsyncByteStream(Protocol):
    def __aiter__(self) -> AsyncIterator[bytes]: ...


Answer: TypeAlias = Dumpable | ByteStream | AsyncByteStream | None


class ReferenceCall(Protocol):
    def __call__(self, *args: CallArgument, **kwargs: CallArgument) -> Answer: ...


class ReferenceAsyncCall(Protocol):
    def __call__(self, *args: CallArgument, **kwargs: CallArgument) -> Awaitable[Answer]: ...


class ReferenceSync(Protocol):
    def __getattr__(self, name: str) -> ReferenceCall: ...


class ReferenceAsync(Protocol):
    def __getattr__(self, name: str) -> ReferenceAsyncCall: ...


class ReferenceErrors(Protocol):
    def __getattr__(self, name: str) -> type[BaseException]: ...


LITELLM: ReferenceSync = importlib.import_module("litellm.google_genai")
LITELLM_ASYNC: ReferenceAsync = importlib.import_module("litellm.google_genai")
LITELLM_ERRORS: ReferenceErrors = importlib.import_module("litellm.exceptions")


def reference_dict(result: Answer) -> dict[str, JsonValue]:
    match result:
        case Dumpable():
            return result.model_dump()
        case ByteStream() | AsyncByteStream() | None:
            raise AssertionError("the reference call answered no model response")


def reference_stream(result: Answer) -> ByteStream:
    match result:
        case ByteStream():
            return result
        case Dumpable() | AsyncByteStream() | None:
            raise AssertionError("the reference call answered no sync iterator")


async def reference_stream_async(result: Awaitable[Answer]) -> AsyncByteStream:
    match await result:
        case AsyncByteStream() as stream:
            return stream
        case Dumpable() | ByteStream() | None:
            raise AssertionError("the reference call answered no async iterator")


def recorded_header(server: GenaiServer, index: int, name: str) -> str | None:
    lowered = name.lower()
    for header, value in server.calls[index]["headers"].items():
        if header.lower() == lowered:
            return value
    return None
