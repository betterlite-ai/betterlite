import asyncio
import importlib
import subprocess
import sys
import threading
from collections.abc import AsyncIterator, Generator, Iterator, Mapping
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from types import TracebackType
from typing import ClassVar, Final, Protocol, Self, TypeAlias, runtime_checkable

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
    AsyncStreamReply,
    PassthroughResponse,
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
        [
            parameter.name,
            parameter.kind.name,
            "" if parameter.default is inspect.Parameter.empty else repr(parameter.default),
        ]
        for parameter in inspect.signature(function).parameters.values()
    ]


ours = importlib.import_module("betterlite.passthrough.main")
theirs = importlib.import_module("litellm.passthrough.main")
ours_utils = importlib.import_module("betterlite.passthrough.utils")
theirs_utils = importlib.import_module("litellm.passthrough.utils")
ours_timeouts = importlib.import_module("betterlite.passthrough.timeout_utils")
theirs_timeouts = importlib.import_module("litellm.passthrough.timeout_utils")
pairs = {
    "llm_passthrough_route": (ours.llm_passthrough_route, theirs.llm_passthrough_route),
    "allm_passthrough_route": (ours.allm_passthrough_route, theirs.allm_passthrough_route),
    "get_merged_query_parameters": (
        ours_utils.BasePassthroughUtils.get_merged_query_parameters,
        theirs_utils.BasePassthroughUtils.get_merged_query_parameters,
    ),
    "forward_headers_from_request": (
        ours_utils.BasePassthroughUtils.forward_headers_from_request,
        theirs_utils.BasePassthroughUtils.forward_headers_from_request,
    ),
    "encode_bedrock_runtime_modelid_arn": (
        ours_utils.CommonUtils.encode_bedrock_runtime_modelid_arn,
        theirs_utils.CommonUtils.encode_bedrock_runtime_modelid_arn,
    ),
    "resolve_pass_through_request_timeout": (
        ours_timeouts.resolve_pass_through_request_timeout,
        theirs_timeouts.resolve_pass_through_request_timeout,
    ),
    "resolve_llm_passthrough_timeout": (
        ours_timeouts.resolve_llm_passthrough_timeout,
        theirs_timeouts.resolve_llm_passthrough_timeout,
    ),
}
print(
    json.dumps(
        {
            "ours": {name: rows(ours_rows) for name, (ours_rows, _) in pairs.items()},
            "reference": {name: rows(theirs_rows) for name, (_, theirs_rows) in pairs.items()},
        }
    )
)
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
    api_base: str | None
    content: Content | None
    data: dict[str, CallValue] | None
    files: Files | None
    params: Params | None
    cookies: Cookies | None
    request_query_params: dict[str, JsonValue] | None
    request_headers: dict[str, str] | None
    forward_headers: bool


class SignatureView(TypedDict):
    ours: dict[str, list[tuple[str, str, str]]]
    reference: dict[str, list[tuple[str, str, str]]]


SIGNATURE_VIEW: Final[TypeAdapter[SignatureView]] = TypeAdapter(SignatureView)


# LiteLLM's own objects come back through this empty protocol, which every type satisfies
class CallResult(Protocol):
    ...


Answer: TypeAlias = tuple[int, dict[str, str], bytes]
CallArgument: TypeAlias = JsonValue | SessionLike


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
        if self.headers.get("Transfer-Encoding", "").lower() == "chunked":
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


def compared_headers(headers: Mapping[str, str]) -> dict[str, str]:
    kept: dict[str, str] = {}
    for name, value in headers.items():
        lowered = name.lower()
        if lowered in VOLATILE:
            continue
        kept[lowered] = value
    return kept


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


async def async_stream(answer: AsyncStreamReply | AsyncStreamView) -> Answer:
    match answer:
        case AsyncStreamView():
            chunks = [chunk async for chunk in answer]
            return answer.status_code, compared_headers(answer.headers), b"".join(chunks)
        case object():
            raise AssertionError("the async call answered with a full response")


async def reference_async_stream(answer: CallResult) -> Answer:
    match answer:
        case AsyncStreamView():
            chunks = [chunk async for chunk in answer]
            return answer.status_code, compared_headers(answer.headers), b"".join(chunks)
        case object():
            raise AssertionError("the reference call answered with no stream")


