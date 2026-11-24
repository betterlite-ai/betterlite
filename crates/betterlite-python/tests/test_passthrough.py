import asyncio
import importlib
import subprocess
import sys
import threading
from collections.abc import AsyncIterator, Generator, Iterator, Mapping
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from types import TracebackType
from typing import ClassVar, Final, Protocol, Self, TypeAlias, TypeVar, runtime_checkable

import httpx
import pytest
from pydantic import JsonValue, TypeAdapter
from typing_extensions import TypedDict, override

import betterlite.passthrough
from betterlite._completion.arguments import SessionLike
from betterlite._completion.errors import UnsupportedCallArgument
from betterlite._endpoints.payload import CallValue
from betterlite._native import NativeError
from betterlite.exceptions import APIError
from betterlite.passthrough.main import (
    AsyncPassthroughStreamingResponse,
    AsyncStreamReply,
    PassthroughResponse,
    PassthroughStreamingResponse,
    StreamReply,
    allm_passthrough_route,
    llm_passthrough_route,
)
from betterlite.passthrough.timeout_utils import (
    resolve_llm_passthrough_timeout,
    resolve_pass_through_request_timeout,
)
from betterlite.passthrough.utils import (
    BasePassthroughUtils,
    CommonUtils,
    Content,
    Cookies,
    Files,
    Params,
)

HOST: Final = "127.0.0.1"
OK: Final = 200
UNAUTHORIZED: Final = 401
SERVER_ERROR: Final = 500
MODEL: Final = "my-model"
PROVIDER: Final = "vllm"
OPENAI_MODEL: Final = "gpt-4o"
OPENAI_PROVIDER: Final = "openai"
API_PATH: Final = "/v1"
CHAT_ENDPOINT: Final = "chat/completions"
CHAT_PATH: Final = f"{API_PATH}/{CHAT_ENDPOINT}"
AUTH_ENDPOINT: Final = "unauthorized"
INPUT: Final = "hi"
INPUT_BODY: Final[dict[str, JsonValue]] = {"model": MODEL, "input": INPUT}
STREAM_BODY: Final[dict[str, JsonValue]] = {"stream": True, "input": INPUT}
ANSWER: Final = b'{"ok": true, "id": "chatcmpl-1"}'
ANSWER_BODY: Final[dict[str, JsonValue]] = {"ok": True, "id": "chatcmpl-1"}
ERROR_TEXT: Final = '{"error": {"message": "Invalid API key", "type": "invalid_request_error"}}'
ERROR_BODY: Final[dict[str, JsonValue]] = {
    "error": {"message": "Invalid API key", "type": "invalid_request_error"},
}
FRAMES: Final = (b'data: {"n": 1}\n\n', b'data: {"n": 2}\n\n', b"data: [DONE]\n\n")
STREAM_BYTES: Final = b"".join(FRAMES)
JSON_TYPE: Final = "application/json"
EVENT_TYPE: Final = "text/event-stream"
FORM_TYPE: Final = "application/x-www-form-urlencoded"
MULTIPART_TYPE: Final = "multipart/form-data"
OCTET_TYPE: Final = "application/octet-stream"
HEADER_NAME: Final = "X-Upstream"
HEADER_VALUE: Final = "yes"
TRUNCATED_ENDPOINT: Final = "truncated"
TRUNCATED_FRAME: Final = b"data: " + b"x" * 65536 + b"\n\n"
TRUNCATED_LENGTH: Final = len(TRUNCATED_FRAME) + 1
CRLF: Final = b"\r\n"
CRLF_LENGTH: Final = len(CRLF)
# the request headers every route comparison reads; the clients add their own volatile ones
COMPARED_REQUEST_HEADERS: Final[frozenset[str]] = frozenset(
    {"authorization", "cookie", "x-api-key", "anthropic-beta", "x-request"}
)
VOLATILE: Final[frozenset[str]] = frozenset(
    {"date", "server", "connection", "keep-alive", "transfer-encoding"}
)
DEAD_BASE: Final = f"http://{HOST}:9{API_PATH}"
POSITIONAL: Final = "POST"
EXPECTED_CALLS: Final = 2
EXPECTED_EXPORTS: Final[list[str]] = [
    "BasePassthroughUtils",
    "allm_passthrough_route",
    "llm_passthrough_route",
]
TIMEOUT_SECONDS: Final = 12.5
ROUTER_SECONDS: Final = 40
STREAM_TIMEOUT: Final = 5
GENERIC_TIMEOUT: Final = 9
STRING_TIMEOUT: Final = "30"
ARN_MODEL: Final = "arn:aws:bedrock:us-east-1:123456789012:application-inference-profile/abc123"
ARN_ENDPOINT: Final = f"/model/{ARN_MODEL}/invoke"
ARN_ENCODED: Final = f"/model/{ARN_MODEL.replace('/', '%2F', 1)}/invoke"
REFERENCE_ERROR_TEXT: Final = "Invalid API key"
PROVIDER_ERROR_TEXT: Final = "Provider openai not found"
FORWARD_REQUEST: Final[dict[str, str]] = {
    "host": "upstream.test",
    "content-length": "5",
    "accept-encoding": "br",
    "x-request": "1",
    "X-Custom": "old",
    "x-pass-anthropic-beta": "beta-1",
}
CUSTOM_HEADERS: Final[dict[str, str]] = {"x-custom": "new"}
PASS_REQUEST: Final[dict[str, str]] = {
    "x-pass-anthropic-beta": "beta-1",
    "x-pass-Authorization": "secret",
    "x-pass-x-amz-date": "2026",
    "x-request": "1",
}
NO_HEADERS: Final[dict[str, str]] = {}
MERGE_QUERY: Final[dict[str, str | list[str]]] = {"shared": "request", "tags": ["a", "b"]}
MERGE_DEFAULTS: Final[dict[str, str | list[str]]] = {
    "shared": "default",
    "from_default": "default",
    "from_url": "default",
}
# the signature comparison runs in its own interpreter, so no untyped parameter value reaches mypy
PROBE: Final = """
import importlib
import inspect
import json


def rows(function):
    return [
        [p.name, p.kind.name, "" if p.default is inspect.Parameter.empty else repr(p.default)]
        for p in inspect.signature(function).parameters.values()
    ]


def pairs():
    found = {}
    for name, group, owner in (
        ("llm_passthrough_route", "main", None),
        ("allm_passthrough_route", "main", None),
        ("get_merged_query_parameters", "utils", "BasePassthroughUtils"),
        ("forward_headers_from_request", "utils", "BasePassthroughUtils"),
        ("encode_bedrock_runtime_modelid_arn", "utils", "CommonUtils"),
        ("resolve_pass_through_request_timeout", "timeout_utils", None),
        ("resolve_llm_passthrough_timeout", "timeout_utils", None),
    ):
        ours = importlib.import_module(f"betterlite.passthrough.{group}")
        theirs = importlib.import_module(f"litellm.passthrough.{group}")
        if owner is not None:
            ours, theirs = getattr(ours, owner), getattr(theirs, owner)
        found[name] = (getattr(ours, name), getattr(theirs, name))
    return found


table = pairs()
print(json.dumps({
    "ours": {name: rows(ours) for name, (ours, _) in table.items()},
    "reference": {name: rows(theirs) for name, (_, theirs) in table.items()},
}))
"""

