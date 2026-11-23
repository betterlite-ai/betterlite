import asyncio
import importlib
import inspect
import re
import threading
from collections.abc import AsyncIterator, Callable, Generator, Iterator, Mapping
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from types import TracebackType
from typing import ClassVar, Final, Protocol, Self, TypeAlias, runtime_checkable

import httpx
import pytest
from pydantic import JsonValue, TypeAdapter
from typing_extensions import TypedDict, override

from betterlite._completion.arguments import SessionLike
from betterlite._completion.errors import UnsupportedCallArgument
from betterlite._native import NativeError
from betterlite.exceptions import APIError
from betterlite.passthrough.main import (
    AsyncStreamReply,
    StreamReply,
    allm_passthrough_route,
    llm_passthrough_route,
)
from betterlite.passthrough.timeout_utils import (
    resolve_llm_passthrough_timeout,
    resolve_pass_through_request_timeout,
)
from betterlite.passthrough.utils import BasePassthroughUtils, CommonUtils

HOST: Final = "127.0.0.1"
OK: Final = 200
UNAUTHORIZED: Final = 401
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
ERROR_TEXT: Final = '{"error": {"message": "Invalid API key", "type": "invalid_request_error"}}'
ERROR_BODY: Final[dict[str, JsonValue]] = {
    "error": {"message": "Invalid API key", "type": "invalid_request_error"},
}
FRAMES: Final = (b'data: {"n": 1}\n\n', b'data: {"n": 2}\n\n', b"data: [DONE]\n\n")
STREAM_BYTES: Final = b"".join(FRAMES)
JSON_TYPE: Final = "application/json"
EVENT_TYPE: Final = "text/event-stream"
HEADER_NAME: Final = "X-Upstream"
HEADER_VALUE: Final = "yes"
VOLATILE: Final[frozenset[str]] = frozenset(
    {"date", "server", "connection", "keep-alive", "transfer-encoding"}
)
DEAD_BASE: Final = f"http://{HOST}:9{API_PATH}"
POSITIONAL: Final = "POST"
EXPECTED_CALLS: Final = 2
TIMEOUT_SECONDS: Final = 12.5
ROUTER_SECONDS: Final = 40
STREAM_TIMEOUT: Final = 5
GENERIC_TIMEOUT: Final = 9
STRING_TIMEOUT: Final = "30"
ARN_MODEL: Final = "arn:aws:bedrock:us-east-1:123456789012:application-inference-profile/abc123"
ARN_ENDPOINT: Final = f"/model/{ARN_MODEL}/invoke"
ARN_ENCODED: Final = f"/model/{ARN_MODEL.replace('/', '%2F', 1)}/invoke"
REFERENCE_ERROR_TEXT: Final = "Invalid API key"
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

BODY_VIEW: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])
QUERY_VIEW: Final[TypeAdapter[dict[str, str | list[str]]]] = TypeAdapter(
    dict[str, str | list[str]]
)
HEADER_VIEW: Final[TypeAdapter[dict[str, str]]] = TypeAdapter(dict[str, str])


class RecordedCall(TypedDict):
    path: str
    body: bytes


# LiteLLM's own objects come back through this empty protocol, which every type satisfies
class CallResult(Protocol):
    ...


Answer: TypeAlias = tuple[int, dict[str, str], bytes]
CallArgument: TypeAlias = JsonValue | SessionLike
SignatureRow: TypeAlias = tuple[str, str, str]
RouteCall: TypeAlias = Callable[..., CallResult]


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

SIGNATURES: Final[list[tuple[str, RouteCall, RouteCall]]] = [
    ("llm_passthrough_route", llm_passthrough_route, REFERENCES.llm_passthrough_route),
    ("allm_passthrough_route", allm_passthrough_route, REFERENCES.allm_passthrough_route),
    (
        "get_merged_query_parameters",
        BasePassthroughUtils.get_merged_query_parameters,
        REFERENCE_UTILS.BasePassthroughUtils.get_merged_query_parameters,
    ),
    (
        "forward_headers_from_request",
        BasePassthroughUtils.forward_headers_from_request,
        REFERENCE_UTILS.BasePassthroughUtils.forward_headers_from_request,
    ),
    (
        "encode_bedrock_runtime_modelid_arn",
        CommonUtils.encode_bedrock_runtime_modelid_arn,
        REFERENCE_UTILS.CommonUtils.encode_bedrock_runtime_modelid_arn,
    ),
    (
        "resolve_pass_through_request_timeout",
        # ast-grep-ignore: no-long-names - LiteLLM's own name is this long
        resolve_pass_through_request_timeout,
        REFERENCE_TIMEOUTS.resolve_pass_through_request_timeout,
    ),
    (
        "resolve_llm_passthrough_timeout",
        # ast-grep-ignore: no-long-names - LiteLLM's own name is this long
        resolve_llm_passthrough_timeout,
        REFERENCE_TIMEOUTS.resolve_llm_passthrough_timeout,
    ),
]


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

    def record(self, path: str, body: bytes) -> None:
        self.calls.append({"path": path, "body": body})

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
        declared = self.headers.get("Content-Length")
        length = 0 if declared is None else int(declared)
        body = self.rfile.read(length)
        self.owner.record(self.path, body)
        if self.path.endswith(AUTH_ENDPOINT):
            self.reply(UNAUTHORIZED, ERROR_TEXT.encode(), JSON_TYPE)
            return
        if self.streams(body):
            self.stream()
            return
        self.reply(OK, ANSWER, JSON_TYPE)

    def streams(self, body: bytes) -> bool:
        if not body:
            return False
        parsed = BODY_VIEW.validate_json(body)
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


def default_text(value: CallResult) -> str:
    if value is inspect.Parameter.empty:
        return ""
    return repr(value)


def signature_of(function: RouteCall) -> list[SignatureRow]:
    parameters = inspect.signature(function).parameters.values()
    return [(item.name, item.kind.name, default_text(item.default)) for item in parameters]


def answer_parts(answer: StreamReply) -> Answer:
    match answer:
        case AnswerView():
            return answer.status_code, compared_headers(answer.headers), answer.content
        case object():
            raise AssertionError("the call answered with a stream")


def stream_parts(answer: StreamReply) -> Answer:
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


async def async_answer(answer: AsyncStreamReply) -> Answer:
    match answer:
        case AnswerView():
            return answer.status_code, compared_headers(answer.headers), answer.content
        case object():
            raise AssertionError("the async call answered with a stream")


async def async_stream(answer: AsyncStreamReply) -> Answer:
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


@pytest.mark.parametrize(("name", "ours", "reference"), SIGNATURES)
def test_signature_matches(name: str, ours: RouteCall, reference: RouteCall) -> None:
    rows = signature_of(reference)
    assert signature_of(ours) == rows, name


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
    match reference:
        case ErrorView():
            assert reference.status_code == UNAUTHORIZED
        case object():
            raise AssertionError("the reference error carries no status")


def test_error_provider_missing() -> None:
    with pytest.raises(Exception, match="Provider openai not found") as ours_error:
        llm_passthrough_route(
            method="POST",
            endpoint=CHAT_ENDPOINT,
            model=OPENAI_MODEL,
            custom_llm_provider=OPENAI_PROVIDER,
            api_base=DEAD_BASE,
            json=dict(INPUT_BODY),
        )
    with pytest.raises(Exception, match="Provider openai not found") as reference_error:
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
    assert str(ours) == "Provider openai not found"
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
