import asyncio
import importlib
import inspect
import json
import threading
from collections.abc import Awaitable
from contextlib import closing
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import ClassVar, Final, NamedTuple, Protocol, TypeAlias, TypeVar, runtime_checkable
from urllib.parse import parse_qsl, urlsplit

import pytest
from batch_server import parameter_rows
from pydantic import JsonValue, TypeAdapter
from typing_extensions import TypedDict

from betterlite._completion.errors import ERROR_PAYLOAD, UnsupportedCallArgument
from betterlite._endpoints.payload import CallValue
from betterlite._native import NativeVideoError
from betterlite.exceptions import AuthenticationError, NotFoundError

HOST: Final = "127.0.0.1"
OK: Final = 200
UNAUTHORIZED: Final = 401
NOT_FOUND: Final = 404
GOOD_KEY: Final = "sk-test"
BAD_KEY: Final = "bad-key"
PROVIDER: Final = "openai"
MODEL: Final = "sora-2"
REPLY_MODEL: Final = "sora-2-2025"
PROMPT: Final = "a kite over the bay"
REMIX_PROMPT: Final = "the same kite at sunset"
SECONDS: Final = "8"
SIZE: Final = "1280x720"
USER: Final = "user-1"
AFTER: Final = "cursor-test"
LIMIT: Final = 2
ORDER: Final = "asc"
VIDEO_ID: Final = "vid-test"
MISSING_ID: Final = "vid-missing"
FIRST_ID: Final = "vid-first"
LAST_ID: Final = "vid-last"
VARIANT: Final = "thumbnail"
VARIANT_FIELD: Final = "variant"
CREATED_AT: Final = 1710000000
COMPLETED_AT: Final = 1710000100
API_PREFIX: Final = "/v1"
VIDEOS_PATH: Final = f"{API_PREFIX}/videos"
STATUS_PATH: Final = f"{VIDEOS_PATH}/{VIDEO_ID}"
CONTENT_PATH: Final = f"{STATUS_PATH}/content"
REMIX_PATH: Final = f"{STATUS_PATH}/remix"
AUTH_TEXT: Final = "Incorrect API key provided: bad-key"
REFERENCE_FIELD: Final = "input_reference"
REFERENCE_FILENAME: Final = "input_reference.png"
PATH_NAME: Final = "reference.png"
STREAM_NAME: Final = "stream.png"
JSON_TYPE: Final = "application/json"
MEDIA_TYPE: Final = "video/mp4"
PNG_TYPE: Final = "image/png"
PNG_BYTES: Final = b"\x89PNG\r\n\x1a\n" + b"\x00\x00\x00\rIHDR" + b"\x00" * 8
MEDIA_BYTES: Final = b"\x00\x00\x00\x18ftypmp42" + b"\x00" * 24
CRLF: Final = b"\r\n"
HEADER_END: Final = b"\r\n\r\n"
GENERATION_FIELDS: Final[tuple[tuple[str, str], ...]] = (
    ("model", MODEL),
    ("prompt", PROMPT),
    ("seconds", SECONDS),
    ("size", SIZE),
    ("user", USER),
)
LIST_QUERY: Final[dict[str, str]] = {"after": AFTER, "limit": str(LIMIT), "order": ORDER}
AUTH_BODY: Final[dict[str, JsonValue]] = {
    "error": {"message": AUTH_TEXT, "type": "invalid_request_error"},
}
MISSING_BODY: Final[dict[str, JsonValue]] = {
    "error": {"message": "no such video", "type": "not_found_error", "code": "not_found"},
}
CREATE_RECORD: Final[dict[str, JsonValue]] = {
    "id": "vid-created",
    "object": "video",
    "status": "queued",
    "created_at": CREATED_AT,
    "seconds": SECONDS,
    "size": SIZE,
    "model": REPLY_MODEL,
    "progress": 0,
}
STATUS_RECORD: Final[dict[str, JsonValue]] = {
    "id": VIDEO_ID,
    "object": "video",
    "status": "completed",
    "created_at": CREATED_AT,
    "completed_at": COMPLETED_AT,
    "seconds": SECONDS,
    "size": SIZE,
    "model": REPLY_MODEL,
    "progress": 100,
}
REMIX_RECORD: Final[dict[str, JsonValue]] = {
    "id": "vid-remixed",
    "object": "video",
    "status": "queued",
    "remixed_from_video_id": VIDEO_ID,
    "created_at": CREATED_AT,
    "seconds": "4",
    "model": REPLY_MODEL,
}
LIST_PAGE: Final[dict[str, JsonValue]] = {
    "object": "list",
    "data": [
        {"id": FIRST_ID, "object": "video", "status": "completed", "model": MODEL},
        {"id": LAST_ID, "object": "video", "status": "completed", "model": REPLY_MODEL},
    ],
    "first_id": FIRST_ID,
    "last_id": LAST_ID,
    "has_more": False,
}
MOCK_ITEMS: Final[list[CallValue]] = [
    {"id": FIRST_ID, "object": "video", "status": "completed", "model": MODEL},
    {"id": LAST_ID, "object": "video", "status": "completed", "model": REPLY_MODEL},
]
JSON_RECORD: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])