BODY_VIEW: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])
QUERY_VIEW: Final[TypeAdapter[dict[str, str | list[str]]]] = TypeAdapter(
    dict[str, str | list[str]]
)
HEADER_VIEW: Final[TypeAdapter[dict[str, str]]] = TypeAdapter(dict[str, str])


class RecordedCall(TypedDict):
    path: str
    body: bytes
    headers: dict[str, str]


class PartView(TypedDict):
    headers: dict[str, str]
    body: bytes


class RouteShape(TypedDict, total=False):
    content: Content | None
    data: dict[str, CallValue] | None
    files: Files | None
    params: Params | None
    cookies: Cookies | None
    request_query_params: dict[str, JsonValue] | None
    request_headers: dict[str, str] | None
    forward_headers: bool


class CallShape(TypedDict):
    method: str
    endpoint: str
    model: str
    custom_llm_provider: str
    api_base: str
    json: dict[str, JsonValue]


class TimeoutCase(TypedDict, total=False):
    kwargs: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue]
    router_timeout: float
    router_stream_timeout: float


class SignatureView(TypedDict):
    ours: dict[str, list[tuple[str, str, str]]]
    reference: dict[str, list[tuple[str, str, str]]]


SIGNATURE_VIEW: Final[TypeAdapter[SignatureView]] = TypeAdapter(SignatureView)


# LiteLLM's own objects come back through this empty protocol, which every type satisfies
class CallResult(Protocol):
    ...


Answer: TypeAlias = tuple[int, dict[str, str], bytes]
CallArgument: TypeAlias = JsonValue | SessionLike
ErrorType = TypeVar("ErrorType", bound=Exception)