async def ours_async(base: str) -> tuple[Answer, Answer]:
    full = await allm_passthrough_route(
        method="POST",
        endpoint=CHAT_ENDPOINT,
        model=MODEL,
        custom_llm_provider=PROVIDER,
        api_base=base,
        json=dict(INPUT_BODY),
    )
    streamed = await allm_passthrough_route(
        method="POST",
        endpoint=CHAT_ENDPOINT,
        model=MODEL,
        custom_llm_provider=PROVIDER,
        api_base=base,
        json=dict(STREAM_BODY),
    )
    return await async_answer(full), await async_stream(streamed)


async def reference_async(base: str) -> tuple[Answer, Answer]:
    full = await awaited(
        REFERENCES.allm_passthrough_route(
            method="POST",
            endpoint=CHAT_ENDPOINT,
            model=MODEL,
            custom_llm_provider=PROVIDER,
            api_base=base,
            json=dict(INPUT_BODY),
        )
    )
    streamed = await awaited(
        REFERENCES.allm_passthrough_route(
            method="POST",
            endpoint=CHAT_ENDPOINT,
            model=MODEL,
            custom_llm_provider=PROVIDER,
            api_base=base,
            json=dict(STREAM_BODY),
        )
    )
    return reference_parts(full), await reference_async_stream(streamed)


async def reference_positional() -> None:
    with pytest.raises(TypeError):
        await awaited(REFERENCES.allm_passthrough_route(POSITIONAL))


async def truncated_async(base: str) -> None:
    response = await allm_passthrough_route(
        method="POST",
        endpoint=TRUNCATED_ENDPOINT,
        model=MODEL,
        custom_llm_provider=PROVIDER,
        api_base=base,
        json=dict(STREAM_BODY),
    )
    match response:
        case AsyncStreamView() as stream:
            iterator = aiter(stream)
            handed_out: bytes | None = None
            failure: APIError | None = None
            try:
                async for chunk in iterator:
                    if handed_out is None:
                        handed_out = chunk
            except APIError as caught:
                failure = caught
            assert failure is not None
            assert failure.status_code == SERVER_ERROR
            assert handed_out is not None
            assert TRUNCATED_FRAME.startswith(handed_out)
            with pytest.raises(StopAsyncIteration):
                await anext(iterator)
        case object():
            raise AssertionError("the truncated call answered with no stream")