class Part(TypedDict):
    filename: str | None
    content_type: str | None
    body: bytes


class RecordedCall(TypedDict):
    method: str
    path: str
    headers: dict[str, str]
    body: bytes


@runtime_checkable
class DumpedObject(Protocol):
    def model_dump_json(self) -> str: ...


VideoResult: TypeAlias = DumpedObject | bytes | dict[str, JsonValue] | list[DumpedObject]
Answer: TypeAlias = bytes | dict[str, JsonValue] | list[dict[str, JsonValue]]


class VideoCall(Protocol):
    def __call__(self, **kwargs: CallValue) -> VideoResult: ...


class AsyncVideoCall(Protocol):
    def __call__(self, **kwargs: CallValue) -> Awaitable[VideoResult]: ...


class VideoModule(Protocol):
    def __getattr__(self, name: str) -> VideoCall: ...


class AsyncVideoModule(Protocol):
    def __getattr__(self, name: str) -> AsyncVideoCall: ...


class ErrorClasses(Protocol):
    def __getattr__(self, name: str) -> type[BaseException]: ...


OURS: VideoModule = importlib.import_module("betterlite.videos._calls")
OURS_ASYNC: AsyncVideoModule = importlib.import_module("betterlite.videos._calls")
REFERENCE: VideoModule = importlib.import_module("litellm")
REFERENCE_ASYNC: AsyncVideoModule = importlib.import_module("litellm")
REFERENCE_ERRORS: ErrorClasses = importlib.import_module("litellm")

SYNC_CALLS: Final[dict[str, VideoCall]] = {
    "generation": OURS.video_generation,
    "content": OURS.video_content,
    "remix": OURS.video_remix,
    "status": OURS.video_status,
}
SYNC_REFERENCES: Final[dict[str, VideoCall]] = {
    "generation": REFERENCE.video_generation,
    "content": REFERENCE.video_content,
    "remix": REFERENCE.video_remix,
    "status": REFERENCE.video_status,
}
ASYNC_CALLS: Final[dict[str, AsyncVideoCall]] = {
    "generation": OURS_ASYNC.avideo_generation,
    "content": OURS_ASYNC.avideo_content,
    "remix": OURS_ASYNC.avideo_remix,
    "status": OURS_ASYNC.avideo_status,
}
ASYNC_REFERENCES: Final[dict[str, AsyncVideoCall]] = {
    "generation": REFERENCE_ASYNC.avideo_generation,
    "content": REFERENCE_ASYNC.avideo_content,
    "remix": REFERENCE_ASYNC.avideo_remix,
    "status": REFERENCE_ASYNC.avideo_status,
}


class Shape(NamedTuple):
    operation: str
    arguments: dict[str, CallValue]
    method: str
    path: str