@runtime_checkable
class AnswerView(Protocol):
    status_code: int
    headers: Mapping[str, str]
    content: bytes


@runtime_checkable
class StreamView(Protocol):
    status_code: int
    headers: Mapping[str, str]

    def __iter__(self) -> Iterator[bytes]: ...


@runtime_checkable
class AsyncStreamView(Protocol):
    status_code: int
    headers: Mapping[str, str]

    def __aiter__(self) -> AsyncIterator[bytes]: ...


@runtime_checkable
class ErrorView(Protocol):
    status_code: int


@runtime_checkable
class PendingAnswer(Protocol):
    def __await__(self) -> Generator[None, None, CallResult]: ...


class ReferenceCall(Protocol):
    def __call__(self, *args: CallArgument, **kwargs: CallArgument) -> CallResult: ...

    def __getattr__(self, name: str) -> "ReferenceCall": ...


class ReferenceModule(Protocol):
    def __getattr__(self, name: str) -> ReferenceCall: ...


OUR_MAIN: ReferenceModule = importlib.import_module("betterlite.passthrough.main")
REFERENCES: ReferenceModule = importlib.import_module("litellm.passthrough.main")
REFERENCE_UTILS: ReferenceModule = importlib.import_module("litellm.passthrough.utils")
REFERENCE_TIMEOUTS: ReferenceModule = importlib.import_module("litellm.passthrough.timeout_utils")


class PassthroughServer(ThreadingHTTPServer):
    daemon_threads = True
    calls: list[RecordedCall]

    def __init__(self) -> None:
        super().__init__((HOST, 0), _Handler)
        self.calls = []
        _Handler.owner = self
        self.thread = threading.Thread(target=self.serve_forever, daemon=True)
        self.thread.start()

    @property
    def base(self) -> str:
        return f"http://{HOST}:{self.server_port}{API_PATH}"

    def record(self, path: str, body: bytes, headers: Mapping[str, str]) -> None:
        self.calls.append(
            {
                "path": path,
                "body": body,
                "headers": {name.lower(): value for name, value in headers.items()},
            }
        )

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
    owner: ClassVar[PassthroughServer]

    def do_POST(self) -> None:
        body = self.read_body()
        self.owner.record(self.path, body, dict(self.headers.items()))
        if self.path.endswith(TRUNCATED_ENDPOINT):
            self.truncate()
            return
        if self.path.endswith(AUTH_ENDPOINT):
            self.reply(UNAUTHORIZED, ERROR_TEXT.encode(), JSON_TYPE)
            return
        if self.streams(body):
            self.stream()
            return
        self.reply(OK, ANSWER, JSON_TYPE)

    def read_body(self) -> bytes:
        encoding = self.headers.get("Transfer-Encoding")
        if encoding is not None and encoding.lower() == "chunked":
            return self.read_chunked()
        declared = self.headers.get("Content-Length")
        return self.rfile.read(0 if declared is None else int(declared))

    def read_chunked(self) -> bytes:
        pieces: list[bytes] = []
        while True:
            size = int(self.rfile.readline().split(b";")[0].strip(), 16)
            if not size:
                self.rfile.readline()
                return b"".join(pieces)
            pieces.append(self.rfile.read(size))
            self.rfile.read(CRLF_LENGTH)

    def streams(self, body: bytes) -> bool:
        if not body:
            return False
        try:
            parsed = BODY_VIEW.validate_json(body)
        except ValueError:
            return False
        return parsed.get("stream") is True

    def reply(self, status: int, payload: bytes, content_type: str) -> None:
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header(HEADER_NAME, HEADER_VALUE)
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def stream(self) -> None:
        self.send_response(OK)
        self.send_header("Content-Type", EVENT_TYPE)
        self.send_header(HEADER_NAME, HEADER_VALUE)
        self.send_header("Connection", "close")
        self.end_headers()
        for frame in FRAMES:
            self.wfile.write(frame)
            self.wfile.flush()
        self.close_connection = True

    def truncate(self) -> None:
        self.send_response(OK)
        self.send_header("Content-Type", EVENT_TYPE)
        self.send_header("Content-Length", str(TRUNCATED_LENGTH))
        self.end_headers()
        self.wfile.write(TRUNCATED_FRAME)
        self.wfile.flush()
        self.close_connection = True


@pytest.fixture(autouse=False)
def server() -> Iterator[PassthroughServer]:
    with PassthroughServer() as running:
        yield running


def compared_headers(headers: Mapping[str, str]) -> dict[str, str]:
    return {
        name.lower(): value for name, value in headers.items() if name.lower() not in VOLATILE
    }