def passthrough_response(answer: StreamReply) -> PassthroughResponse:
    match answer:
        case PassthroughResponse() as response:
            return response
        case object():
            raise AssertionError("the call answered with a stream")


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
    content_type = record["headers"].get("content-type", "")
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
    server: PassthroughServer, shape: RouteShape
) -> tuple[RecordedCall, RecordedCall]:
    api_base = shape.get("api_base")
    base = server.base if api_base is None else api_base
    ours = llm_passthrough_route(
        method="POST",
        endpoint=CHAT_ENDPOINT,
        model=MODEL,
        custom_llm_provider=PROVIDER,
        api_base=base,
        request_query_params=shape.get("request_query_params"),
        request_headers=shape.get("request_headers"),
        content=shape.get("content"),
        data=shape.get("data"),
        files=shape.get("files"),
        params=shape.get("params"),
        cookies=shape.get("cookies"),
        forward_headers=shape.get("forward_headers", False),
    )
    reference = REFERENCES.llm_passthrough_route(
        method="POST",
        endpoint=CHAT_ENDPOINT,
        model=MODEL,
        custom_llm_provider=PROVIDER,
        api_base=base,
        request_query_params=shape.get("request_query_params"),
        request_headers=shape.get("request_headers"),
        content=shape.get("content"),
        data=shape.get("data"),
        files=shape.get("files"),
        params=shape.get("params"),
        cookies=shape.get("cookies"),
        forward_headers=shape.get("forward_headers", False),
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


def test_calls_match() -> None:
    with PassthroughServer() as server:
        ours = llm_passthrough_route(
            method="POST",
            endpoint=CHAT_ENDPOINT,
            model=MODEL,
            custom_llm_provider=PROVIDER,
            api_base=server.base,
            json=dict(INPUT_BODY),
        )
        reference = REFERENCES.llm_passthrough_route(
            method="POST",
            endpoint=CHAT_ENDPOINT,
            model=MODEL,
            custom_llm_provider=PROVIDER,
            api_base=server.base,
            json=dict(INPUT_BODY),
        )
        records = list(server.calls)
    assert answer_parts(ours) == reference_parts(reference)
    assert answer_parts(ours) == (OK, expected_headers(JSON_TYPE, len(ANSWER)), ANSWER)
    assert len(records) == EXPECTED_CALLS
    assert [record["path"] for record in records] == [CHAT_PATH, CHAT_PATH]
    assert [recorded_body(record) for record in records] == [INPUT_BODY, INPUT_BODY]
    response = passthrough_response(ours)
    assert response.json() == ANSWER_BODY
    assert response.text == ANSWER.decode()


def test_response_raise_for_status() -> None:
    with pytest.raises(APIError) as failure:
        PassthroughResponse(UNAUTHORIZED, {}, ERROR_TEXT.encode()).raise_for_status()
    assert failure.value.status_code == UNAUTHORIZED
    assert failure.value.message == ERROR_TEXT
    assert PassthroughResponse(OK, {}, ANSWER).raise_for_status() is None


def test_calls_stream_match() -> None:
    with PassthroughServer() as server:
        ours = llm_passthrough_route(
            method="POST",
            endpoint=CHAT_ENDPOINT,
            model=MODEL,
            custom_llm_provider=PROVIDER,
            api_base=server.base,
            json=dict(STREAM_BODY),
        )
        reference = REFERENCES.llm_passthrough_route(
            method="POST",
            endpoint=CHAT_ENDPOINT,
            model=MODEL,
            custom_llm_provider=PROVIDER,
            api_base=server.base,
            json=dict(STREAM_BODY),
        )
        ours_answer = stream_parts(ours)
        reference_answer = reference_stream_parts(reference)
    assert ours_answer == reference_answer
    assert ours_answer == (OK, expected_headers(EVENT_TYPE), STREAM_BYTES)


def test_calls_async_match() -> None:
    with PassthroughServer() as server:
        ours = asyncio.run(ours_async(server.base))
        reference = asyncio.run(reference_async(server.base))
    assert ours == reference
    assert ours == (
        (OK, expected_headers(JSON_TYPE, len(ANSWER)), ANSWER),
        (OK, expected_headers(EVENT_TYPE), STREAM_BYTES),
    )


def test_calls_content_match() -> None:
    with PassthroughServer() as server:
        text = round_trip(server, RouteShape(content="hi"))
        raw = round_trip(server, RouteShape(content=b"raw"))
        chunks = round_trip(server, RouteShape(content=[b"a", b"b"]))
    assert [call[0]["body"] for call in (text, raw, chunks)] == [b"hi", b"raw", b"ab"]
    for ours, reference in (text, raw, chunks):
        assert ours["headers"].get("content-type") == OCTET_TYPE
        assert "content-type" not in reference["headers"]


def test_calls_form_match() -> None:
    with PassthroughServer() as server:
        ours, reference = round_trip(
            server, RouteShape(data={"field": "value", "repeat": ["1", "2"]})
        )
    assert ours["body"] == reference["body"] == b"field=value&repeat=1&repeat=2"
    assert ours["headers"].get("content-type") == FORM_TYPE
    assert reference["headers"].get("content-type") == FORM_TYPE


def test_calls_params_match() -> None:
    with PassthroughServer() as server:
        mapping = round_trip(server, RouteShape(params={"q": "1", "tag": ["a", "b"]}))
        text = round_trip(server, RouteShape(params="a=1&b=2"))
        sequence = round_trip(server, RouteShape(params=[("k", "v"), ("k2", "v2")]))
    assert mapping[0]["path"] == mapping[1]["path"] == f"{CHAT_PATH}?q=1&tag=a&tag=b"
    assert text[0]["path"] == text[1]["path"] == f"{CHAT_PATH}?a=1&b=2"
    assert sequence[0]["path"] == sequence[1]["path"] == f"{CHAT_PATH}?k=v&k2=v2"


def test_calls_query_params_match() -> None:
    with PassthroughServer() as server:
        plain = round_trip(server, RouteShape(request_query_params={"req": "1"}))
        asked = round_trip(
            server,
            RouteShape(
                api_base=f"{server.base}?from_base=1",
                request_query_params={"req": "1"},
            ),
        )
    assert plain[0]["path"] == plain[1]["path"] == f"{CHAT_PATH}?req=1"
    # format_url replaces the base query, so the endpoint text never leaves the path
    assert asked[0]["path"] == asked[1]["path"] == f"{API_PATH}?req=1"


def test_calls_cookies_match() -> None:
    with PassthroughServer() as server:
        ours, reference = round_trip(server, RouteShape(cookies={"session": "abc"}))
    assert ours["headers"].get("cookie") == reference["headers"].get("cookie")
    assert ours["headers"].get("cookie") == "session=abc"


def test_calls_files_match() -> None:
    with PassthroughServer() as server:
        mapping = round_trip(
            server, RouteShape(files={"image": ("cat.png", b"png-bytes", "image/png")})
        )
        repeated = round_trip(
            server,
            RouteShape(
                files=[
                    ("image", ("one.png", b"one", "image/png")),
                    ("image", ("two.png", b"two", "image/png")),
                ]
            ),
        )
    assert mapping[0]["headers"].get("content-type", "").startswith(MULTIPART_TYPE)
    assert mapping[1]["headers"].get("content-type", "").startswith(MULTIPART_TYPE)
    assert multipart_parts(mapping[0]) == multipart_parts(mapping[1])
    assert multipart_parts(repeated[0]) == multipart_parts(repeated[1])
    assert [part["headers"]["content-disposition"] for part in multipart_parts(mapping[0])] == [
        'form-data; name="image"; filename="cat.png"'
    ]
    assert [part["body"] for part in multipart_parts(repeated[0])] == [b"one", b"two"]


def test_calls_request_headers_match() -> None:
    request_headers = {"x-pass-anthropic-beta": "beta-1", "x-request": "1"}
    with PassthroughServer() as server:
        quiet = round_trip(server, RouteShape(request_headers=dict(request_headers)))
        asked = round_trip(
            server,
            RouteShape(request_headers=dict(request_headers), forward_headers=True),
        )
    for ours in (quiet[0], asked[0]):
        assert ours["headers"].get("anthropic-beta") == "beta-1"
        assert "x-request" not in ours["headers"]
    assert quiet[0]["headers"].get("anthropic-beta") == quiet[1]["headers"].get("anthropic-beta")


def test_calls_helpers_match() -> None:
    url = httpx.URL(f"http://{HOST}:9{API_PATH}?from_url=url&shared=url&blank=")
    ours_query = BasePassthroughUtils.get_merged_query_parameters(url, MERGE_QUERY, MERGE_DEFAULTS)
    reference_query = QUERY_VIEW.validate_python(
        REFERENCE_UTILS.BasePassthroughUtils.get_merged_query_parameters(
            existing_url=url,
            request_query_params=MERGE_QUERY,
            default_query_params=MERGE_DEFAULTS,
        )
    )
    assert ours_query == reference_query
    ours_headers = BasePassthroughUtils.forward_headers_from_request(
        dict(FORWARD_REQUEST), dict(CUSTOM_HEADERS), forward_headers=True
    )
    reference_headers = HEADER_VIEW.validate_python(
        REFERENCE_UTILS.BasePassthroughUtils.forward_headers_from_request(
            request_headers=dict(FORWARD_REQUEST),
            headers=dict(CUSTOM_HEADERS),
            forward_headers=True,
        )
    )
    assert ours_headers == reference_headers
    ours_pass = BasePassthroughUtils.forward_headers_from_request(
        dict(PASS_REQUEST), dict(NO_HEADERS), forward_headers=False
    )
    reference_pass = HEADER_VIEW.validate_python(
        REFERENCE_UTILS.BasePassthroughUtils.forward_headers_from_request(
            request_headers=dict(PASS_REQUEST),
            headers=dict(NO_HEADERS),
            forward_headers=False,
        )
    )
    assert ours_pass == reference_pass
    ours_arn = CommonUtils.encode_bedrock_runtime_modelid_arn(ARN_ENDPOINT)
    reference_arn = str_of(
        REFERENCE_UTILS.CommonUtils.encode_bedrock_runtime_modelid_arn(endpoint=ARN_ENDPOINT)
    )
    assert ours_arn == reference_arn == ARN_ENCODED
    assert resolve_pass_through_request_timeout() == float_of(
        REFERENCE_TIMEOUTS.resolve_pass_through_request_timeout()
    )
    assert resolve_pass_through_request_timeout(TIMEOUT_SECONDS) == float_of(
        REFERENCE_TIMEOUTS.resolve_pass_through_request_timeout(
            endpoint_timeout=TIMEOUT_SECONDS
        )
    )
    assert resolve_llm_passthrough_timeout() == float_of(
        REFERENCE_TIMEOUTS.resolve_llm_passthrough_timeout()
    )
    assert resolve_llm_passthrough_timeout({"timeout": TIMEOUT_SECONDS}) == float_of(
        REFERENCE_TIMEOUTS.resolve_llm_passthrough_timeout(
            kwargs={"timeout": TIMEOUT_SECONDS}
        )
    )
    assert resolve_llm_passthrough_timeout(
        {"stream": True, "stream_timeout": STREAM_TIMEOUT, "timeout": GENERIC_TIMEOUT}
    ) == float_of(
        REFERENCE_TIMEOUTS.resolve_llm_passthrough_timeout(
            kwargs={
                "stream": True,
                "stream_timeout": STREAM_TIMEOUT,
                "timeout": GENERIC_TIMEOUT,
            }
        )
    )
    assert resolve_llm_passthrough_timeout(router_timeout=ROUTER_SECONDS) == float_of(
        REFERENCE_TIMEOUTS.resolve_llm_passthrough_timeout(router_timeout=ROUTER_SECONDS)
    )
    assert resolve_llm_passthrough_timeout({"timeout": STRING_TIMEOUT}) == float_of(
        REFERENCE_TIMEOUTS.resolve_llm_passthrough_timeout(
            kwargs={"timeout": STRING_TIMEOUT}
        )
    )
    assert resolve_llm_passthrough_timeout(
        litellm_params={"timeout": TIMEOUT_SECONDS}
    ) == float_of(
        REFERENCE_TIMEOUTS.resolve_llm_passthrough_timeout(
            litellm_params={"timeout": TIMEOUT_SECONDS}
        )
    )
    assert resolve_llm_passthrough_timeout(
        litellm_params={"request_timeout": STRING_TIMEOUT}
    ) == float_of(
        REFERENCE_TIMEOUTS.resolve_llm_passthrough_timeout(
            litellm_params={"request_timeout": STRING_TIMEOUT}
        )
    )
    assert resolve_llm_passthrough_timeout(
        kwargs={"stream": True}, litellm_params={"stream_timeout": STREAM_TIMEOUT}
    ) == float_of(
        REFERENCE_TIMEOUTS.resolve_llm_passthrough_timeout(
            kwargs={"stream": True}, litellm_params={"stream_timeout": STREAM_TIMEOUT}
        )
    )
    assert resolve_llm_passthrough_timeout(
        kwargs={"stream": True, "timeout": GENERIC_TIMEOUT},
        router_stream_timeout=ROUTER_SECONDS,
    ) == float_of(
        REFERENCE_TIMEOUTS.resolve_llm_passthrough_timeout(
            kwargs={"stream": True, "timeout": GENERIC_TIMEOUT},
            router_stream_timeout=ROUTER_SECONDS,
        )
    )
    assert resolve_llm_passthrough_timeout({"request_timeout": TIMEOUT_SECONDS}) == float_of(
        REFERENCE_TIMEOUTS.resolve_llm_passthrough_timeout(
            kwargs={"request_timeout": TIMEOUT_SECONDS}
        )
    )
    assert resolve_llm_passthrough_timeout(
        {"timeout": GENERIC_TIMEOUT}, litellm_params={"timeout": "later"}
    ) == float_of(
        REFERENCE_TIMEOUTS.resolve_llm_passthrough_timeout(
            kwargs={"timeout": GENERIC_TIMEOUT}, litellm_params={"timeout": "later"}
        )
    )
    with pytest.raises(ValueError, match="soon"):
        resolve_llm_passthrough_timeout({"timeout": "soon"})
    with pytest.raises(ValueError, match="soon"):
        REFERENCE_TIMEOUTS.resolve_llm_passthrough_timeout(kwargs={"timeout": "soon"})
    relative = BasePassthroughUtils.get_merged_query_parameters(
        httpx.URL("/v1/chat?from_url=url"), MERGE_QUERY, MERGE_DEFAULTS
    )
    reference_relative = QUERY_VIEW.validate_python(
        REFERENCE_UTILS.BasePassthroughUtils.get_merged_query_parameters(
            existing_url=httpx.URL("/v1/chat?from_url=url"),
            request_query_params=MERGE_QUERY,
            default_query_params=MERGE_DEFAULTS,
        )
    )
    assert relative == reference_relative
    with pytest.raises(ValueError, match="not a URL"):
        BasePassthroughUtils.get_merged_query_parameters(
            "http://", MERGE_QUERY, MERGE_DEFAULTS
        )


def test_error_upstream() -> None:
    with PassthroughServer() as server:
        with pytest.raises(APIError) as ours_error:
            llm_passthrough_route(
                method="POST",
                endpoint=AUTH_ENDPOINT,
                model=MODEL,
                custom_llm_provider=PROVIDER,
                api_base=server.base,
                json=dict(INPUT_BODY),
            )
        with pytest.raises(Exception, match=REFERENCE_ERROR_TEXT) as reference_error:
            REFERENCES.llm_passthrough_route(
                method="POST",
                endpoint=AUTH_ENDPOINT,
                model=MODEL,
                custom_llm_provider=PROVIDER,
                api_base=server.base,
                json=dict(INPUT_BODY),
            )
    ours = ours_error.value
    assert ours.status_code == UNAUTHORIZED
    assert ours.message == f"litellm.APIError: {ERROR_TEXT}"
    assert ours.body == ERROR_BODY
    match ours.__cause__:
        case NativeError():
            pass
        case object():
            raise AssertionError("the raised error carries no native cause")
    reference = reference_error.value
    assert ERROR_TEXT in str(reference)
    assert status_of(reference) == UNAUTHORIZED


def test_error_stream_truncated() -> None:
    with PassthroughServer() as server:
        response = llm_passthrough_route(
            method="POST",
            endpoint=TRUNCATED_ENDPOINT,
            model=MODEL,
            custom_llm_provider=PROVIDER,
            api_base=server.base,
            json=dict(STREAM_BODY),
        )
        match response:
            case StreamView() as stream:
                handed_out: bytes | None = None
                failure: APIError | None = None
                try:
                    for chunk in stream:
                        if handed_out is None:
                            handed_out = chunk
                except APIError as caught:
                    failure = caught
                assert failure is not None
                assert failure.status_code == SERVER_ERROR
                assert handed_out is not None
                assert TRUNCATED_FRAME.startswith(handed_out)
                with pytest.raises(StopIteration):
                    next(iter(stream))
            case object():
                raise AssertionError("the truncated call answered with no stream")


def test_error_stream_truncated_async() -> None:
    with PassthroughServer() as server:
        asyncio.run(truncated_async(server.base))


def test_error_provider_missing() -> None:
    with pytest.raises(Exception, match=PROVIDER_ERROR_TEXT) as ours_error:
        llm_passthrough_route(
            method="POST",
            endpoint=CHAT_ENDPOINT,
            model=OPENAI_MODEL,
            custom_llm_provider=OPENAI_PROVIDER,
            api_base=DEAD_BASE,
            json=dict(INPUT_BODY),
        )
    with pytest.raises(Exception, match=PROVIDER_ERROR_TEXT) as reference_error:
        REFERENCES.llm_passthrough_route(
            method="POST",
            endpoint=CHAT_ENDPOINT,
            model=OPENAI_MODEL,
            custom_llm_provider=OPENAI_PROVIDER,
            api_base=DEAD_BASE,
            json=dict(INPUT_BODY),
        )
    ours = ours_error.value
    assert type(ours).__name__ == "Exception"
    assert str(ours) == PROVIDER_ERROR_TEXT
    assert type(reference_error.value).__name__ == "Exception"
    match ours.__cause__:
        case NativeError():
            pass
        case object():
            raise AssertionError("the raised error carries no native cause")


def test_unsupported_argument() -> None:
    with pytest.raises(UnsupportedCallArgument) as client_error:
        llm_passthrough_route(
            method="POST",
            endpoint=CHAT_ENDPOINT,
            model=MODEL,
            custom_llm_provider=PROVIDER,
            api_base=DEAD_BASE,
            client=object(),
        )
    assert client_error.value.argument == "client"
    with pytest.raises(UnsupportedCallArgument) as session_error:
        llm_passthrough_route(
            method="POST",
            endpoint=CHAT_ENDPOINT,
            model=MODEL,
            custom_llm_provider=PROVIDER,
            api_base=DEAD_BASE,
            shared_session=object(),
        )
    assert session_error.value.argument == "shared_session"