SHAPES: Final[tuple[Shape, ...]] = (
    Shape(
        "generation",
        {"prompt": PROMPT, "model": MODEL, "seconds": SECONDS, "size": SIZE, "user": USER},
        "POST",
        VIDEOS_PATH,
    ),
    Shape("content", {"video_id": VIDEO_ID}, "GET", CONTENT_PATH),
    Shape("remix", {"video_id": VIDEO_ID, "prompt": REMIX_PROMPT}, "POST", REMIX_PATH),
    Shape("status", {"video_id": VIDEO_ID}, "GET", STATUS_PATH),
)


class Comparison(NamedTuple):
    ours: VideoResult
    reference: VideoResult
    ours_call: RecordedCall
    reference_call: RecordedCall


def field_value(header: str, key: str) -> str | None:
    for segment in header.split(";"):
        name, _, value = segment.strip().partition("=")
        if name == key:
            return value.strip().strip('"')
    return None


# the multipart body an upload reaches the server with, by part name
def sent_parts(call: RecordedCall) -> dict[str, Part]:
    lowered = {name.lower(): value for name, value in call["headers"].items()}
    kind = lowered.get("content-type")
    if kind is None or "multipart/form-data" not in kind:
        return {}
    marker = "boundary="
    start = kind.find(marker)
    if start < 0:
        return {}
    boundary = kind[start + len(marker):].split(";")[0].strip('"').encode()
    parts: dict[str, Part] = {}
    for chunk in call["body"].split(b"--" + boundary):
        section = chunk.removeprefix(CRLF)
        if not section.startswith(b"Content-Disposition:"):
            continue
        head, separator, content = section.partition(HEADER_END)
        if not separator:
            continue
        lines = head.decode().split(CRLF.decode())
        name = field_value(lines[0], "name")
        if name is None:
            continue
        content_type: str | None = None
        for line in lines[1:]:
            field, _, value = line.partition(":")
            if field.lower() == "content-type":
                content_type = value.strip()
        parts[name] = {
            "filename": field_value(lines[0], "filename"),
            "content_type": content_type,
            "body": content.removesuffix(CRLF),
        }
    return parts


def objects_dump(value: list[DumpedObject]) -> list[dict[str, JsonValue]]:
    return [
        JSON_RECORD.validate_json(item.model_dump_json())
        for item in value
    ]


def answer_dump(value: VideoResult) -> Answer:
    match value:
        case DumpedObject():
            return JSON_RECORD.validate_json(value.model_dump_json())
        case bytes() as content:
            return content
        case dict() as page:
            return page
        case list() as items:
            return objects_dump(items)


def rows_dump(value: VideoResult) -> list[dict[str, JsonValue]]:
    match value:
        case list() as items:
            return objects_dump(items)
        case DumpedObject() | bytes() | dict():
            raise AssertionError("the list call did not answer with objects")


def page_dump(value: VideoResult) -> dict[str, JsonValue]:
    match value:
        case dict() as page:
            return page
        case DumpedObject() | bytes() | list():
            raise AssertionError("the list call did not answer with a page")


def bytes_dump(value: VideoResult) -> bytes:
    match value:
        case bytes() as content:
            return content
        case DumpedObject() | dict() | list():
            raise AssertionError("the content call did not answer with bytes")


def base_path(call: RecordedCall) -> str:
    parts = urlsplit(call["path"])
    return parts.path


def query_of(call: RecordedCall) -> dict[str, str]:
    parts = urlsplit(call["path"])
    return dict(parse_qsl(parts.query))


def native_cause(error: BaseException) -> NativeVideoError:
    cause = error.__cause__
    match cause:
        case NativeVideoError():
            return cause
        case object():
            raise AssertionError("the exception does not carry the native error as its cause")


ResultT = TypeVar("ResultT")


async def awaited(awaitable: Awaitable[ResultT]) -> ResultT:
    match await awaitable:
        case result:
            return result


def endpoint_values(server: "VideoServer", **arguments: CallValue) -> dict[str, CallValue]:
    values: dict[str, CallValue] = {"custom_llm_provider": PROVIDER, "api_base": server.url}
    values["api_key"] = GOOD_KEY
    values.update(arguments)
    return values