def expected_headers(content_type: str, length: int | None = None) -> dict[str, str]:
    headers: dict[str, str] = {"content-type": content_type, "x-upstream": HEADER_VALUE}
    if length is not None:
        headers["content-length"] = str(length)
    return headers


def recorded_body(record: RecordedCall) -> dict[str, JsonValue]:
    text = record["body"].decode()
    return BODY_VIEW.validate_json(text)


def signature_view() -> SignatureView:
    done = subprocess.run(
        [sys.executable, "-c", PROBE], capture_output=True, text=True, check=False
    )
    assert done.returncode == 0, done.stderr
    return SIGNATURE_VIEW.validate_json(done.stdout)


def call_for(
    body: dict[str, JsonValue],
    base: str,
    endpoint: str = CHAT_ENDPOINT,
    model: str = MODEL,
    provider: str = PROVIDER,
) -> CallShape:
    return CallShape(
        method="POST",
        endpoint=endpoint,
        model=model,
        custom_llm_provider=provider,
        api_base=base,
        json=dict(body),
    )


def answer_parts(answer: StreamReply | AnswerView) -> Answer:
    match answer:
        case AnswerView():
            return answer.status_code, compared_headers(answer.headers), answer.content
        case object():
            raise AssertionError("the call answered with a stream")


def stream_parts(answer: StreamReply | StreamView) -> Answer:
    match answer:
        case StreamView():
            return answer.status_code, compared_headers(answer.headers), b"".join(answer)
        case object():
            raise AssertionError("the call answered with a full response")


def reference_parts(answer: CallResult) -> Answer:
    match answer:
        case AnswerView():
            return answer.status_code, compared_headers(answer.headers), answer.content
        case object():
            raise AssertionError("the reference call answered with no response")


def reference_stream_parts(answer: CallResult) -> Answer:
    match answer:
        case StreamView():
            return answer.status_code, compared_headers(answer.headers), b"".join(answer)
        case object():
            raise AssertionError("the reference call answered with no stream")


def status_of(error: Exception | ErrorView) -> int:
    match error:
        case ErrorView():
            return error.status_code
        case object():
            raise AssertionError("the reference error carries no status")


def float_of(value: CallResult) -> float:
    match value:
        case float() as seconds:
            return seconds
        case object():
            raise AssertionError("the reference helper answered with no float")


def str_of(value: CallResult) -> str:
    match value:
        case str() as text:
            return text
        case object():
            raise AssertionError("the reference helper answered with no string")


async def awaited(answer: CallResult) -> CallResult:
    match answer:
        case PendingAnswer():
            return await answer
        case object():
            raise AssertionError("the reference call answered with no awaitable")


async def async_answer(answer: AsyncStreamReply | AnswerView) -> Answer:
    match answer:
        case AnswerView():
            return answer.status_code, compared_headers(answer.headers), answer.content
        case object():
            raise AssertionError("the async call answered with a stream")


async def async_stream(answer: AsyncStreamReply) -> Answer:
    stream = astream_of(answer)
    returned = stream.aiter_bytes()
    assert returned is stream
    chunks = [chunk async for chunk in returned]
    return stream.status_code, compared_headers(stream.headers), b"".join(chunks)


async def reference_async_stream(answer: CallResult) -> Answer:
    match answer:
        case AsyncStreamView():
            chunks = [chunk async for chunk in answer]
            return answer.status_code, compared_headers(answer.headers), b"".join(chunks)
        case object():
            raise AssertionError("the reference call answered with no stream")


async def ours_async(base: str) -> tuple[Answer, Answer]:
    full = await allm_passthrough_route(**call_for(INPUT_BODY, base))
    streamed = await allm_passthrough_route(**call_for(STREAM_BODY, base))
    return await async_answer(full), await async_stream(streamed)


async def reference_async(base: str) -> tuple[Answer, Answer]:
    full = await awaited(REFERENCES.allm_passthrough_route(**call_for(INPUT_BODY, base)))
    streamed = await awaited(REFERENCES.allm_passthrough_route(**call_for(STREAM_BODY, base)))
    return reference_parts(full), await reference_async_stream(streamed)


async def reference_positional() -> None:
    with pytest.raises(TypeError):
        await awaited(REFERENCES.allm_passthrough_route(POSITIONAL))


def stream_of(answer: StreamReply) -> PassthroughStreamingResponse:
    match answer:
        case PassthroughStreamingResponse() as stream:
            return stream
        case object():
            raise AssertionError("the truncated call answered with a full response")


# ast-grep-ignore: no-long-names
def astream_of(answer: AsyncStreamReply) -> AsyncPassthroughStreamingResponse:
    match answer:
        # ast-grep-ignore: no-long-names
        case AsyncPassthroughStreamingResponse() as stream:
            return stream
        case object():
            raise AssertionError("the truncated call answered with a full response")


def drain_chunks(stream: PassthroughStreamingResponse) -> None:
    for _chunk in stream:
        pass


# ast-grep-ignore: no-long-names
async def drain_chunks_async(stream: AsyncPassthroughStreamingResponse) -> None:
    async for _chunk in stream:
        pass


def chunks_then_failure(stream: PassthroughStreamingResponse) -> tuple[bytes, APIError]:
    first = next(stream)
    with pytest.raises(APIError) as failure:
        drain_chunks(stream)
    with pytest.raises(StopIteration):
        next(stream)
    return first, failure.value


# ast-grep-ignore: no-long-names
async def achunks_then_failure(stream: AsyncPassthroughStreamingResponse) -> tuple[bytes, APIError]:
    first = await anext(stream)
    with pytest.raises(APIError) as failure:
        await drain_chunks_async(stream)
    with pytest.raises(StopAsyncIteration):
        await anext(stream)
    return first, failure.value


async def atruncated_stream(server: PassthroughServer) -> tuple[bytes, int]:
    arguments = call_for(STREAM_BODY, server.base, endpoint=TRUNCATED_ENDPOINT)
    asked = await allm_passthrough_route(**arguments)
    first, failure = await achunks_then_failure(astream_of(asked))
    return first, failure.status_code


def passthrough_response(answer: StreamReply) -> PassthroughResponse:
    match answer:
        case PassthroughResponse() as response:
            return response
        case object():
            raise AssertionError("the call answered with a stream")


def native_cause(error: BaseException) -> None:
    match error.__cause__:
        case NativeError():
            pass
        case object():
            raise AssertionError("the raised error carries no native cause")


def both_raise(
    asked: CallShape, error: type[ErrorType], match: str
) -> tuple[ErrorType, Exception]:
    with pytest.raises(error, match=match) as ours_error:
        llm_passthrough_route(**asked)
    with pytest.raises(Exception, match=match) as reference_error:
        REFERENCES.llm_passthrough_route(**asked)
    return ours_error.value, reference_error.value


def compared_request_headers(record: RecordedCall) -> dict[str, str]:
    return {
        name: value
        for name, value in record["headers"].items()
        if name in COMPARED_REQUEST_HEADERS
    }


def part_headers(text: str) -> dict[str, str]:
    headers: dict[str, str] = {}
    for line in text.split("\r\n"):
        name, _, value = line.partition(":")
        headers[name.strip().lower()] = value.strip()
    return headers


def multipart_parts(record: RecordedCall) -> list[PartView]:
    content_type = record["headers"]["content-type"]
    boundary = content_type.partition("boundary=")[2]
    assert boundary, content_type
    parts: list[PartView] = []
    for chunk in record["body"].split(f"--{boundary}".encode())[1:-1]:
        head, _, body = chunk.partition(b"\r\n\r\n")
        parts.append(
            {
                "headers": part_headers(head.decode().removeprefix("\r\n")),
                "body": body.removesuffix(CRLF),
            }
        )
    return parts


def round_trip(
    server: PassthroughServer, shape: RouteShape, base: str | None = None
) -> tuple[RecordedCall, RecordedCall]:
    address = server.base if base is None else base
    ours = llm_passthrough_route(
        method="POST",
        endpoint=CHAT_ENDPOINT,
        model=MODEL,
        custom_llm_provider=PROVIDER,
        api_base=address,
        **shape,
    )
    reference = REFERENCES.llm_passthrough_route(
        method="POST",
        endpoint=CHAT_ENDPOINT,
        model=MODEL,
        custom_llm_provider=PROVIDER,
        api_base=address,
        **shape,
    )
    assert answer_parts(ours) == reference_parts(reference)
    ours_call, reference_call = server.calls[-2:]
    assert ours_call["path"] == reference_call["path"]
    # a multipart body carries its own random boundary, so its parts are compared instead
    if shape.get("files") is None:
        assert ours_call["body"] == reference_call["body"]
    assert compared_request_headers(ours_call) == compared_request_headers(reference_call)
    return ours_call, reference_call


def test_signature_matches() -> None:
    view = signature_view()
    for name, rows in view["reference"].items():
        assert view["ours"][name] == rows, name