def compared(
    server: "VideoServer",
    ours: VideoCall,
    reference: VideoCall,
    arguments: dict[str, CallValue],
) -> Comparison:
    values = endpoint_values(server, **arguments)
    reference_answer = reference(**values)
    reference_call = server.last()
    server.calls.clear()
    ours_answer = ours(**values)
    return Comparison(ours_answer, reference_answer, server.last(), reference_call)


async def answered_together(
    server: "VideoServer",
    ours: AsyncVideoCall,
    reference: AsyncVideoCall,
    arguments: dict[str, CallValue],
) -> Comparison:
    values = endpoint_values(server, **arguments)
    reference_answer = await reference(**values)
    ours_answer = await ours(**values)
    followed = server.calls[-2:]
    return Comparison(ours_answer, reference_answer, followed[1], followed[0])


def assert_same_call(
    reference_call: RecordedCall, ours_call: RecordedCall, method: str, path: str,
) -> None:
    assert ours_call["method"] == reference_call["method"] == method
    assert ours_call["path"] == reference_call["path"] == path


class VideoServer(ThreadingHTTPServer):
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
        return f"http://{HOST}:{self.server_port}{API_PREFIX}"

    def record(self, method: str, path: str, headers: dict[str, str], body: bytes) -> None:
        self.calls.append({"method": method, "path": path, "headers": headers, "body": body})

    def last(self) -> RecordedCall:
        return self.calls[-1]

    def close(self) -> None:
        self.shutdown()
        self.server_close()
        self.thread.join()


class _Handler(BaseHTTPRequestHandler):
    owner: ClassVar[VideoServer]

    def do_GET(self) -> None:
        self.handle_call()

    def do_POST(self) -> None:
        self.handle_call()

    def handle_call(self) -> None:
        declared = self.headers.get("Content-Length")
        body = b"" if declared is None else self.rfile.read(int(declared))
        self.owner.record(self.command, self.path, dict(self.headers.items()), body)
        if self.headers.get("Authorization") != f"Bearer {GOOD_KEY}":
            self.reply(UNAUTHORIZED, AUTH_BODY)
            return
        self.route(urlsplit(self.path).path)

    def route(self, target: str) -> None:
        command = self.command
        if command == "POST" and target == VIDEOS_PATH:
            self.reply(OK, CREATE_RECORD)
        elif command == "POST" and target == REMIX_PATH:
            self.reply(OK, REMIX_RECORD)
        elif command == "GET" and target == VIDEOS_PATH:
            self.reply(OK, LIST_PAGE)
        elif command == "GET" and target == STATUS_PATH:
            self.reply(OK, STATUS_RECORD)
        elif command == "GET" and target == CONTENT_PATH:
            self.send_media(MEDIA_BYTES)
        else:
            self.reply(NOT_FOUND, MISSING_BODY)

    def reply(self, status: int, payload: dict[str, JsonValue]) -> None:
        encoded = json.dumps(payload).encode()
        self.send_response(status)
        self.send_header("Content-Type", JSON_TYPE)
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)

    def send_media(self, content: bytes) -> None:
        self.send_response(OK)
        self.send_header("Content-Type", MEDIA_TYPE)
        self.send_header("Content-Length", str(len(content)))
        self.end_headers()
        self.wfile.write(content)


def test_signature_matches_litellm() -> None:
    assert parameter_rows(inspect.signature(OURS.video_generation)) == parameter_rows(
        inspect.signature(REFERENCE.video_generation))
    assert parameter_rows(inspect.signature(OURS.avideo_generation)) == parameter_rows(
        inspect.signature(REFERENCE_ASYNC.avideo_generation))
    assert parameter_rows(inspect.signature(OURS.video_content)) == parameter_rows(
        inspect.signature(REFERENCE.video_content))
    assert parameter_rows(inspect.signature(OURS.avideo_content)) == parameter_rows(
        inspect.signature(REFERENCE_ASYNC.avideo_content))
    assert parameter_rows(inspect.signature(OURS.video_remix)) == parameter_rows(
        inspect.signature(REFERENCE.video_remix))
    assert parameter_rows(inspect.signature(OURS.avideo_remix)) == parameter_rows(
        inspect.signature(REFERENCE_ASYNC.avideo_remix))
    assert parameter_rows(inspect.signature(OURS.video_list)) == parameter_rows(
        inspect.signature(REFERENCE.video_list))
    assert parameter_rows(inspect.signature(OURS.avideo_list)) == parameter_rows(
        inspect.signature(REFERENCE_ASYNC.avideo_list))
    assert parameter_rows(inspect.signature(OURS.video_status)) == parameter_rows(
        inspect.signature(REFERENCE.video_status))
    assert parameter_rows(inspect.signature(OURS.avideo_status)) == parameter_rows(
        inspect.signature(REFERENCE_ASYNC.avideo_status))
    assert not inspect.iscoroutinefunction(OURS.video_generation)
    assert not inspect.iscoroutinefunction(OURS.video_content)
    assert not inspect.iscoroutinefunction(OURS.video_remix)
    assert not inspect.iscoroutinefunction(OURS.video_list)
    assert not inspect.iscoroutinefunction(OURS.video_status)
    assert inspect.iscoroutinefunction(OURS_ASYNC.avideo_generation)
    assert inspect.iscoroutinefunction(OURS_ASYNC.avideo_content)
    assert inspect.iscoroutinefunction(OURS_ASYNC.avideo_remix)
    assert inspect.iscoroutinefunction(OURS_ASYNC.avideo_list)
    assert inspect.iscoroutinefunction(OURS_ASYNC.avideo_status)


@pytest.mark.parametrize("shape", SHAPES)
def test_calls_match_litellm(shape: Shape) -> None:
    with closing(VideoServer()) as server:
        swapped = compared(
            server, SYNC_CALLS[shape.operation], SYNC_REFERENCES[shape.operation],
            shape.arguments)
        asynchronous = asyncio.run(
            answered_together(
                server, ASYNC_CALLS[shape.operation], ASYNC_REFERENCES[shape.operation],
                shape.arguments))
    assert_same_call(swapped.reference_call, swapped.ours_call, shape.method, shape.path)
    assert_same_call(
        asynchronous.reference_call, asynchronous.ours_call, shape.method, shape.path)
    assert answer_dump(swapped.ours) == answer_dump(swapped.reference)
    assert answer_dump(asynchronous.ours) == answer_dump(asynchronous.reference)


def test_generation_form_calls() -> None:
    arguments: dict[str, CallValue] = {
        "prompt": PROMPT, "model": MODEL, "seconds": SECONDS, "size": SIZE, "user": USER,
    }
    with closing(VideoServer()) as server:
        swapped = compared(
            server, SYNC_CALLS["generation"], SYNC_REFERENCES["generation"], arguments)
        ours_parts = sent_parts(swapped.ours_call)
        reference_parts = sent_parts(swapped.reference_call)
    for name, expected in GENERATION_FIELDS:
        assert ours_parts[name]["body"] == reference_parts[name]["body"] == expected.encode()
    assert answer_dump(swapped.ours) == answer_dump(swapped.reference)


def test_generation_bytes_calls() -> None:
    arguments: dict[str, CallValue] = {
        "prompt": PROMPT, "model": MODEL, "input_reference": PNG_BYTES,
    }
    with closing(VideoServer()) as server:
        swapped = compared(
            server, SYNC_CALLS["generation"], SYNC_REFERENCES["generation"], arguments)
        ours_parts = sent_parts(swapped.ours_call)
        reference_parts = sent_parts(swapped.reference_call)
    expected: Part = {
        "filename": REFERENCE_FILENAME,
        "content_type": PNG_TYPE,
        "body": PNG_BYTES,
    }
    assert ours_parts[REFERENCE_FIELD] == expected
    assert reference_parts[REFERENCE_FIELD] == expected
    assert answer_dump(swapped.ours) == answer_dump(swapped.reference)