def test_signature_exports_match() -> None:
    exports = betterlite.passthrough.__all__
    assert exports == EXPECTED_EXPORTS


def test_signature_refusal() -> None:
    routes = (OUR_MAIN.llm_passthrough_route, REFERENCES.llm_passthrough_route)
    for route in routes:
        with pytest.raises(TypeError):
            route(POSITIONAL)
        with pytest.raises(TypeError):
            route(method=POSITIONAL)


def test_signature_refusal_async() -> None:
    with pytest.raises(TypeError):
        OUR_MAIN.allm_passthrough_route(POSITIONAL)
    asyncio.run(reference_positional())


def test_calls_match(server: PassthroughServer) -> None:
    ours = llm_passthrough_route(**call_for(INPUT_BODY, server.base))
    reference = REFERENCES.llm_passthrough_route(**call_for(INPUT_BODY, server.base))
    assert answer_parts(ours) == reference_parts(reference)
    assert answer_parts(ours) == (OK, expected_headers(JSON_TYPE, len(ANSWER)), ANSWER)
    assert [record["path"] for record in server.calls] == [CHAT_PATH] * EXPECTED_CALLS
    assert [recorded_body(record) for record in server.calls] == [INPUT_BODY] * EXPECTED_CALLS
    response = passthrough_response(ours)
    assert (response.json(), response.text) == (ANSWER_BODY, ANSWER.decode())


def test_calls_raise_for_status() -> None:
    with pytest.raises(APIError) as failure:
        PassthroughResponse(UNAUTHORIZED, {}, ERROR_TEXT.encode()).raise_for_status()
    expected = (UNAUTHORIZED, f"litellm.APIError: {ERROR_TEXT}")
    assert (failure.value.status_code, failure.value.message) == expected
    PassthroughResponse(OK, {}, ANSWER).raise_for_status()


def test_calls_stream_match(server: PassthroughServer) -> None:
    ours = stream_parts(llm_passthrough_route(**call_for(STREAM_BODY, server.base)))
    reference = REFERENCES.llm_passthrough_route(**call_for(STREAM_BODY, server.base))
    assert ours == reference_stream_parts(reference)
    assert ours == (OK, expected_headers(EVENT_TYPE), STREAM_BYTES)


def test_calls_async_match(server: PassthroughServer) -> None:
    ours = asyncio.run(ours_async(server.base))
    expected = (
        (OK, expected_headers(JSON_TYPE, len(ANSWER)), ANSWER),
        (OK, expected_headers(EVENT_TYPE), STREAM_BYTES),
    )
    assert ours == expected == asyncio.run(reference_async(server.base))


def test_calls_content_match(server: PassthroughServer) -> None:
    text = round_trip(server, RouteShape(content="hi"))
    raw = round_trip(server, RouteShape(content=b"raw"))
    chunks = round_trip(server, RouteShape(content=[b"a", b"b"]))
    calls = (text, raw, chunks)
    assert [call[0]["body"] for call in calls] == [b"hi", b"raw", b"ab"]
    for ours, reference in calls:
        assert ours["headers"]["content-type"] == OCTET_TYPE
        assert "content-type" not in reference["headers"]
    # the iterable is buffered, so betterlite frames it with Content-Length where httpx chunks it
    iterable_ours, iterable_reference = chunks
    assert iterable_ours["headers"]["content-length"] == "2"
    assert "transfer-encoding" not in iterable_ours["headers"]
    assert iterable_reference["headers"]["transfer-encoding"] == "chunked"


def test_calls_form_match(server: PassthroughServer) -> None:
    ours, reference = round_trip(server, RouteShape(data={"field": "value", "repeat": ["1", "2"]}))
    assert ours["body"] == reference["body"] == b"field=value&repeat=1&repeat=2"
    assert (ours["headers"]["content-type"], reference["headers"]["content-type"]) == (
        FORM_TYPE,
        FORM_TYPE,
    )


def test_calls_params_match(server: PassthroughServer) -> None:
    mapping = round_trip(server, RouteShape(params={"q": "1", "tag": ["a", "b"]}))
    text = round_trip(server, RouteShape(params="a=1&b=2"))
    sequence = round_trip(server, RouteShape(params=[("k", "v"), ("k2", "v2")]))
    assert [call[0]["path"] for call in (mapping, text, sequence)] == [
        f"{CHAT_PATH}?q=1&tag=a&tag=b",
        f"{CHAT_PATH}?a=1&b=2",
        f"{CHAT_PATH}?k=v&k2=v2",
    ]