def test_generation_path_calls(tmp_path: Path) -> None:
    source = tmp_path / PATH_NAME
    source.write_bytes(PNG_BYTES)
    arguments: dict[str, CallValue] = {
        "prompt": PROMPT, "model": MODEL, "input_reference": source,
    }
    with closing(VideoServer()) as server:
        swapped = compared(
            server, SYNC_CALLS["generation"], SYNC_REFERENCES["generation"], arguments)
        ours_file = sent_parts(swapped.ours_call)[REFERENCE_FIELD]
        reference_file = sent_parts(swapped.reference_call)[REFERENCE_FIELD]
    assert ours_file["body"] == reference_file["body"] == PNG_BYTES
    assert ours_file["content_type"] == reference_file["content_type"] == PNG_TYPE
    assert ours_file["filename"] == PATH_NAME
    assert reference_file["filename"] == REFERENCE_FILENAME
    assert answer_dump(swapped.ours) == answer_dump(swapped.reference)


def test_generation_stream_calls(tmp_path: Path) -> None:
    source = tmp_path / STREAM_NAME
    source.write_bytes(PNG_BYTES)
    with closing(VideoServer()) as server, source.open("rb") as stream:
        values = endpoint_values(server, prompt=PROMPT, model=MODEL, input_reference=stream)
        reference_answer = SYNC_REFERENCES["generation"](**values)
        reference_call = server.last()
        server.calls.clear()
        stream.seek(0)
        start = stream.tell()
        ours_answer = SYNC_CALLS["generation"](**values)
        assert stream.tell() == start
        ours_call = server.last()
    ours_file = sent_parts(ours_call)[REFERENCE_FIELD]
    reference_file = sent_parts(reference_call)[REFERENCE_FIELD]
    assert ours_file["body"] == reference_file["body"] == PNG_BYTES
    assert ours_file["content_type"] == reference_file["content_type"] == PNG_TYPE
    assert ours_file["filename"] == STREAM_NAME
    assert reference_file["filename"] == str(source)
    assert answer_dump(ours_answer) == answer_dump(reference_answer)


def test_content_variant_calls() -> None:
    arguments: dict[str, CallValue] = {"video_id": VIDEO_ID, "variant": VARIANT}
    with closing(VideoServer()) as server:
        swapped = compared(server, SYNC_CALLS["content"], SYNC_REFERENCES["content"], arguments)
    assert base_path(swapped.ours_call) == base_path(swapped.reference_call)
    assert base_path(swapped.ours_call) == CONTENT_PATH
    assert query_of(swapped.ours_call) == query_of(swapped.reference_call)
    assert query_of(swapped.ours_call) == {VARIANT_FIELD: VARIANT}
    assert bytes_dump(swapped.ours) == MEDIA_BYTES


def test_list_query_calls() -> None:
    arguments: dict[str, CallValue] = {"after": AFTER, "limit": LIMIT, "order": ORDER}
    with closing(VideoServer()) as server:
        swapped = compared(server, OURS.video_list, REFERENCE.video_list, arguments)
        asynchronous = asyncio.run(
            answered_together(
                server, OURS_ASYNC.avideo_list, REFERENCE_ASYNC.avideo_list,
                arguments))
    assert base_path(swapped.ours_call) == base_path(swapped.reference_call)
    assert base_path(swapped.ours_call) == VIDEOS_PATH
    assert query_of(swapped.ours_call) == query_of(swapped.reference_call)
    assert query_of(swapped.ours_call) == LIST_QUERY
    assert page_dump(swapped.ours) == page_dump(swapped.reference)
    assert page_dump(asynchronous.ours) == page_dump(swapped.reference)


def test_list_mock_calls() -> None:
    values: dict[str, CallValue] = {"mock_response": MOCK_ITEMS, "custom_llm_provider": PROVIDER}
    reference_rows = rows_dump(REFERENCE.video_list(**values))
    ours_rows = rows_dump(OURS.video_list(**values))
    asynchronous = rows_dump(asyncio.run(awaited(OURS_ASYNC.avideo_list(**values))))
    assert ours_rows == reference_rows
    assert asynchronous == reference_rows
    assert [row["id"] for row in ours_rows] == [FIRST_ID, LAST_ID]


def test_remix_body_calls() -> None:
    arguments: dict[str, CallValue] = {"video_id": VIDEO_ID, "prompt": REMIX_PROMPT}
    with closing(VideoServer()) as server:
        swapped = compared(server, SYNC_CALLS["remix"], SYNC_REFERENCES["remix"], arguments)
        ours_body = JSON_RECORD.validate_json(swapped.ours_call["body"])
        reference_body = JSON_RECORD.validate_json(swapped.reference_call["body"])
    assert ours_body == reference_body == {"prompt": REMIX_PROMPT}


def test_error_matches_litellm() -> None:
    reference_class = REFERENCE_ERRORS.AuthenticationError
    values: dict[str, CallValue] = {
        "video_id": VIDEO_ID, "prompt": REMIX_PROMPT, "api_key": BAD_KEY,
        "custom_llm_provider": PROVIDER,
    }
    with closing(VideoServer()) as server:
        values["api_base"] = server.url
        with pytest.raises(reference_class):
            REFERENCE.video_remix(**values)
        with pytest.raises(AuthenticationError) as made:
            SYNC_CALLS["remix"](**values)
        with pytest.raises(AuthenticationError) as asynchronous:
            asyncio.run(awaited(ASYNC_CALLS["remix"](**values)))
    assert made.value.status_code == UNAUTHORIZED
    assert asynchronous.value.status_code == UNAUTHORIZED
    payload = ERROR_PAYLOAD.validate_json(str(native_cause(made.value)))
    assert payload["type"] == reference_class.__name__
    assert payload["status_code"] == UNAUTHORIZED


@pytest.mark.parametrize("operation", ["status", "content"])
def test_read_error_calls(operation: str) -> None:
    with closing(VideoServer()) as server, pytest.raises(AuthenticationError) as made:
        SYNC_CALLS[operation](
            video_id=VIDEO_ID, api_key=BAD_KEY, api_base=server.url,
            custom_llm_provider=PROVIDER)
    assert made.value.status_code == UNAUTHORIZED


def test_content_missing_calls() -> None:
    with closing(VideoServer()) as server:
        reference_answer = REFERENCE.video_content(
            video_id=MISSING_ID, api_key=GOOD_KEY, api_base=server.url,
            custom_llm_provider=PROVIDER)
        with pytest.raises(NotFoundError) as made:
            OURS.video_content(
                video_id=MISSING_ID, api_key=GOOD_KEY, api_base=server.url,
                custom_llm_provider=PROVIDER)
    assert made.value.status_code == NOT_FOUND
    assert bytes_dump(reference_answer) == json.dumps(MISSING_BODY).encode()


def test_unsupported_objects() -> None:
    for call in SYNC_CALLS.values():
        with pytest.raises(UnsupportedCallArgument, match="client"):
            call(prompt=PROMPT, video_id=VIDEO_ID, client="client")
    with pytest.raises(UnsupportedCallArgument, match="client"):
        OURS.video_list(client="client")
    for async_call in ASYNC_CALLS.values():
        with pytest.raises(UnsupportedCallArgument, match="client"):
            asyncio.run(awaited(async_call(prompt=PROMPT, video_id=VIDEO_ID, client="client")))
    with pytest.raises(UnsupportedCallArgument, match="client"):
        asyncio.run(awaited(OURS_ASYNC.avideo_list(client="client")))
    with pytest.raises(UnsupportedCallArgument, match="shared_session"):
        OURS.video_generation(prompt=PROMPT, shared_session="session")
    with pytest.raises(UnsupportedCallArgument, match="shared_session"):
        asyncio.run(awaited(OURS_ASYNC.avideo_generation(
            prompt=PROMPT, shared_session="session")))