def test_calls_base_query_match(server: PassthroughServer) -> None:
    plain = round_trip(server, RouteShape(request_query_params={"req": "1"}))
    asked = round_trip(
        server, RouteShape(request_query_params={"req": "1"}), base=f"{server.base}?from_base=1"
    )
    # format_url replaces the base query, so the endpoint text never leaves the path
    assert (plain[0]["path"], asked[0]["path"]) == (f"{CHAT_PATH}?req=1", f"{API_PATH}?req=1")


def test_calls_cookies_match(server: PassthroughServer) -> None:
    ours, reference = round_trip(server, RouteShape(cookies={"session": "abc"}))
    assert ours["headers"]["cookie"] == reference["headers"]["cookie"] == "session=abc"


def test_calls_files_match(server: PassthroughServer) -> None:
    image = ("cat.png", b"png-bytes", "image/png")
    mapping = round_trip(server, RouteShape(files={"image": image}))
    first = ("one.png", b"one", "image/png")
    second = ("two.png", b"two", "image/png")
    repeated = round_trip(server, RouteShape(files=[("image", first), ("image", second)]))
    assert multipart_parts(mapping[0]) == multipart_parts(mapping[1])
    assert multipart_parts(repeated[0]) == multipart_parts(repeated[1])
    assert [part["body"] for part in multipart_parts(repeated[0])] == [b"one", b"two"]
    assert [part["headers"]["content-disposition"] for part in multipart_parts(mapping[0])] == [
        'form-data; name="image"; filename="cat.png"'
    ]
    for call in (mapping[0], mapping[1]):
        assert call["headers"]["content-type"].startswith(MULTIPART_TYPE)


# the route hands forward_headers=False to the policy, so the flag changes nothing
def test_calls_headers_match(server: PassthroughServer) -> None:
    headers = {"x-pass-anthropic-beta": "beta-1", "x-request": "1"}
    quiet = round_trip(server, RouteShape(request_headers=dict(headers)))
    asked = round_trip(server, RouteShape(request_headers=dict(headers), forward_headers=True))
    for ours in (quiet[0], asked[0]):
        assert ours["headers"]["anthropic-beta"] == "beta-1"
        assert "x-request" not in ours["headers"]


def test_calls_helpers_match() -> None:
    url = httpx.URL(f"http://{HOST}:9{API_PATH}?from_url=url&shared=url&blank=")
    ours = BasePassthroughUtils.get_merged_query_parameters(url, MERGE_QUERY, MERGE_DEFAULTS)
    reference = REFERENCE_UTILS.BasePassthroughUtils.get_merged_query_parameters(
        existing_url=url, request_query_params=MERGE_QUERY, default_query_params=MERGE_DEFAULTS
    )
    assert ours == QUERY_VIEW.validate_python(reference)
    relative_url = httpx.URL("/v1/chat?from_url=url")
    relative = BasePassthroughUtils.get_merged_query_parameters(
        relative_url, MERGE_QUERY, MERGE_DEFAULTS
    )
    relative_reference = REFERENCE_UTILS.BasePassthroughUtils.get_merged_query_parameters(
        existing_url=relative_url,
        request_query_params=MERGE_QUERY,
        default_query_params=MERGE_DEFAULTS,
    )
    assert relative == QUERY_VIEW.validate_python(relative_reference)
    forwarded = BasePassthroughUtils.forward_headers_from_request(
        dict(FORWARD_REQUEST), dict(CUSTOM_HEADERS), forward_headers=True
    )
    forwarded_reference = REFERENCE_UTILS.BasePassthroughUtils.forward_headers_from_request(
        request_headers=dict(FORWARD_REQUEST),
        headers=dict(CUSTOM_HEADERS),
        forward_headers=True,
    )
    assert forwarded == HEADER_VIEW.validate_python(forwarded_reference)
    passed = BasePassthroughUtils.forward_headers_from_request(
        dict(PASS_REQUEST), dict(NO_HEADERS), forward_headers=False
    )
    passed_reference = REFERENCE_UTILS.BasePassthroughUtils.forward_headers_from_request(
        request_headers=dict(PASS_REQUEST),
        headers=dict(NO_HEADERS),
        forward_headers=False,
    )
    assert passed == HEADER_VIEW.validate_python(passed_reference)
    arn = CommonUtils.encode_bedrock_runtime_modelid_arn(ARN_ENDPOINT)
    arn_reference = REFERENCE_UTILS.CommonUtils.encode_bedrock_runtime_modelid_arn(
        endpoint=ARN_ENDPOINT
    )
    assert (arn, str_of(arn_reference)) == (ARN_ENCODED, ARN_ENCODED)
    with pytest.raises(ValueError, match="not a URL"):
        BasePassthroughUtils.get_merged_query_parameters("http://", MERGE_QUERY, MERGE_DEFAULTS)


def test_calls_timeout_matches() -> None:
    asked = REFERENCE_TIMEOUTS.resolve_pass_through_request_timeout(
        endpoint_timeout=TIMEOUT_SECONDS
    )
    default = REFERENCE_TIMEOUTS.resolve_pass_through_request_timeout()
    assert resolve_pass_through_request_timeout(TIMEOUT_SECONDS) == float_of(asked)
    assert resolve_pass_through_request_timeout() == float_of(default)


TIMEOUT_CASES: Final[tuple[TimeoutCase, ...]] = (
    {},
    {"kwargs": {"timeout": TIMEOUT_SECONDS}},
    {"kwargs": {"stream": True, "stream_timeout": STREAM_TIMEOUT, "timeout": GENERIC_TIMEOUT}},
    {"router_timeout": ROUTER_SECONDS},
    {"kwargs": {"timeout": STRING_TIMEOUT}},
    {"litellm_params": {"timeout": TIMEOUT_SECONDS}},
    {"litellm_params": {"request_timeout": STRING_TIMEOUT}},
    {"kwargs": {"stream": True}, "litellm_params": {"stream_timeout": STREAM_TIMEOUT}},
    {
        "kwargs": {"stream": True, "timeout": GENERIC_TIMEOUT},
        "router_stream_timeout": ROUTER_SECONDS,
    },
    {"kwargs": {"request_timeout": TIMEOUT_SECONDS}},
    {"kwargs": {"timeout": GENERIC_TIMEOUT}, "litellm_params": {"timeout": "later"}},
)


@pytest.mark.parametrize("case", TIMEOUT_CASES)
def test_calls_timeouts_match(case: TimeoutCase) -> None:
    reference = REFERENCE_TIMEOUTS.resolve_llm_passthrough_timeout(**case)
    assert resolve_llm_passthrough_timeout(**case) == float_of(reference)


def test_calls_timeouts_refuse() -> None:
    with pytest.raises(ValueError, match="soon"):
        resolve_llm_passthrough_timeout(kwargs={"timeout": "soon"})
    with pytest.raises(ValueError, match="soon"):
        REFERENCE_TIMEOUTS.resolve_llm_passthrough_timeout(kwargs={"timeout": "soon"})


def test_error_upstream(server: PassthroughServer) -> None:
    asked = call_for(INPUT_BODY, server.base, endpoint=AUTH_ENDPOINT)
    ours, reference = both_raise(asked, APIError, REFERENCE_ERROR_TEXT)
    expected = (UNAUTHORIZED, f"litellm.APIError: {ERROR_TEXT}", ERROR_BODY)
    assert (ours.status_code, ours.message, ours.body) == expected
    native_cause(ours)
    assert ERROR_TEXT in str(reference)
    assert status_of(reference) == UNAUTHORIZED


def test_error_stream_truncated(server: PassthroughServer) -> None:
    asked = llm_passthrough_route(**call_for(STREAM_BODY, server.base, TRUNCATED_ENDPOINT))
    first, failure = chunks_then_failure(stream_of(asked))
    assert first
    assert TRUNCATED_FRAME.startswith(first)
    assert failure.status_code == SERVER_ERROR


def test_error_truncated_async(server: PassthroughServer) -> None:
    first, status = asyncio.run(atruncated_stream(server))
    assert first
    assert TRUNCATED_FRAME.startswith(first)
    assert status == SERVER_ERROR


def test_error_provider_missing() -> None:
    asked = call_for(INPUT_BODY, DEAD_BASE, model=OPENAI_MODEL, provider=OPENAI_PROVIDER)
    ours, reference = both_raise(asked, Exception, PROVIDER_ERROR_TEXT)
    assert (type(ours).__name__, str(ours)) == ("Exception", PROVIDER_ERROR_TEXT)
    assert type(reference).__name__ == "Exception"
    native_cause(ours)


def test_unsupported_argument() -> None:
    with pytest.raises(UnsupportedCallArgument) as client_error:
        llm_passthrough_route(**call_for(INPUT_BODY, DEAD_BASE), client=object())
    with pytest.raises(UnsupportedCallArgument) as session_error:
        llm_passthrough_route(**call_for(INPUT_BODY, DEAD_BASE), shared_session=object())
    assert (client_error.value.argument, session_error.value.argument) == (
        "client",
        "shared_session",
    )
